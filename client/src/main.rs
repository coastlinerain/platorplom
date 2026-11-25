mod hud;

use game::config::TICK_HZ;
use game::maps::Map;
use macroquad::prelude::*;
use metroidvania_rust::network::logging::log_packet;
use metroidvania_rust::network::packet_io::PacketIO;
use metroidvania_rust::network::world::send_join_req;
use metroidvania_rust::world::game::World;
use std::net::SocketAddr;
use std::time::Duration;

/// RealPacketIO wraps the laminar socket for production use.
struct RealPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<protocol::GamePacket>(p.payload()) {
                    log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
        log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            protocol::GamePacket::JoinRequest | protocol::GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

/// HeadlessPacketIO for running without a window (CI/testing).
struct HeadlessPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for HeadlessPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<protocol::GamePacket>(p.payload()) {
                    log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
        log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            protocol::GamePacket::JoinRequest | protocol::GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

async fn run_headless(duration_secs: u64, server_addr: SocketAddr) {
    let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
    let sender = socket.get_packet_sender();
    let receiver = socket.get_event_receiver();
    std::thread::spawn(move || socket.start_polling());

    let io = HeadlessPacketIO {
        sender,
        receiver,
        server_addr,
    };
    let mut boxed: Box<dyn PacketIO> = Box::new(io);
    send_join_req(&mut *boxed);

    let world_data = Map::new();
    let mut game_world = World::new(world_data, boxed);

    let tick_dt = 1.0 / TICK_HZ as f32;
    let max_ticks = if duration_secs > 0 {
        (duration_secs as f32 * TICK_HZ as f32) as u64
    } else {
        u64::MAX
    };

    println!(
        "Headless client running for {} seconds ({} ticks)",
        duration_secs, max_ticks
    );

    for tick in 0..max_ticks {
        game_world.update(tick_dt);

        // Periodic status log
        if tick % (TICK_HZ as u64 * 5) == 0 && tick > 0 {
            println!("Headless tick: {}", tick);
        }

        std::thread::sleep(Duration::from_millis(1000 / TICK_HZ as u64));
    }

    println!(
        "Headless client finished after {} ticks",
        max_ticks.min(max_ticks)
    );
}

#[macroquad::main("Rustvania")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");

    // Parse --duration=N (seconds)
    let duration_secs = args
        .iter()
        .find(|a| a.starts_with("--duration="))
        .and_then(|a| a.split('=').nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0); // 0 = run until SIGINT (in headless, we'll use a large number)

    // Parse --server=ADDR
    let server_addr = args
        .iter()
        .find(|a| a.starts_with("--server="))
        .and_then(|a| a.split('=').nth(1))
        .unwrap_or("127.0.0.1:12345")
        .parse()
        .unwrap();

    if headless {
        run_headless(duration_secs, server_addr).await;
        return;
    }

    let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
    let sender = socket.get_packet_sender();
    let receiver = socket.get_event_receiver();
    std::thread::spawn(move || socket.start_polling());

    let io = RealPacketIO {
        sender,
        receiver,
        server_addr,
    };
    let mut boxed: Box<dyn PacketIO> = Box::new(io);
    send_join_req(&mut *boxed);

    let world_data = Map::new();
    let mut game_world = World::new(world_data, boxed);

    loop {
        if is_key_pressed(KeyCode::P) {
            println!("{:#?}", game_world);
        }
        clear_background(BLACK);
        game_world.update(get_frame_time());
        game_world.draw();

        next_frame().await
    }
}
