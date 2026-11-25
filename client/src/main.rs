mod hud;

use game::config::{HEADLESS_BUSY_WAIT_MS, MAX_ACCUMULATED_FRAMES, TICK_HZ};
use game::fixed_timestep::FixedTimestep;
use game::maps::Map;
use macroquad::prelude::*;
use metroidvania_rust::network::logging::log_packet;
use metroidvania_rust::network::packet_io::PacketIO;
use metroidvania_rust::network::world::send_join_req;
use metroidvania_rust::world::game::World;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

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

fn run_headless(duration_secs: u64, server_addr: SocketAddr) {
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
    let tick_interval = Duration::from_secs_f64(1.0 / TICK_HZ as f64);
    let max_ticks = if duration_secs > 0 {
        (duration_secs as f32 * TICK_HZ as f32) as u64
    } else {
        u64::MAX
    };

    println!(
        "Headless client running for {} seconds ({} ticks)",
        duration_secs, max_ticks
    );

    // Fixed timestep with no drift: advance `next_tick` by a constant interval
    // each iteration, then sleep for the bulk of the interval and busy-wait the
    // last ~HEADLESS_BUSY_WAIT_MS for sub-millisecond precision (spec D2).
    let mut next_tick = Instant::now() + tick_interval;

    for tick in 0..max_ticks {
        game_world.update(tick_dt);

        // Periodic status log
        if tick % (TICK_HZ as u64 * 5) == 0 && tick > 0 {
            println!("Headless tick: {}", tick);
        }

        let now = Instant::now();
        if now < next_tick {
            let remaining = next_tick - now;
            // Sleep for most of the interval, then busy-wait the remainder.
            if remaining > Duration::from_millis(HEADLESS_BUSY_WAIT_MS) {
                std::thread::sleep(remaining - Duration::from_millis(HEADLESS_BUSY_WAIT_MS));
            }
            while Instant::now() < next_tick {
                std::hint::spin_loop();
            }
        }
        next_tick += tick_interval;
    }

    println!("Headless client finished after {} ticks", max_ticks);
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
        run_headless(duration_secs, server_addr);
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

    // Fixed-timestep accumulator (spec D1): separate the simulation dt (fixed
    // 1/60s) from the render dt (variable). `FixedTimestep` owns the
    // accumulate → clamp → step → alpha logic (shared with the e2e tests).
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);
    let tick_dt = 1.0 / TICK_HZ as f32;
    let mut last_time = get_time();

    loop {
        // Sample keyboard once per render frame and inject into World fields.
        // (Input sampling must live here, not in World::update, so the sim is
        // testable without a macroquad window.)
        game_world.move_left = is_key_down(KeyCode::Left);
        game_world.move_right = is_key_down(KeyCode::Right);
        game_world.pressed_jump = is_key_pressed(KeyCode::Space);
        game_world.pressed_shoot = is_key_pressed(KeyCode::Z);

        // Debug: F11 key to manually reset clock sync (moved from World::update).
        if is_key_pressed(KeyCode::F11) {
            game_world.client_tick = 0;
            game_world.server_tick_offset = 0;
            game_world.clock_offset_ms = 0;
            game_world.last_server_tick = 0;
            game_world.last_server_time_ms = 0;
            game_world.last_acked_server_tick = 0;
            println!("Manual clock sync reset (F11)");
        }

        if is_key_pressed(KeyCode::P) {
            println!("{:#?}", game_world);
        }
        clear_background(BLACK);

        // Accumulate the real frame time (clamped to avoid the spiral of death).
        let current_time = get_time();
        let frame_dt = current_time - last_time;
        last_time = current_time;

        // Inject the current viewport once per render frame (the camera does
        // not query `screen_width`/`screen_height` itself, so `World::update`
        // stays headless-testable). Must run BEFORE the accumulator loop below.
        game_world
            .camera
            .set_viewport(screen_width(), screen_height());

        // Run 0..=MAX_ACCUMULATED_FRAMES fixed-dt simulation steps.
        let steps = timestep.advance(frame_dt);
        for _ in 0..steps {
            game_world.update(tick_dt);
        }

        // Interpolation alpha ∈ [0, 1): 0.0 = previous tick, ~1.0 = next tick.
        game_world.draw(timestep.alpha());

        next_frame().await
    }
}
