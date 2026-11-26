pub mod handshake;
pub mod interpolation;
pub mod logging;
pub mod packet_io;
pub mod prediction;
pub mod transport;
pub mod world;

// RED tests for the handshake helper (spec AC6, plan Box 2).
//
// `crate::network::handshake` does not exist yet, so this module fails to
// compile — the intended RED state. The test double below is deliberately local
// (not `e2e::LoopbackTransport`, which lives in the `e2e` crate) so the helper
// is tested against an in-memory queue with no transport dependency.
#[cfg(test)]
mod handshake_tests {
    use crate::network::handshake::{HandshakeResult, poll_join_response};
    use crate::network::packet_io::PacketIO;
    use protocol::GamePacket;
    use std::collections::VecDeque;

    /// Minimal `PacketIO`: an in-memory packet queue, no network.
    struct DummyPacketIO {
        inbox: VecDeque<GamePacket>,
    }

    impl DummyPacketIO {
        fn with_inbox(packets: Vec<GamePacket>) -> Self {
            Self {
                inbox: packets.into(),
            }
        }
    }

    impl PacketIO for DummyPacketIO {
        fn try_recv(&mut self) -> Option<GamePacket> {
            self.inbox.pop_front()
        }

        fn send(&mut self, _packet: GamePacket) {}
    }

    /// AC6: a queued `JoinResponse` is reported as `Joined` with its assigned id.
    #[test]
    fn poll_join_response_reports_joined_on_join_response() {
        let mut io = DummyPacketIO::with_inbox(vec![GamePacket::JoinResponse { assigned_id: 7 }]);

        assert_eq!(
            poll_join_response(&mut io),
            HandshakeResult::Joined { assigned_id: 7 }
        );
    }

    /// AC6: packets queued before the `JoinResponse` are discarded.
    #[test]
    fn poll_join_response_discards_packets_before_join_response() {
        let mut io = DummyPacketIO::with_inbox(vec![
            GamePacket::EnemyStateRequest {
                room_x: 0,
                room_y: 0,
            },
            GamePacket::JoinResponse { assigned_id: 3 },
        ]);

        assert_eq!(
            poll_join_response(&mut io),
            HandshakeResult::Joined { assigned_id: 3 }
        );
    }

    /// AC6: the helper returns at the first `JoinResponse`; packets queued after
    /// it are left for the game loop (plan: "returns on the first JoinResponse").
    #[test]
    fn poll_join_response_stops_at_the_first_join_response() {
        let mut io = DummyPacketIO::with_inbox(vec![
            GamePacket::JoinResponse { assigned_id: 5 },
            GamePacket::Leave { id: 9 },
        ]);

        assert_eq!(
            poll_join_response(&mut io),
            HandshakeResult::Joined { assigned_id: 5 }
        );
        assert_eq!(
            io.try_recv(),
            Some(GamePacket::Leave { id: 9 }),
            "packets after the JoinResponse must stay queued"
        );
    }

    /// AC6: an empty queue reports `Pending` immediately (never blocks).
    #[test]
    fn poll_join_response_reports_pending_on_empty_queue() {
        let mut io = DummyPacketIO::with_inbox(vec![]);

        assert_eq!(poll_join_response(&mut io), HandshakeResult::Pending);
    }

    /// AC6: non-join packets only report `Pending`, and the drain consumed them.
    #[test]
    fn poll_join_response_ignores_non_join_packets() {
        let mut io = DummyPacketIO::with_inbox(vec![
            GamePacket::Leave { id: 1 },
            GamePacket::EnemyStateRequest {
                room_x: 2,
                room_y: 3,
            },
        ]);

        assert_eq!(poll_join_response(&mut io), HandshakeResult::Pending);
        assert!(
            io.inbox.is_empty(),
            "the drain must consume every pending non-join packet"
        );
    }
}
