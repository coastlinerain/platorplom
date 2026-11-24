use protocol::GamePacket;

/// Abstracts network I/O so `World` can run against real laminar or a loopback
/// transport.  The real impl dispatches through the same laminar channel; the
/// loopback impl uses `std::sync::mpsc` channels carrying serialized `Vec<u8>`.
pub trait PacketIO {
    /// Try to receive one deserialized `GamePacket`. Returns `None` if no packet
    /// is available (non-blocking).
    fn try_recv(&mut self) -> Option<GamePacket>;

    /// Serialize `packet` and send it to the server (or loopback channel).
    fn send(&mut self, packet: GamePacket);
}