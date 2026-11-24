use game::entities::player::PlayerInput;

/// An entry in the unacknowledged-input ring buffer.
///
/// `shoot` is intentionally NOT stored — bullets are never re-spawned during
/// reconciliation replay (D7).
#[derive(Debug, Clone, PartialEq)]
pub struct InputEntry {
    pub seq: u16,
    pub move_left: bool,
    pub move_right: bool,
    pub jump: bool,
}

impl InputEntry {
    pub fn from_game_input(seq: u16, input: &PlayerInput) -> Self {
        Self {
            seq,
            move_left: input.move_left,
            move_right: input.move_right,
            jump: input.jump,
        }
    }
}

/// Fixed-capacity ring buffer of 64 unacknowledged inputs.
///
/// Uses wrapping sequence-number comparison with a 32767 threshold so that u16
/// wraparound is handled correctly even across the ~18-minute cycle.
#[derive(Debug)]
pub struct InputRingBuffer {
    entries: [Option<InputEntry>; 64],
    head: usize,
    len: usize,
}

impl InputRingBuffer {
    pub fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            head: 0,
            len: 0,
        }
    }

    /// Insert `entry`. If the buffer is full (64 entries), evict the oldest
    /// entry before inserting.
    pub fn push(&mut self, entry: InputEntry) {
        if self.len == 64 {
            // Evict the oldest entry: advance head past it.
            self.head = (self.head + 1) % 64;
            self.len -= 1;
        }
        let idx = (self.head + self.len) % 64;
        self.entries[idx] = Some(entry);
        self.len += 1;
    }

    /// Remove every entry whose seq is NOT newer than `last_seq`.
    pub fn drain_acknowledged(&mut self, last_seq: u16) {
        while self.len > 0 {
            let entry = match self.entries[self.head].as_ref() {
                Some(e) => e,
                None => break,
            };
            if !game::is_newer_seq(entry.seq, last_seq) {
                self.entries[self.head] = None;
                self.head = (self.head + 1) % 64;
                self.len -= 1;
            } else {
                break;
            }
        }
    }

    /// Iterate remaining entries in FIFO order (oldest → newest).
    pub fn replay_order(&self) -> impl Iterator<Item = &InputEntry> {
        (0..self.len).filter_map(move |i| {
            let idx = (self.head + i) % 64;
            self.entries[idx].as_ref()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl Default for InputRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Sticky edge helper for unreliable input transport.
///
/// Models the inline sticky logic:
///   1. If pressed: set sticky = 3.
///   2. flag = sticky > 0.
///   3. If sticky > 0: decrement.
///
/// Returns `(should_send, new_sticky)`.
/// Produces exactly 3 frames of `true` per press.
pub fn advance_sticky(sticky: u8, pressed: bool) -> (bool, u8) {
    let sticky = if pressed { 3 } else { sticky };
    let flag = sticky > 0;
    let next = if sticky > 0 { sticky - 1 } else { 0 };
    (flag, next)
}

/// Reconcile the player's position against an authoritative server snapshot.
///
/// 1. If `last_seq` is newer than `last_acked_seq` (via `game::is_newer_seq`):
///    updates `last_acked_seq`, drains acknowledged inputs from the buffer,
///    rewinds the player to `(server_x, server_y, server_dir)`, then replays
///    all remaining unacknowledged inputs (movement only — `shoot: false`)
///    at `1.0 / game::config::CLIENT_HZ` dt using the provided `level`.
///
/// 2. Returns the new `last_acked_seq` value (whether updated or not).
///
/// The `level` parameter is the collision level for the client's current room
/// (never the server-reported room — see AC 38).
#[allow(clippy::too_many_arguments)]
pub fn reconcile(
    player: &mut game::entities::player::Player,
    buffer: &mut InputRingBuffer,
    last_acked_seq: &mut u16,
    server_x: f32,
    server_y: f32,
    server_dir: f32,
    server_last_seq: u16,
    level: &game::level::Level,
) -> u16 {
    // 1. Stale check: only process if server_last_seq is newer.
    if !game::is_newer_seq(server_last_seq, *last_acked_seq) {
        return *last_acked_seq;
    }

    // 2. Update last_acked_seq.
    *last_acked_seq = server_last_seq;

    // 3. Discard acknowledged inputs.
    buffer.drain_acknowledged(server_last_seq);

    // 4. Rewind to server position/dir.
    player.pos = glam::Vec2::new(server_x, server_y);
    player.dir = server_dir;

    // 5. Replay unacknowledged inputs (movement only — shoot: false).
    let replay_dt = 1.0 / game::config::CLIENT_HZ as f32;
    for entry in buffer.replay_order() {
        let replay_input = game::entities::player::PlayerInput {
            move_left: entry.move_left,
            move_right: entry.move_right,
            jump: entry.jump,
            shoot: false, // D7: never re-spawn bullets during replay
        };
        player.update(replay_dt, level, &replay_input);
    }

    *last_acked_seq
}

#[cfg(test)]
mod tests {
    use super::{InputEntry, InputRingBuffer, advance_sticky};
    use game::entities::player::Player;
    use game::level::Level;
    use game::types::{EntityType, TileType};

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    /// A 12×12 empty level (480×480 px) so the player's (100,100) spawn is
    /// fully in bounds.
    fn empty_level() -> Level {
        Level::new(
            vec![vec![TileType::Empty; 12]; 12],
            vec![vec![EntityType::Empty; 12]; 12],
        )
    }

    /// Two 12×12 rooms side-by-side (columns 0 and 1), both empty.
    fn two_room_level_matrix() -> Vec<Vec<Level>> {
        vec![vec![empty_level(), empty_level()]]
    }

    // ---------------------------------------------------------------------------
    // Group B — InputRingBuffer tests
    // ---------------------------------------------------------------------------

    /// AC 6 — push entries and replay in FIFO order (oldest → newest).
    #[test]
    fn buffer_push_and_replay_is_fifo() {
        let mut buf = InputRingBuffer::new();

        buf.push(InputEntry {
            seq: 10,
            move_left: false,
            move_right: false,
            jump: false,
        });
        buf.push(InputEntry {
            seq: 20,
            move_left: false,
            move_right: false,
            jump: false,
        });
        buf.push(InputEntry {
            seq: 30,
            move_left: false,
            move_right: false,
            jump: false,
        });

        let entries: Vec<_> = buf.replay_order().collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].seq, 10);
        assert_eq!(entries[1].seq, 20);
        assert_eq!(entries[2].seq, 30);
    }

    /// AC 7 — drain_acknowledged removes entries ≤ last_seq; newer entries remain.
    #[test]
    fn buffer_drain_acknowledged_removes_older_seqs() {
        let mut buf = InputRingBuffer::new();

        for s in [1, 2, 3, 4] {
            buf.push(InputEntry {
                seq: s,
                move_left: false,
                move_right: false,
                jump: false,
            });
        }

        buf.drain_acknowledged(2);

        let entries: Vec<_> = buf.replay_order().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq, 3);
        assert_eq!(entries[1].seq, 4);
    }

    /// AC 8 — draining an empty buffer is a no-op (no panic).
    #[test]
    fn buffer_drain_acknowledged_empty_buffer_noop() {
        let mut buf = InputRingBuffer::new();
        assert!(buf.is_empty());

        buf.drain_acknowledged(5);

        assert!(buf.is_empty());
    }

    /// AC 9 — push beyond capacity evicts the oldest entry.
    #[test]
    fn buffer_push_full_evicts_oldest() {
        let mut buf = InputRingBuffer::new();

        // Fill with 64 entries (seq 0..63).
        for s in 0..64 {
            buf.push(InputEntry {
                seq: s as u16,
                move_left: false,
                move_right: false,
                jump: false,
            });
        }
        assert_eq!(buf.len(), 64);

        // Push one more — seq 0 should be evicted.
        buf.push(InputEntry {
            seq: 64,
            move_left: false,
            move_right: false,
            jump: false,
        });

        assert_eq!(buf.len(), 64);

        let entries: Vec<_> = buf.replay_order().collect();
        assert_eq!(entries.len(), 64);
        // First entry should be seq 1 (seq 0 was evicted), last should be seq 64.
        assert_eq!(entries[0].seq, 1);
        assert_eq!(entries[63].seq, 64);
    }

    /// Buffer empty reports is_empty = true, len = 0.
    #[test]
    fn buffer_empty_reports_empty() {
        let buf = InputRingBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    // ---------------------------------------------------------------------------
    // Group F — Sticky edge tests (advance_sticky)
    // ---------------------------------------------------------------------------

    /// AC 22, AC 24 — a single press produces exactly 3 frames of true.
    #[test]
    fn advance_sticky_press_produces_three_frames() {
        // Frame of press: sticky=0, pressed=true → sets to 3, returns true, decrements to 2.
        let (flag, next) = advance_sticky(0, true);
        assert!(flag);
        assert_eq!(next, 2);

        // Frame 2 (follow-up, not pressed): sticky=2, false → returns true, decrements to 1.
        let (flag, next) = advance_sticky(next, false);
        assert!(flag);
        assert_eq!(next, 1);

        // Frame 3 (last true frame): sticky=1, false → returns true, decrements to 0.
        let (flag, next) = advance_sticky(next, false);
        assert!(flag);
        assert_eq!(next, 0);

        // Frame 4 (expired): sticky=0, false → returns false, stays 0.
        let (flag, next) = advance_sticky(next, false);
        assert!(!flag);
        assert_eq!(next, 0);
    }

    /// Second frame of sticky countdown (sticky=2 → returns true, decrements to 1).
    #[test]
    fn advance_sticky_countdown_frame_1() {
        let (flag, next) = advance_sticky(2, false);
        assert!(flag);
        assert_eq!(next, 1);
    }

    /// Third frame of sticky countdown (sticky=1 → returns true, decrements to 0).
    #[test]
    fn advance_sticky_countdown_frame_2() {
        let (flag, next) = advance_sticky(1, false);
        assert!(flag);
        assert_eq!(next, 0);
    }

    /// After countdown expires, subsequent frames return false with sticky=0.
    #[test]
    fn advance_sticky_expired() {
        let (flag, next) = advance_sticky(0, false);
        assert!(!flag);
        assert_eq!(next, 0);
    }

    /// Re-pressing during the sticky window resets the counter.
    #[test]
    fn advance_sticky_repress_resets() {
        // Start at sticky=1 (mid-countdown), press again.
        let (flag, next) = advance_sticky(1, true);
        assert!(flag);
        // Press sets sticky to 3, then decrements → 2.
        assert_eq!(next, 2);
    }

    // ---------------------------------------------------------------------------
    // Group C — Reconciliation replay tests
    // ---------------------------------------------------------------------------

    /// AC 13 — replayed move_right inputs advance the player from the server position.
    #[test]
    fn replay_move_right_from_server_position() {
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);
        player.dir = 1.0;

        let mut buffer = InputRingBuffer::new();

        // Buffer 3 unacknowledged move_right inputs with seq 0, 1, 2.
        for s in 0..3 {
            buffer.push(InputEntry {
                seq: s,
                move_left: false,
                move_right: true,
                jump: false,
            });
        }

        // last_acked_seq starts at u16::MAX (no ack yet).
        // Server sends last_seq=1, acknowledging seq 0 and 1.
        // seq 2 survives for replay.
        let mut last_acked_seq = u16::MAX;
        let level = empty_level();

        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            100.0, // server x
            100.0, // server y
            1.0,   // server dir
            1,     // last_seq acknowledges seq 0,1; seq 2 survives
            &level,
        );

        // After reconciliation: rewind to (100,100), replay seq 2 (move_right=true).
        // Player should move right from the server position.
        assert!(
            player.pos.x > 100.0,
            "player should move right from (100,100) after replay; got x={}",
            player.pos.x
        );
        assert_eq!(player.dir, 1.0);
    }

    /// AC 14 — conflicting input: server corrects position, unacked move_left
    /// replays from the corrected position.
    #[test]
    fn replay_conflict_with_server_position() {
        let mut player = Player::new(0);
        // Player predicted it's at (200, 100).
        player.pos = glam::Vec2::new(200.0, 100.0);

        let mut buffer = InputRingBuffer::new();

        // Buffer 2 unacknowledged move_left inputs with seq 0 and 1.
        for s in 0..2 {
            buffer.push(InputEntry {
                seq: s,
                move_left: true,
                move_right: false,
                jump: false,
            });
        }

        // last_acked_seq starts at u16::MAX ("no ack yet").
        // last_seq=0 acknowledges seq 0 only; seq 1 survives for replay.
        let mut last_acked_seq = u16::MAX;
        let level = empty_level();

        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            50.0, // server says x=50 — conflict with client prediction
            100.0,
            1.0,
            0, // acknowledges seq 0; seq 1 survives for replay
            &level,
        );

        // After reconciliation: rewind to (50, 100), replay seq 1 (move_left=true).
        // Player should move left from 50.
        assert!(
            player.pos.x < 50.0,
            "player should move left from server position 50 after replay; got x={}",
            player.pos.x
        );
        assert_eq!(
            player.dir, -1.0,
            "dir should be -1.0 after replaying move_left; got {}",
            player.dir
        );
    }

    /// AC 25 — reconciliation replay forces shoot=false even if stored input
    /// had jump (and, indirectly, shoot is not stored at all in InputEntry).
    #[test]
    fn replay_shoot_forced_false() {
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);

        let mut buffer = InputRingBuffer::new();

        // Store a jump input with seq 5 (so it survives when we ack up to 4).
        // seq 5 leaves seq 5 unacked — it gets replayed.
        buffer.push(InputEntry {
            seq: 5,
            move_left: false,
            move_right: false,
            jump: true,
        });

        let bullets_before = player.bullets.len();

        // last_acked_seq=0, last_seq=4: seq 5 is newer than 4, seq 5 survives.
        let mut last_acked_seq = 0u16;
        let level = empty_level();

        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            100.0,
            100.0,
            1.0,
            4, // acknowledges up to 4; seq 5 survives for replay
            &level,
        );

        // No bullet should have been spawned during reconciliation replay.
        // (Shoot is never stored in InputEntry, and the replay forces shoot=false.)
        assert_eq!(
            player.bullets.len(),
            bullets_before,
            "reconciliation should not spawn bullets"
        );
    }

    /// AC 16 — the first PlayerPos after join sets last_acked_seq and clears
    /// pre-ack inputs.
    ///
    /// The client starts with `last_acked_seq = u16::MAX`. The first PlayerPos
    /// carries `last_seq = 0`. `is_newer_seq(0, u16::MAX)` is true, so
    /// reconciliation proceeds: `drain_acknowledged(0)` removes entries
    /// that are NOT newer than 0 (i.e., seq 0), and the player position is
    /// snapped to the authoritative value.
    ///
    /// In production, `JoinResponse` already resets `input_buffer` to empty,
    /// so the first PlayerPos typically sees zero entries. The test verifies
    /// the reconcile-path isolation: with a non-empty buffer, the first ack
    /// correctly drains at least seq 0 and snaps position.
    #[test]
    fn first_playerpos_sets_last_acked_and_clears_buffer() {
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);

        let mut buffer = InputRingBuffer::new();

        // Client has sent 1 input (seq 0) before receiving any PlayerPos.
        buffer.push(InputEntry {
            seq: 0,
            move_left: false,
            move_right: false,
            jump: false,
        });

        let mut last_acked_seq = u16::MAX; // initial value (no ack yet)
        let level = empty_level();

        // First PlayerPos arrives: last_seq=0.
        // is_newer_seq(0, u16::MAX) → 0.wrapping_sub(65535) = 1 → true.
        // drain_acknowledged(0) removes seq 0 (not newer than 0).
        // Buffer is now empty. Position snaps to (150, 200, -1.0).
        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            150.0, // server says x=150 (different from client's 100)
            200.0,
            -1.0,
            0, // first ack from server
            &level,
        );

        // After first reconciliation: buffer empty, position snapped to server.
        assert!(
            buffer.is_empty(),
            "buffer should be empty after first PlayerPos drains seq 0; got len={}",
            buffer.len()
        );
        assert_eq!(player.pos.x, 150.0);
        assert_eq!(player.pos.y, 200.0);
        assert_eq!(player.dir, -1.0);
    }

    /// AC 15 — a stale PlayerPos (same or behind last_acked_seq) is ignored.
    #[test]
    fn stale_playerpos_ignored() {
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);
        player.dir = 1.0;

        let mut buffer = InputRingBuffer::new();
        buffer.push(InputEntry {
            seq: 10,
            move_left: false,
            move_right: true,
            jump: false,
        });

        let mut last_acked_seq = 5u16; // we already ack'd up to seq 5
        let level = empty_level();

        // Server sends last_seq=3 (stale — older than 5).
        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            999.0, // different server position
            999.0,
            0.0,
            3, // stale last_seq
            &level,
        );

        // last_acked_seq should NOT have changed.
        assert_eq!(last_acked_seq, 5);

        // Buffer should still contain seq 10 (not drained).
        assert!(!buffer.is_empty());
        assert_eq!(buffer.replay_order().collect::<Vec<_>>().len(), 1);

        // Position should NOT have been overwritten to (999, 999).
        assert_eq!(player.pos.x, 100.0);
        assert_eq!(player.pos.y, 100.0);
        assert_eq!(player.dir, 1.0);
    }

    /// AC 38 (edge case) — replay uses the client's current room, not the
    /// server-reported coords.
    ///
    /// The server PlayerPos may report level_x/level_y different from the
    /// client's current_coords (e.g. during a room transition where the server
    /// lags behind). The replay MUST use the client's current room's collision
    /// level, NOT the server-reported room. The test verifies that calling
    /// reconcile with a level from a different room than the server reports
    /// completes without panic and produces a valid position.
    #[test]
    fn replay_uses_client_room_not_server_coords() {
        // Player is in room (0, 0). Server PlayerPos might claim room (1, 0)
        // but the client dictates which level is used for collision.
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);

        let mut buffer = InputRingBuffer::new();
        // Buffer seq 10 and 20 (so they survive drain by seq 5).
        buffer.push(InputEntry {
            seq: 10,
            move_left: false,
            move_right: true,
            jump: false,
        });
        buffer.push(InputEntry {
            seq: 20,
            move_left: false,
            move_right: true,
            jump: false,
        });

        // last_acked_seq=0, server reports last_seq=5: acknowledges 0..5,
        // seq 10 and 20 survive for replay.
        let mut last_acked_seq = 0u16;

        // Use the two-room matrix: client is in column 0.
        let matrix = two_room_level_matrix();
        let client_level = &matrix[0][0]; // room (col=0, row=0)

        // The test calls reconcile with client_level (room 0,0) even though
        // conceptually the server might report (1,0). The function must use the
        // provided level for collision, not the server's coords.
        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            100.0,
            100.0,
            1.0,
            5, // drain by 5 leaves seq 10 and 20
            client_level,
        );

        // The function should not panic. The player's position should be valid
        // (no OOB teleport) after replaying from the client's room.
        assert!(player.pos.x >= 0.0 && player.pos.x <= 480.0);
        assert!(player.pos.y >= 0.0 && player.pos.y <= 480.0);

        // Seq 10 and 20 were replayed with move_right=true → player moved right.
        assert!(
            player.pos.x > 100.0,
            "player should move right after replay; got x={}",
            player.pos.x
        );
    }

    /// AC 40 (edge case) — two PlayerPos in one frame: drain loop processes
    /// both correctly.
    #[test]
    fn multiple_reconciliation_in_one_frame() {
        let mut player = Player::new(0);
        player.pos = glam::Vec2::new(100.0, 100.0);

        let mut buffer = InputRingBuffer::new();
        // Buffer seq 1..6.
        for s in 1..=6 {
            buffer.push(InputEntry {
                seq: s,
                move_left: false,
                move_right: true,
                jump: false,
            });
        }

        let mut last_acked_seq = 0u16;
        let level = empty_level();

        // First PlayerPos: last_seq=3. Should acknowledge seq 1,2,3.
        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            110.0, // server position
            110.0,
            1.0,
            3,
            &level,
        );
        assert_eq!(last_acked_seq, 3);

        // Second PlayerPos (same frame): last_seq=5. Should acknowledge seq 4,5.
        let _ = super::reconcile(
            &mut player,
            &mut buffer,
            &mut last_acked_seq,
            120.0, // more recent server position
            120.0,
            1.0,
            5,
            &level,
        );
        assert_eq!(last_acked_seq, 5);

        // After both reconciliations: only seq 6 remains.
        let remaining: Vec<_> = buffer.replay_order().collect();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].seq, 6);
    }
}
