//! Non-blocking join handshake helper, shared by the GUI and headless paths.
//!
//! The `Connecting` state drains the transport looking for the server's
//! `JoinResponse`. Timeout handling is deliberately the caller's job (the
//! runner compares `Instant::now()` against `connect_deadline`), which keeps
//! this helper deterministic and testable with a virtual clock.

use crate::network::packet_io::PacketIO;
use protocol::GamePacket;

/// Outcome of one non-blocking poll of the join handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeResult {
    /// A `JoinResponse` arrived, carrying the server-assigned player id.
    Joined { assigned_id: u8 },
    /// No `JoinResponse` yet (the transport queue emptied without one).
    Pending,
}

/// Drain pending packets looking for `JoinResponse`.
///
/// Returns `Joined` with the assigned id on the first `JoinResponse`
/// (discarding any packets that precede it and leaving any that follow it
/// queued), or `Pending` if the queue empties without one. Never blocks
/// (`try_recv`).
pub fn poll_join_response(io: &mut dyn PacketIO) -> HandshakeResult {
    while let Some(packet) = io.try_recv() {
        if let GamePacket::JoinResponse { assigned_id } = packet {
            return HandshakeResult::Joined { assigned_id };
        }
        // Discard any packet that precedes the JoinResponse.
    }
    HandshakeResult::Pending
}
