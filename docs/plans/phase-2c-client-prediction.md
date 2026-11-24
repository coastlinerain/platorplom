# Phase 2c — Client prediction + reconciliation — Implementation Plan

- Status: draft (reviewed 2026-11-24; see fixes below)
- Spec: `docs/specs/phase-2c-client-prediction.md`
- Date: 2026-11-24
- Branch: `feat/client-prediction`

## Overview

Phase 2b made the server authoritative for player movement, at the cost of visible
input latency: a keypress travels client → server → back before movement appears
(~1 RTT). Phase 2c eliminates that latency by adding **client-side prediction**
and **reconciliation**, the standard netcode technique for authoritative-server
games.

The client resumes simulating its own player locally every frame (exactly as it
did before 2b), showing the predicted position immediately. Each outgoing input
carries a `seq: u16` and the server echoes the highest processed `seq` back as
`last_seq` in every `PlayerPos`. On authoritative position arrival, the client
discards acknowledged inputs and **re-simulates** any remaining unacknowledged
inputs (movement only — `shoot: false`) from the server-corrected state.

Remote players are smoothed via **entity interpolation** (lerp between the last
two `PlayerPos` snapshots), eliminating the 20 Hz stepping artifact.

Input transport switches from reliable-ordered to **unreliable** with 3-frame
sticky edge flags for jump/shoot. Sequence numbers handle the detection and
correction of dropped inputs.

**Scope boundary** (unchanged from 2b): only *movement* is authoritative. Enemies,
bullets, room transitions, and death handling remain client-side.

### ⚠ This phase restores client-side self-simulation

Unlike 2b (which removed `player.update` from `World::update`), this phase
**re-adds** it — the local player moves immediately on keypress, with the server
correcting in the background. The reviewer and QA must check:

1. **Prediction runs before network.** `World::update` calls
   `self.player.update(dt, level, &raw_input)` first, then `update_network`
   (which may correct via reconciliation). The frame shows the predicted position.
2. **Reconciliation is invisible under normal conditions.** On LAN, the prediction
   matches the server's simulation exactly (same `Player::update` code, same
   inputs), so `pos` and `dir` do not visibly jump on reconciliation — the
   corrected values are identical to the predicted ones.
3. **Sticky edge flags mean 3 frames of jump/shoot sent.** A single `Space` press
   produces 3 unreliable packets with `jump: true`. The server's edge-OR-latch
   deduplicates naturally. Under nominal LAN conditions, the jump arrives on the
   first frame; the sticky edge is insurance against the unreliable channel.
4. **Entity interpolation renders remote players smoothly.** Remote `Nemesis`
   positions are lerp'd between the last 2 snapshots. Under stable 20 Hz
   delivery, this produces continuous motion with no visible stepping.

### Decision: `CLIENT_HZ = 60` added to `game/src/config.rs`

The spec's D9 says "the client derives the replay dt and interpolation interval
from the same constant" (`TICK_HZ = 20`), but using 1/20 (50 ms) as the
replay step would produce ~3× more integration error per replayed input than
1/60 (~16.7 ms), because the client's original inputs were sampled at ~60 Hz.
**Decision:** add `pub const CLIENT_HZ: u32 = 60;` to `game/src/config.rs`.
The replay dt = `1.0 / CLIENT_HZ as f32`. This is a new constant but a pragmatic
one: the value must match the client's approximate frame rate, not the server's
tick rate, and `TICK_HZ` (20) is semantically the wrong constant to use here.

## Public API (targets for the tester)

### `protocol/src/lib.rs` — two variants gain fields

```rust
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    // ... existing variants unchanged: JoinRequest, JoinResponse, Action, Leave, Hit

    PlayerPos {
        id: u8,
        x: f32,
        y: f32,
        dir: f32,
        level_x: u8,
        level_y: u8,
        last_seq: u16,     // NEW — highest input seq processed by the server for this player
    },

    PlayerInput {
        id: u8,
        level_x: u8,
        level_y: u8,
        move_left: bool,
        move_right: bool,
        jump: bool,
        shoot: bool,
        seq: u16,          // NEW — monotonically-increasing input sequence number
    },
}
```

### `game/src/config.rs` — one new constant

```rust
/// Approximate client frame rate, used for replay dt during reconciliation and
/// interpolation interval clamping. Must match the typical display refresh rate.
pub const CLIENT_HZ: u32 = 60;
```

### `client/src/network/prediction.rs` (new file)

```rust
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
    pub fn from_game_input(seq: u16, input: &PlayerInput) -> Self { ... }
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
    pub fn new() -> Self;

    /// Insert `entry`. If the buffer is full (64 entries), evict the oldest
    /// entry before inserting (oldest acknowledged by then under normal
    /// conditions).
    pub fn push(&mut self, entry: InputEntry);

    /// Remove every entry whose seq is NOT newer than `last_seq`.
    /// Uses `is_newer_seq` for wrapping-aware comparison: entries with
    /// `seq` equal to `last_seq` or behind it (or too far behind — >32767)
    /// are removed.
    pub fn drain_acknowledged(&mut self, last_seq: u16);

    /// Iterate remaining entries in FIFO order (oldest → newest).
    pub fn replay_order(&self) -> impl Iterator<Item = &InputEntry>;

    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
}

/// Wrapping-aware sequence comparison. Returns `true` when `seq_a` is ahead of
/// `seq_b` by 1..32767 (inclusive), `false` otherwise (including equal and
/// behind). The threshold of 32767 (half the u16 range) correctly distinguishes
/// "newer" from "far behind due to wraparound."
pub fn is_newer_seq(seq_a: u16, seq_b: u16) -> bool {
    seq_a.wrapping_sub(seq_b) > 0 && seq_a.wrapping_sub(seq_b) < 32768
}
```

`client/src/network/mod.rs` gains `pub mod prediction;`.

### `client/src/network/interpolation.rs` (new file)

```rust
use game::entities::nemesis::Nemesis;
use std::time::Instant;

/// Per-remote-player interpolation state (client-only, not in `game`).
///
/// `Nemesis` holds the raw positions (`interp_snapshots`); this struct holds the
/// arrival timestamps needed to compute the lerp factor.
pub struct RemoteInterp {
    /// When each snapshot arrived. Parallels `nemesis.interp_snapshots`:
    /// `timestamps[0]` corresponds to the older snapshot (if any),
    /// `timestamps[1]` to the newer.
    pub timestamps: [Option<Instant>; 2],
}

impl RemoteInterp {
    pub fn new() -> Self;

    /// Push a new snapshot into the Nemesis and record its arrival time.
    ///
    /// Internally calls `nemesis.push_interp_snapshot(x, y, dir)` and shifts
    /// timestamps to keep the 2 most recent entries in sync.
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32);

    /// Compute interpolated `(x, y, dir)` from the Nemesis snapshot buffer and
    /// the recorded timestamps.
    ///
    /// - **0 snapshots**: returns `None`.
    /// - **1 snapshot**: returns that snapshot's `(x, y, dir)` (snap).
    /// - **2 snapshots**: lerps position between older and newer based on
    ///   elapsed time since the older timestamp, clamped to [0, 1] so that
    ///   elapsed > interval snaps to the newer position. `dir` always snaps to
    ///   the newer snapshot.
    /// - **Stale guard**: if the newer snapshot's timestamp is >200 ms old,
    ///   snaps to the newer position (treats the remote as disconnected).
    pub fn interpolate(&self, nemesis: &Nemesis) -> Option<(f32, f32, f32)>;

    /// Same as `interpolate` but accepts an explicit `now` for testability.
    /// `interpolate` delegates to this with `Instant::now()`.
    pub fn interpolate_at(&self, nemesis: &Nemesis, now: Instant) -> Option<(f32, f32, f32)>;
}
```

`client/src/network/mod.rs` gains `pub mod interpolation;`.

### `game/src/entities/nemesis.rs` — updated

```rust
#[derive(Debug)]
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,              // raw latest position (used for bullet spawn compat)
    pub w: f32,
    pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
    // --- new interpolation fields ---
    /// Ring of 2 position snapshots: oldest at index 0, newest at index 1
    /// (or empty slots). Each snapshot is `(x, y, dir)`.
    pub interp_snapshots: [Option<(f32, f32, f32)>; 2],
    /// Number of valid snapshots (0, 1, or 2).
    pub interp_len: u8,
}

impl Nemesis {
    pub fn new(id: u8, pos: Vec2) -> Self {
        Self {
            id, pos,
            w: 30.0, h: 38.0,
            last_dir: 1.0,
            coords: (0, 0),
            interp_snapshots: [None, None],
            interp_len: 0,
        }
    }

    /// Push a new `(x, y, dir)` snapshot into the 2-entry ring.
    /// On the third push, the oldest is evicted (shift [1]→[0], put new at [1]).
    pub fn push_interp_snapshot(&mut self, x: f32, y: f32, dir: f32);
}
```

**Why `Nemesis` owns the snapshots and not `RemoteInterp`:** `Nemesis` lives in
the `game` crate (shared, no `std::time`), while `RemoteInterp` lives in the
client and owns `Instant` timestamps. Splitting the data keeps the game crate
timing-agnostic.

### `client/src/world/game.rs` — `World` struct + `update` + `draw`

```rust
use crate::network::prediction::InputRingBuffer;
use crate::network::interpolation::RemoteInterp;
use std::collections::HashMap;

pub struct World {
    // --- existing fields unchanged ---
    pub id: Option<u8>,
    pub player: Player,
    pub enemies: Vec<Enemy>,
    pub levels_matrix: Vec<Vec<Level>>,
    pub current_coords: (usize, usize),
    pub camera: GameCamera,
    pub network: NetworkClient,
    pub other_players: HashMap<u8, Nemesis>,
    pub enemy_bullets: Vec<Bullet>,
    pub bullets: Vec<Bullet>,

    // --- new prediction fields ---
    pub input_seq: u16,                          // next sequence number to send
    pub last_acked_seq: u16,                     // highest last_seq from server
    pub input_buffer: InputRingBuffer,           // unacknowledged inputs for replay
    pub jump_sticky: u8,                         // remaining frames to send jump: true
    pub shoot_sticky: u8,                        // remaining frames to send shoot: true
    pub other_interp: HashMap<u8, RemoteInterp>, // per-remote-player interpolation state
}
```

`World::new` initializes `input_seq: u16::MAX` (so the first increment wraps to 0),
`last_acked_seq: u16::MAX` (so the first `PlayerPos` with `last_seq: 0` is
recognized as newer and processed as an initial snap), and empty `input_buffer`,
`other_interp`, zero `jump_sticky`/`shoot_sticky`.

`World::update`:
```rust
pub fn update(&mut self, dt: f32) {
    check_level_transitions(self);
    update_enemy_bullets_position(self, dt);
    check_player_death(self);

    {
        let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];

        // --- CLIENT PREDICTION: simulate local player immediately ---
        let local_input = game::entities::player::PlayerInput {
            move_left: is_key_down(KeyCode::Left),
            move_right: is_key_down(KeyCode::Right),
            jump: is_key_pressed(KeyCode::Space),    // raw edge for local feel
            shoot: false,                            // bullet spawn is separate (AC 32)
        };
        self.player.update(dt, level, &local_input);

        // Own-bullet spawn (unchanged from 2b: once per Z press).
        if is_key_pressed(KeyCode::Z) {
            self.bullets.push(Bullet::new(
                self.player.pos + glam::Vec2::new(self.player.w / 2.0, self.player.h / 2.0),
                self.player.dir,
                self.player.id,
            ));
        }

        // Own-bullet update and collision (unchanged from 2b).
        for bullet in &mut self.bullets { bullet.update(dt, level); }
        for bullet in &mut self.bullets {
            for enemy in &mut self.enemies {
                if bullet.check_collision(enemy) {
                    bullet.active = false;
                    enemy.die();
                }
            }
        }

        // Enemy simulation (unchanged).
        for enemy in &mut self.enemies {
            let random = macroquad::rand::gen_range(0, 4);
            enemy.update(dt, level, random);
        }

        self.camera.update(self.player.pos, vec2(self.player.w, self.player.h), level);
    }

    update_network(self);

    self.bullets.retain(|b| b.active);
    self.enemies.retain(|e| e.alive);
}
```

### `client/src/network/world.rs` — `update_network` (modified)

```rust
pub fn update_network(world: &mut World) {
    // Receive first (may correct before we send).
    while let Ok(event) = world.network.receiver.try_recv() {
        if let SocketEvent::Packet(packet) = event
            && let Ok(decoded) = postcard::from_bytes::<GamePacket>(packet.payload())
        {
            handle_packet(world, decoded);
        }
    }

    // Send inputs.
    if let Some(id) = world.id {
        // --- Sticky edge handling (D6) ---
        let (jump, jump_sticky) = advance_sticky(world.jump_sticky, is_key_pressed(KeyCode::Space));
        let (shoot, shoot_sticky) = advance_sticky(world.shoot_sticky, is_key_pressed(KeyCode::Z));
        world.jump_sticky = jump_sticky;
        world.shoot_sticky = shoot_sticky;

        // --- Sequence number ---
        world.input_seq = world.input_seq.wrapping_add(1);
        let seq = world.input_seq;

        let move_left = is_key_down(KeyCode::Left);
        let move_right = is_key_down(KeyCode::Right);

        let input_msg = GamePacket::PlayerInput {
            id,
            level_x: world.current_coords.0 as u8,
            level_y: world.current_coords.1 as u8,
            move_left,
            move_right,
            jump,
            shoot,
            seq,
        };

        // Store for reconciliation replay (shoot is NOT stored — D7).
        world.input_buffer.push(InputEntry {
            seq,
            move_left,
            move_right,
            jump,
        });

        let bytes = postcard::to_allocvec(&input_msg).unwrap();
        let _ = world.network.sender.send(laminar::Packet::unreliable(
            world.network.server_addr,
            bytes,
        ));
    }
}
```

### `client/src/network/world.rs` — `handle_packet` (modified)

```rust
fn handle_packet(world: &mut World, decoded: GamePacket) {
    match decoded {
        GamePacket::JoinResponse { assigned_id } => {
            world.id = Some(assigned_id);
            world.player.id = assigned_id;
            // Reset ack state for the new session.
            world.input_seq = u16::MAX;
            world.last_acked_seq = u16::MAX;
            world.input_buffer = InputRingBuffer::new();
            world.jump_sticky = 0;
            world.shoot_sticky = 0;
            println!("Connected! My ID is {}", assigned_id);
        }
        GamePacket::PlayerPos {
            id, x, y, dir, level_x, level_y, last_seq,
        } => {
            if Some(id) == world.id {
                // --- RECONCILIATION for own player ---
                if is_newer_seq(last_seq, world.last_acked_seq) {
                    world.last_acked_seq = last_seq;

                    // Discard acknowledged inputs.
                    world.input_buffer.drain_acknowledged(last_seq);

                    // Rewind to server position/dir.
                    // (vel and grounded are NOT corrected — PlayerPos does not
                    // carry them. Replaying unacknowledged inputs from the
                    // server position recomputes them naturally via
                    // Player::update's physics.)
                    world.player.pos = vec2(x, y);
                    world.player.dir = dir;

                    // Replay unacknowledged inputs (movement only — D7).
                    if !world.input_buffer.is_empty() {
                        let level = &world.levels_matrix
                            [world.current_coords.1][world.current_coords.0];
                        let replay_dt = 1.0 / game::config::CLIENT_HZ as f32;
                        for entry in world.input_buffer.replay_order() {
                            let replay_input = game::entities::player::PlayerInput {
                                move_left: entry.move_left,
                                move_right: entry.move_right,
                                jump: entry.jump,
                                shoot: false,      // D7: never re-spawn bullets
                            };
                            world.player.update(replay_dt, level, &replay_input);
                        }
                    }
                }
            } else {
                // --- Remote player: push interpolation snapshot ---
                let nemesis = world.other_players.entry(id).or_insert_with(|| {
                    let mut n = Nemesis::new(id, vec2(x, y));
                    n.last_dir = dir;
                    n.coords = (level_x as usize, level_y as usize);
                    n
                });
                // Update raw pos/dir/coords for non-interpolation fallback.
                nemesis.pos = vec2(x, y);
                nemesis.last_dir = dir;
                nemesis.coords = (level_x as usize, level_y as usize);

                let interp = world.other_interp.entry(id).or_insert_with(RemoteInterp::new);
                interp.push(nemesis, x, y, dir);
            }
        }
        GamePacket::Action { id, kind, dir } => {
            if kind == "shoot" {
                if let Some(enemy) = world.other_players.get(&id) {
                    let spawn_center = vec2(
                        enemy.pos.x + (enemy.w / 2.0),
                        enemy.pos.y + (enemy.h / 2.0),
                    );
                    if enemy.coords == world.current_coords {
                        world.enemy_bullets.push(Bullet::new(spawn_center, dir, enemy.id));
                    }
                }
            }
        }
        GamePacket::Leave { id } => {
            world.other_players.remove(&id);
            world.other_interp.remove(&id);
            println!("Player {} left. Removed from world.", id);
        }
        _ => {}
    }
}
```

### `server/src/world.rs` — `ServerWorld` + `apply_input`

```rust
pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords:  HashMap<u8, (u8, u8)>,
    levels:  Vec<Vec<Level>>,
    // --- new ---
    max_seq: HashMap<u8, u16>,   // highest seq received per player
}

impl ServerWorld {
    // new(), spawn(), remove(), update(), players(), coords() — unchanged

    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16) {
        // ... existing accumulator logic unchanged (coords insert, held overwrite,
        //     edge OR-accumulate) ...

        // Track the highest received sequence number.
        let current = self.max_seq.entry(id).or_insert(0);
        if is_newer_seq(seq, *current) {
            *current = seq;
        }
    }

    pub fn max_seq(&self) -> &HashMap<u8, u16> { &self.max_seq }
}
```

`new` initializes `max_seq: HashMap::new()`. `spawn` inserts `max_seq.insert(id, 0)`.
`remove` drops `max_seq.remove(&id)`.

Note: `is_newer_seq` lives in `game/src/lib.rs` as `pub fn is_newer_seq(a: u16, b: u16) -> bool`
and is used by both the client (`prediction.rs`) and the server (`world.rs`) via
`use game::is_newer_seq;`.

### `server/src/main.rs` — tick loop (modified)

In the `PlayerInput` match arm (line ~53 of current `main.rs`), extract `seq`:

```rust
GamePacket::PlayerInput {
    id, level_x, level_y, move_left, move_right, jump, shoot, seq,
} => {
    let resolved = addrs.get(&addr).copied();
    if resolved != Some(id) { continue; }
    world.apply_input(
        id, level_x, level_y,
        game::entities::player::PlayerInput { move_left, move_right, jump, shoot },
        seq,
    );
}
```

In the `PlayerPos` broadcast loop (line ~132), include `last_seq`:

```rust
let last_seq = world.max_seq().get(&id).copied().unwrap_or(0);
let bytes = postcard::to_allocvec(&GamePacket::PlayerPos {
    id,
    x: player.pos.x,
    y: player.pos.y,
    dir: player.dir,
    level_x: lx,
    level_y: ly,
    last_seq,
}).unwrap();
```

### `client/src/render.rs` — interpolated Nemesis draw

```rust
/// Draw a Nemesis at an interpolated position instead of its raw `.pos`.
/// Called from `World::draw` when `RemoteInterp::interpolate` returns a value.
pub fn draw_nemesis_at(nemesis: &Nemesis, x: f32, y: f32, dir: f32) {
    draw_rectangle(x, y, nemesis.w, nemesis.h, ORANGE);

    let eye_x = if dir > 0.0 { x + 20.0 } else { x + 5.0 };
    draw_rectangle(eye_x, y + 10.0, 5.0, 5.0, WHITE);

    draw_text(
        format!("P: {}", nemesis.id),
        x,
        y - 10.0,
        15.0,
        WHITE,
    );
}
```

`Drawable for Nemesis` stays unchanged (draws raw `.pos`/`.last_dir`) as the
fallback path.

### `client/src/world/game.rs` — `draw` (modified)

The remote-player draw loop uses interpolation when available:

```rust
// Remote players: interpolated when available, raw fallback otherwise.
for (id, nemesis) in &self.other_players {
    if nemesis.coords == self.current_coords {
        if let Some(interp) = self.other_interp.get(id) {
            if let Some((ix, iy, idir)) = interp.interpolate(nemesis) {
                crate::render::draw_nemesis_at(nemesis, ix, iy, idir);
                continue;
            }
        }
        nemesis.draw();
    }
}
```

## Test cases (tester writes these first)

Legend: **[NEW]** = genuinely new, RED until the noted box lands. **[MODIFIED]** =
existing test whose assertion or input must be updated. **[UNCHANGED]** = already
passing, must stay green. **[CODE-READ]** = verified by grep, not a unit test.

---

### Group A — Protocol (AC 1–5)

These are in `protocol/tests/round_trip.rs`.

**[NEW]** `round_trip_player_input_seq` — round-trips `PlayerInput` with a
non-zero `seq`:
```rust
GamePacket::PlayerInput {
    id: 7, level_x: 1, level_y: 2,
    move_left: true, move_right: false, jump: true, shoot: false,
    seq: 42,
}
// Also: seq = 0 (backward compat), seq = u16::MAX (wraparound)
```

**[MODIFIED]** `round_trip_player_pos` — add `last_seq: 0` to both existing test
values; add a third value with `last_seq: 42`.

**[NEW]** `round_trip_player_input_seq_wraparound` — `seq: u16::MAX` serializes
and deserializes to `u16::MAX`.

**[UNCHANGED]** All 6 other round-trip tests (`join_request`, `join_response`,
`action`, `leave`, `hit`, `player_input` with seq=0) — existing assertions are
compatible, just make sure `player_input` includes `seq: 0`.

---

### Group B — Input ring buffer (AC 6–10)

These are in `client/src/network/prediction.rs` `#[cfg(test)] mod tests`.

**[NEW]** `buffer_push_and_replay_is_fifo` — push entries with seq 10, 20, 30;
`replay_order()` yields them in that order (oldest → newest): AC 6.

**[NEW]** `buffer_drain_acknowledged_removes_older_seqs` — push seq 1, 2, 3, 4;
`drain_acknowledged(2)`; remaining replay_order yields seq 3, 4 only: AC 7.

**[NEW]** `buffer_drain_acknowledged_empty_buffer_noop` — call
`drain_acknowledged(5)` on an empty buffer; no panic, `is_empty()` still true: AC 8.

**[NEW]** `buffer_push_full_evicts_oldest` — push 64 entries (seq 0..63), then
push seq 64; replay_order yields seq 1..64 (len=64, seq 0 evicted): AC 9.

**[NEW]** `is_newer_seq_ahead` — `is_newer_seq(10, 5)` → true: AC 10.

**[NEW]** `is_newer_seq_equal` — `is_newer_seq(5, 5)` → false.

**[NEW]** `is_newer_seq_behind` — `is_newer_seq(5, 10)` → false.

**[NEW]** `is_newer_seq_wraparound` — `is_newer_seq(0, u16::MAX)` → true;
`is_newer_seq(u16::MAX, 0)` → false; `is_newer_seq(1, 65535)` → true (within
threshold).

**[NEW]** `is_newer_seq_beyond_threshold` — `is_newer_seq(40000, 0)` → false
(a difference > 32767 means the smaller value is actually newer after wraparound).

---

### Group C — Reconciliation replay (AC 11–16, 25)

These are in `client/src/network/prediction.rs` `#[cfg(test)] mod tests`.
They use `game::entities::player::{Player, PlayerInput}` and a simple empty
`Level` (same 12×12 `TileType::Empty` fixture as `game`'s player tests), with
`replay_dt = 1.0 / CLIENT_HZ as f32`.

**[NEW]** `replay_move_right_from_server_position` — AC 13. Construct a player at
(100, 100), dir=1. Buffer 3 `move_right: true` inputs. Replay them at 1/60 dt.
Assert `player.pos.x > 100.0` and `player.dir == 1.0`.

**[NEW]** `replay_conflict_with_server_position` — AC 14. Construct a player at
(100, 100). Reconcile to server position (50, 100), dir=1. Buffer 2 `move_left: true`
inputs. Replay them at 1/60 dt. Assert `player.pos.x < 50.0` and
`player.dir == -1.0` (the player was moved left from the corrected position).

**[NEW]** `replay_shoot_forced_false` — AC 25. A stored `InputEntry` has `jump: false`.
Replay passes `shoot: false`. Verify `player.bullets.is_empty()` after replay
(no bullet spawned).

**[NEW]** `first_playerpos_sets_last_acked_and_clears_buffer` — AC 16. Simulate:
buffer has 3 entries (seq 0, 1, 2), `last_acked_seq = u16::MAX`. Call
`drain_acknowledged(0)` (like the first `PlayerPos` with `last_seq: 0` arrives,
`is_newer_seq(0, u16::MAX)` → true). Verify the buffer is now empty.

**[NEW]** `stale_playerpos_ignored` — AC 15. `last_acked_seq = 5`. A `PlayerPos`
arrives with `last_seq = 3`. `is_newer_seq(3, 5)` → false. Verify no buffer drain
occurs (the buffer retains all entries).

**[NEW]** `replay_uses_client_room_not_server_coords` — AC 38 (edge case). A
player is in room `(0, 0)` with `current_coords = (0, 0)`. The `PlayerPos`
carries `level_x: 1, level_y: 0` (server thinks the player changed rooms, but
client hasn't transitioned yet). Replay uses the collision level at
`(0, 0)` (the client's current room), not `(1, 0)`. Verify the replay
completes without panic and the player's position is valid (no out-of-bounds
teleport). Requires a fixture with at least 2 rooms side by side.

**[NEW]** `multiple_reconciliation_calls_in_one_frame` — AC 40 (edge case).
Buffer: seq 1..6. `last_acked_seq = 0`. Two `PlayerPos` arrive in the same
drain loop: first with `last_seq = 3`, second with `last_seq = 5`. After both:
`last_acked_seq = 5`, buffer retains only seq 6, player is at the position
produced by replaying seq 6 from the state after the second rewind. Verify
the buffer is trimmed correctly and the final position matches replay from the
more recent server position.

---

### Group D — Entity interpolation (AC 17–21)

These are in `client/src/network/interpolation.rs` `#[cfg(test)] mod tests`.

**[NEW]** `interpolate_none_when_empty` — AC 18 (empty case). `RemoteInterp::new()`,
empty `Nemesis`. `interpolate_at(nemesis, now)` returns `None`.

**[NEW]** `interpolate_snaps_to_single` — AC 18 (single case). Push one snapshot
at `(100.0, 200.0, 1.0)`. Call `interpolate_at` with the Instant recorded at push
time. Returns `(100.0, 200.0, 1.0)`.

**[NEW]** `interpolate_lerps_at_half_interval` — AC 19. Push snapshot A at
`(100.0, 100.0, 1.0)` at `t=0`. Push snapshot B at `(200.0, 100.0, -1.0)` at
`t=100ms`. Call `interpolate_at` with `now = t+50ms`. Returns approximately
`(150.0, 100.0, -1.0)` (position is mid-lerp; dir snaps to newest).

**[NEW]** `interpolate_clamped_to_newer` — AC 20. Same setup, but `now = t+200ms`
(elapsed > interval). Returns `(200.0, 100.0, -1.0)` (snaps to newer, no
overshoot).

**[NEW]** `interpolate_stale_guard` — AC 20 (stale case). Push snapshot at
`(100.0, 100.0, 1.0)` at `t=0`. Call `interpolate_at` with `now = t+300ms`
(>200ms stale threshold). Returns `(100.0, 100.0, 1.0)` (snaps to latest position).

**[NEW]** `push_evicts_oldest_on_third` — AC 17. Push three snapshots
A(0,0,1), B(10,10,1), C(20,20,1). Verify only B and C remain.

**[NEW]** `interpolate_stale_guard_boundary` — AC 39 (edge case). Push a snapshot
at t=0. Call `interpolate_at` with `now` at exactly 200ms → should still return
the snapped position (≤200ms: valid). Call with `now` at 201ms (just over
threshold) → should also return the snapped position (same behavior: with only
1 snapshot, the stale guard returns it regardless). Instead, use 2 snapshots:
push A at t=0, push B at t=50ms. Call at t=250ms (>200ms since B): returns B's
position (stale guard fires). Call at t=200ms (exactly 200ms since A): returns B
(in practice the interval is 50ms so elapsed > interval, clamped to 1.0 — still
returns B). The key assertion is that at >200ms after the newest snapshot,
interpolation still returns a position (does not return None) — it snaps rather
than interpolates.

---

### Group E — Server max_seq (AC 27–29)

These are in `server/src/world.rs` `#[cfg(test)] mod tests`.

**[NEW]** `max_seq_updated_on_apply_input` — AC 28. `spawn(0)`. Call
`apply_input(0, 0, 0, PlayerInput::default(), 5)`. Assert `max_seq()[&0] == 5`.
Call with `seq: 3` (older). Assert max_seq stays 5. Call with `seq: 10` (newer).
Assert max_seq becomes 10.

**[NEW]** `playerpos_broadcast_includes_last_seq` — AC 29 (code-read on the server
side, plus an assertion that `max_seq` starts at 0 for new players). `spawn(0)`;
assert `max_seq()[&0] == 0` (server reports 0 for players with no inputs).

Actual `PlayerPos` content is verified by the protocol round-trip test.

---

### Group F — Sticky edges (AC 22–24)

These are in `client/src/network/prediction.rs` `#[cfg(test)] mod tests`.
The `advance_sticky` helper (defined in `prediction.rs`) encapsulates the inline
sticky logic so it's testable. `update_network` calls it directly.

**[NEW]** `advance_sticky_press_produces_three_frames` — AC 22, 24.
```rust
/// Models the inline sticky logic in update_network:
///   1. If pressed: set sticky = 3.
///   2. flag = sticky > 0.
///   3. If sticky > 0: decrement.
/// Returns (should_send, new_sticky).
pub fn advance_sticky(sticky: u8, pressed: bool) -> (bool, u8) {
    let sticky = if pressed { 3 } else { sticky };
    let flag = sticky > 0;
    let next = if sticky > 0 { sticky - 1 } else { 0 };
    (flag, next)
}
```
Tests:
- `advance_sticky(0, true)` → (true, 2) — press sets to 3, sends true, decrements to 2.
- `advance_sticky(2, false)` → (true, 1) — still counting down.
- `advance_sticky(1, false)` → (true, 0) — last true frame.
- `advance_sticky(0, false)` → (false, 0) — sticky expired.
- `advance_sticky(1, true)` → (true, 2) — re-press during sticky window resets to 3 and re-counts.

This produces exactly 3 frames of `true`: press frame + 2 follow-up frames. ✓

In `update_network`, the inline call site (same as the function body):
```rust
let (jump, new_jump_sticky) = advance_sticky(world.jump_sticky, is_key_pressed(KeyCode::Space));
world.jump_sticky = new_jump_sticky;
// same for shoot with KeyCode::Z
```

---

### Group G — Unreliable transport (AC 26–27)

**[CODE-READ]** `grep "reliable_ordered" client/src/network/world.rs` → zero
matches for `PlayerInput` sending. The send call uses
`laminar::Packet::unreliable`.

**[CODE-READ]** `grep "unreliable" client/src/network/world.rs` → exactly one
match on the `PlayerInput` send line.

---

### Group H — Client self-simulation restored (AC 30–33)

**[CODE-READ]** `grep "player.update" client/src/world/game.rs` → exactly one
match, inside `World::update`, before `update_network(self)`.

**[CODE-READ]** `grep "is_key_pressed(KeyCode::Z)" client/src/world/game.rs` →
one match for bullet spawn, zero inside reconciliation code.

**[CODE-READ]** Enemy simulation, camera, `check_level_transitions`,
`check_player_death` code is structurally identical to 2b (verified by diff).

---

### Group I — Behavior parity (AC 34–37)

These are manual/visual tests, not unit tests:

**[MANUAL]** Two-client LAN test: holding Right moves the local player immediately
(no visible latency); remote player moves smoothly (no 20 Hz stepping).

**[MANUAL]** Single Space press produces a jump under normal LAN conditions.
Single Z press shoots.

**[MANUAL]** Under simulated packet loss (e.g., `tc qdisc` on loopback), the
player still moves and reconciles without visible teleportation (the re-simulation
absorbs small errors).

---

### Group J — Unchanged tests (must stay green)

**[UNCHANGED]** `game` crate: all 27 tests from Phase 2a (player input,
enemies_from_data, level collision, rect, maps, etc.).

**[UNCHANGED]** `server` crate: all 6 tests from Phase 2b (input accumulator
semantics, authoritative position drive, jump, out-of-range coords). These tests
need a `seq` in `apply_input` — the tester adds `seq: 0` to existing calls.

**[UNCHANGED]** `protocol` crate: the 6 non-`PlayerInput`/non-`PlayerPos`
round-trip tests.

**[UNCHANGED]** `client`: no existing unit tests (the client is a binary crate
with no `#[cfg(test)]` modules currently).

---

## Implementation checklist

Each box is a small, independently verifiable step in dependency order. The
`implementer` fills the `Explain` fields.

---

### Group 1 — Protocol + config (AC 1–5, D9)

#### Box 1 — Add `seq: u16` to `PlayerInput`, `last_seq: u16` to `PlayerPos`, and `CLIENT_HZ` to config

- [ ] done
- **Acceptance**: `protocol/src/lib.rs` — `PlayerInput` gains `seq: u16` as the
  last field; `PlayerPos` gains `last_seq: u16` as the last field. All other
  variants byte-identical shape. `game/src/config.rs` gains
  `pub const CLIENT_HZ: u32 = 60;`. `cargo build --workspace` compiles
  (though with "field not used" warnings from the server/client — those are
  resolved in later boxes).
- **Test**: `cargo test -p protocol` — all 9+ round-trip tests green (the 6
  existing + 3 new/modified). `grep -rn "CLIENT_HZ" game/src/config.rs` → one
  match.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 2 — Input buffer + sequence helpers (AC 6–10, Box 1 dep)

#### Box 2 — `InputRingBuffer` + `advance_sticky` (`client/src/network/prediction.rs`) + `is_newer_seq` (`game/src/lib.rs`)

- [ ] done
- **Acceptance**: `game/src/lib.rs` gains `pub fn is_newer_seq(a: u16, b: u16) -> bool`.
  `client/src/network/prediction.rs` exists with `InputEntry`,
  `InputRingBuffer` (new/push/drain_acknowledged/replay_order/is_empty/len),
  and `advance_sticky(sticky, pressed) -> (bool, u8)` as specified above.
  `client/src/network/mod.rs` declares `pub mod prediction;`.
  `cargo build -p metroidvania-rust` compiles. The server's `world.rs` also
  imports `is_newer_seq` from `game`.
- **Test**: `cargo test -p metroidvania-rust` — the prediction unit tests green
  (buffer push/fifo, drain acknowledged, drain empty, push-full evict, 5
  `is_newer_seq` cases, 6 `advance_sticky` cases — see Groups B and F below).
  `cargo test -p metroidvania-server` — server max_seq tests use
  `game::is_newer_seq`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 3 — Entity interpolation (AC 17–21, Box 2 dep)

#### Box 3 — `Nemesis` interpolation fields (`game/src/entities/nemesis.rs`)

- [ ] done
- **Acceptance**: `Nemesis` gains `interp_snapshots: [Option<(f32, f32, f32)>; 2]`,
  `interp_len: u8`, and `push_interp_snapshot(x, y, dir)`. `new()` initializes
  `interp_snapshots: [None, None]`, `interp_len: 0`.
  `cargo build -p game` compiles; existing game tests pass.
- **Test**: `cargo test -p game` (all 27 tests green; `Nemesis` is not directly
  unit-tested in `game`, but the struct compiles and existing tests are unchanged).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 4 — `RemoteInterp` module (`client/src/network/interpolation.rs`)

- [ ] done
- **Acceptance**: `client/src/network/interpolation.rs` exists with `RemoteInterp`
  (new/push/interpolate/interpolate_at) as specified. `client/src/network/mod.rs`
  declares `pub mod interpolation;`. `cargo build -p metroidvania-rust` compiles.
- **Test**: `cargo test -p metroidvania-rust` — all 6 interpolation unit tests
  green (see Group D above).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 4 — Server seq tracking (AC 27–29, Box 1 dep)

#### Box 5 — `max_seq` in `ServerWorld` + broadcast (`server/src/world.rs` + `server/src/main.rs`)

- [ ] done
- **Acceptance**: `ServerWorld` gains `max_seq: HashMap<u8, u16>` (initialized
  empty in `new`, inserted 0 in `spawn`, removed in `remove`).
  `apply_input` signature becomes `(id, x, y, input: PlayerInput, seq: u16)`
  and updates `max_seq` using `game::is_newer_seq`. New accessor
  `pub fn max_seq(&self) -> &HashMap<u8, u16>`.
  `server/src/main.rs`: `PlayerInput` match arm extracts `seq` and passes it to
  `apply_input`; `PlayerPos` broadcast includes `last_seq` from `max_seq`.
  `cargo build -p metroidvania-server` compiles. Using `game::is_newer_seq`
  avoids duplicating the wrapping comparison between client and server.
- **Test**: `cargo test -p metroidvania-server` — all 8 tests green (6 unchanged
  from 2b with `seq: 0` added to `apply_input` calls + 2 new `max_seq` tests
  from Group E). Manual: `grep -n "last_seq" server/src/main.rs` finds one
  match in the `PlayerPos` construction.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 5 — Client prediction loop (AC 11–16, 22–27, 30–33, Box 1–5 deps)

#### Box 6 — `World` struct fields + prediction in `update` (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: `World` struct gains `input_seq: u16`, `last_acked_seq: u16`,
  `input_buffer: InputRingBuffer`, `jump_sticky: u8`, `shoot_sticky: u8`,
  `other_interp: HashMap<u8, RemoteInterp>` as specified.
  `World::new` initializes them (`input_seq: u16::MAX`, `last_acked_seq: u16::MAX`,
  empty buffer, zero stickies, empty `other_interp`).
  `World::update` calls `self.player.update(dt, level, &local_input)` **before**
  `update_network(self)` — prediction runs first. Local `PlayerInput` uses raw
  `is_key_pressed(Space)` (not sticky) for jump, `shoot: false`.
  `cargo build -p metroidvania-rust` compiles.
- **Test**: code-read: `grep -n "player.update" client/src/world/game.rs` → one
  match before `update_network`. `grep -n "input_seq\|last_acked_seq\|input_buffer\|jump_sticky\|shoot_sticky\|other_interp" client/src/world/game.rs` → each
  appears in struct definition + init.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 7 — Sticky edges + unreliable send + buffer store (`client/src/network/world.rs` — `update_network`)

- [ ] done
- **Acceptance**: `update_network` implements the sticky edge logic using
  `advance_sticky` from `prediction.rs`. Each frame: `input_seq` is incremented
  (wrapping), an `InputEntry` (without shoot) is pushed into `input_buffer`,
  and `GamePacket::PlayerInput` is sent via `Packet::unreliable` (not
  `reliable_ordered`). The `PlayerInput` on the wire includes `seq`.
  `cargo build -p metroidvania-rust` compiles.
- **Test**: code-read: `grep "reliable_ordered" client/src/network/world.rs` →
  zero matches for the `PlayerInput` send; `grep "unreliable" client/src/network/world.rs` →
  one match at the `PlayerInput` send. `grep "advance_sticky\|input_seq\|input_buffer.push" client/src/network/world.rs` → matches. Unit tests for
  `advance_sticky` already green from Box 2.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 8 — Reconciliation + interp push in `handle_packet` (`client/src/network/world.rs`)

- [ ] done
- **Acceptance**: `handle_packet` `PlayerPos` match arm implements the full
  reconciliation flow for own player (stale check with `is_newer_seq`, update
  `last_acked_seq`, `drain_acknowledged`, rewind pos/dir, replay unacknowledged
  inputs with `shoot: false` at `1.0/CLIENT_HZ` dt) and the interpolation push
  for remote players (create `Nemesis`+`RemoteInterp` on first sight, push
  snapshot). `JoinResponse` resets ack state. `Leave` removes from `other_interp`
  as well. `cargo build -p metroidvania-rust` compiles.
- **Test**: `cargo test -p metroidvania-rust` — all reconciliation replay tests
  green (see Group C above, from Box 2). Code-read: `grep -n "drain_acknowledged\|is_newer_seq\|replay_dt\|\"shoot: false\"\|interp\.push" client/src/network/world.rs`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 6 — Render (AC 21, Box 3–4 deps)

#### Box 9 — Interpolated Nemesis draw (`client/src/render.rs` + `client/src/world/game.rs` draw)

- [ ] done
- **Acceptance**: `client/src/render.rs` gains `pub fn draw_nemesis_at(nemesis, x, y, dir)`.
  `World::draw` remote-player loop iterates `other_players` + `other_interp`
  simultaneously, calling `draw_nemesis_at` when `interpolate` returns `Some`,
  falling back to `nemesis.draw()` when `None`. `cargo build -p metroidvania-rust`
  compiles.
- **Test**: Interpolation unit tests already green from Box 4. Manual visual:
  two clients, remote player moves smoothly (no 20 Hz stepping).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 7 — Behavior parity (AC 34–37)

#### Box 10 — Manual smoke test + local feel

- [ ] done
- **Acceptance**: Two-client `cargo run` (server + 2 clients, LAN/localhost).
  - Holding `Right` moves the local player immediately (no visible latency).
  - Remote player moves smoothly (continuous, no stepping).
  - Single `Space` press → jump; single `Z` press → shoot.
  - Reconciliation is invisible under normal conditions (no position snapping).
  - `cargo run -- server` does not crash; clients connect, move, and disconnect
    cleanly.
- **Test**: Manual. No unit test covers the full loop (rendering/network are
  excluded by project rule).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Group 8 — Quality gates (AC 38–42)

#### Box 11 — Workspace build/test/clippy/fmt

- [ ] done
- **Acceptance**:
  - `cargo build --workspace` succeeds (all four crates: game, protocol, server, client).
  - `cargo test --workspace` passes: all existing game (27) + protocol (9+)
    + server (8) + new client prediction/interpolation buffer tests green.
  - `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
  - `cargo fmt --check --all` reports no diffs.
  - `grep -rn "unwrap()\|\.expect(" client/src/network/prediction.rs` → zero
    matches (no panics in prediction code — AC 42).
  - `grep -rn "unwrap()\|\.expect(" client/src/network/interpolation.rs` → zero
    matches.
  - `protocol/Cargo.toml` declares no dependency on `game` or `glam` (AC 4).
- **Test**: `cargo build --workspace`; `cargo test --workspace`;
  `cargo clippy --all-targets --workspace -- -D warnings`; `cargo fmt --check --all`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

## Open questions

- **`is_newer_seq` location — RESOLVED.** Placed in `game/src/lib.rs` as
  `pub fn is_newer_seq(a: u16, b: u16) -> bool`. It is a math utility used by
  both the client (`prediction.rs`) and the server (`world.rs`); the `game`
  crate is the natural shared dependency. Both crates import it as
  `use game::is_newer_seq;`.

- **`advance_sticky` correctness — RESOLVED.** The function (in
  `client/src/network/prediction.rs`) now matches the inline logic:
  `let sticky = if pressed { 3 } else { sticky };` then `let flag = sticky > 0;`
  then `let next = if sticky > 0 { sticky - 1 } else { 0 };`. This produces
  exactly 3 frames of `true`. `update_network` calls `advance_sticky` directly;
  the function is unit-tested in `prediction.rs`.

- **Replay dt precision.** `1.0 / CLIENT_HZ as f32` = `1.0 / 60.0` ≈ `0.016666668`.
  The original frames used `get_frame_time()` which is ~16.667ms at 60 Hz but
  has micro-variation. The fixed replay dt means each replayed input produces a
  slightly different position than the original prediction — this is the
  "sub-pixel drift" acknowledged in the spec (D6). At 20 Hz tick rate, the
  server corrects every 50 ms, well before the drift accumulates beyond a
  pixel. Accept and move on; exact dt matching is a later optimization.

- **`PlayerPos` does not carry `vel` or `grounded`.** The spec explicitly accepts
  this trade-off: the client keeps its predicted `vel`/`grounded` across the
  rewind, and replaying inputs from the corrected position recomputes them via
  the same `Player::update` physics. Adding `vel`/`grounded` to `PlayerPos`
  would add 8 bytes per player per tick (unreliable broadcast) — not worth it
  for a small reconvergence speedup. Flag for a later phase if visual snapping
  becomes an issue under high packet loss.

- **Server lib/bin split.** Deferred from Phase 2b. The server still has no
  integration tests and all pure logic tests are inline. Revisit when the server
  grows more logic or alongside the client lib/bin split (already noted in
  `docs/progress.md`).

- **`Action` and `Leave` packet channel.** The spec (D2) switches only
  `PlayerInput` to unreliable. `Action{shoot}` and `Leave` stay on
  reliable-ordered (unchanged from 2b). This is correct: shoot notifications
  and leave events must not be silently dropped.