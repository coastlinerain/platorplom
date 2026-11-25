use metroidvania_rust::network::packet_io::PacketIO;
use protocol::GamePacket;
use std::sync::mpsc;

/// An in-memory PacketIO implementation backed by mpsc channels.
/// Carries serialized `Vec<u8>` (postcard) so the same serialization path is
/// exercised as in production — only laminar is replaced.
pub struct LoopbackTransport {
    /// Packets *from* the server arrive here (read by the client).
    pub rx: mpsc::Receiver<Vec<u8>>,
    /// Packets *to* the server are sent here (read by the harness' server side).
    pub tx: mpsc::Sender<Vec<u8>>,
}

impl PacketIO for LoopbackTransport {
    fn try_recv(&mut self) -> Option<GamePacket> {
        match self.rx.try_recv() {
            Ok(bytes) => postcard::from_bytes(&bytes).ok(),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => None,
        }
    }

    fn send(&mut self, packet: GamePacket) {
        if let Ok(bytes) = postcard::to_allocvec(&packet) {
            let _ = self.tx.send(bytes);
        }
    }
}

/// Creates a crossed pair so what A sends appears in B's recv queue and
/// vice versa. Returns (client_side, server_side).
pub fn create_loopback_pair() -> (LoopbackTransport, LoopbackTransport) {
    let (tx_a, rx_a) = mpsc::channel();
    let (tx_b, rx_b) = mpsc::channel();

    let client_side = LoopbackTransport { rx: rx_a, tx: tx_b };
    let server_side = LoopbackTransport { rx: rx_b, tx: tx_a };

    (client_side, server_side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::GamePacket;

    #[test]
    fn loopback_round_trip_join_request() {
        let (mut client_tr, mut server_tr) = create_loopback_pair();

        client_tr.send(GamePacket::JoinRequest);

        let received = server_tr.try_recv();
        assert_eq!(received, Some(GamePacket::JoinRequest));
    }

    #[test]
    fn loopback_round_trip_player_pos_preserves_fields() {
        let (mut client_tr, mut server_tr) = create_loopback_pair();

        let packet = GamePacket::PlayerPos {
            id: 42,
            x: 123.45,
            y: 678.90,
            dir: -1.0,
            level_x: 3,
            level_y: 7,
            last_seq: 999,
            server_tick: 12345,
            server_time_ms: 67890,
        };

        client_tr.send(packet);

        let received = server_tr.try_recv();
        match received {
            Some(GamePacket::PlayerPos {
                id,
                x,
                y,
                dir,
                level_x,
                level_y,
                last_seq,
                server_tick,
                server_time_ms,
            }) => {
                assert_eq!(id, 42);
                assert!((x - 123.45).abs() < 0.001);
                assert!((y - 678.90).abs() < 0.001);
                assert_eq!(dir, -1.0);
                assert_eq!(level_x, 3);
                assert_eq!(level_y, 7);
                assert_eq!(last_seq, 999);
                assert_eq!(server_tick, 12345);
                assert_eq!(server_time_ms, 67890);
            }
            other => panic!("expected PlayerPos, got {:?}", other),
        }
    }
}
