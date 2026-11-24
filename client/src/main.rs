mod hud;

use game::maps::Map;
use macroquad::prelude::*;
use metroidvania_rust::network::packet_io::PacketIO;
use metroidvania_rust::network::world::send_join_req;
use metroidvania_rust::world::game::World;
use std::net::SocketAddr;

/// RealPacketIO wraps the laminar socket for production use.
struct RealPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => postcard::from_bytes(p.payload()).ok(),
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
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

#[macroquad::main("Rustvania")]
async fn main() {
    let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
    let sender = socket.get_packet_sender();
    let receiver = socket.get_event_receiver();
    let server_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
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