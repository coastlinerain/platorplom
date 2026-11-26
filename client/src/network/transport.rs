use crate::app::TransportFactory;
use crate::network::logging::log_packet;
use crate::network::packet_io::PacketIO;
use protocol::GamePacket;
use std::net::SocketAddr;

/// RealPacketIO wraps the laminar socket for the GUI path.
struct RealPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<GamePacket>(p.payload()) {
                    log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: GamePacket) {
        log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            GamePacket::JoinRequest | GamePacket::RequestRoomChange { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

/// HeadlessPacketIO for running without rendering (CI/testing). Structurally
/// identical to `RealPacketIO`; kept separate so the headless factory closure
/// and the GUI factory closure each reference a concrete type.
struct HeadlessPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for HeadlessPacketIO {
    fn try_recv(&mut self) -> Option<GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<GamePacket>(p.payload()) {
                    log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: GamePacket) {
        log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            GamePacket::JoinRequest | GamePacket::RequestRoomChange { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

/// Build a transport factory that binds a fresh ephemeral socket per join. The
/// ephemeral-port bind is effectively infallible, so `.unwrap()` is an accepted,
/// documented exception (see plan notes).
pub fn make_transport_factory(server_addr: SocketAddr, headless: bool) -> TransportFactory {
    Box::new(move || {
        let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
        let sender = socket.get_packet_sender();
        let receiver = socket.get_event_receiver();
        std::thread::spawn(move || socket.start_polling());

        if headless {
            Box::new(HeadlessPacketIO {
                sender,
                receiver,
                server_addr,
            }) as Box<dyn PacketIO>
        } else {
            Box::new(RealPacketIO {
                sender,
                receiver,
                server_addr,
            }) as Box<dyn PacketIO>
        }
    })
}
