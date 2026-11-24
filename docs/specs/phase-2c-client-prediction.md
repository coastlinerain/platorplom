# Phase 2c — Client prediction + reconciliation

- Status: draft
- Date: 2026-11-24
- Branch: `feat/client-prediction`

## Summary

Phase 2b made the server authoritative for player movement, at the cost of visible
input latency: a keypress travels client → server → back before movement appears
(~1 RTT). Phase 2c eliminates that latency by adding **client-side prediction**
and **reconciliation**, the standard netcode technique for authoritative-server
games.

The client resumes simulating its own player locally every frame (exactly as it
did before 2b), showing the predicted position immediately. Each outgoing input
carries a monotonically-increasing **sequence number**. The server echoes the
highest processed `seq` back in every `PlayerPos`. When an authoritative position
arrives, the client discards acknowledged inputs from its history and
**re-simulates** any remaining unacknowledged inputs (movement only — no bullet
spawn) from the server-corrected state. The result is zero-latency local feel with
eventual server correction.

Remote players are smoothed via **entity interpolation** between the last two
`PlayerPos` snapshots, eliminating the 20 Hz stepping artifact.

**Scope boundary** (unchanged from 2b): only *movement* is authoritative. Enemies,
bullets, room transitions, and death handling remain client-side. Own bullets
continue to be spawned once in `World::update` (not re-spawned during
reconciliation).

## Decisions

### D1. Input sequence number: `seq: u16` on `PlayerInput`

**Decision:** add a `seq: u16` field to `GamePacket::PlayerInput`. The client
maintains a monotonically-increasing counter, incremented per *sent* input
(wrapping at `u16::MAX` is fine — ~18 minutes at 60 Hz). The server records the
highest `seq` received per player and echoes it as `last_seq` in `PlayerPos`.

**Rejected alternative:** a separate ack-only packet. Adds network traffic and
desynchronization risk for no benefit; piggybacking on the already-every-tick
`PlayerPos` is free.

### D2. Server echo: `last_seq: u16` on `PlayerPos`

**Decision:** add a `last_seq: u16` field to `GamePacket::PlayerPos`. Every tick,
after simulating, the server reads the player's `max_received_seq` and writes it
into the outgoing `PlayerPos` for that player. The client uses this to know which
inputs the server has processed.

No new packet variant is created — just an extra field on the existing one.

### D3. Reconciliation: full rewind-and-replay

**Decision:** on receiving an authoritative `PlayerPos` with `last_seq`:

1. **Stale guard:** if `seq.wrapping_sub(last_acked_seq) == 0` or the `seq` is
   behind (using wrapping comparison with a 32767 threshold), ignore.
2. **Discard acknowledged:** remove all entries from the input buffer whose
   `seq` ≤ `last_seq`.
3. **Rewind:** overwrite the local player's `pos` and `dir` from the
   authoritative state. `PlayerPos` does not carry `vel` or `grounded` (adding
   8 bytes per player per tick to the unreliable broadcast is not justified).
   Replaying unacknowledged inputs from the corrected position recomputes `vel`
   and `grounded` naturally via `Player::update`'s physics.
   from the authoritative state.
4. **Replay:** iterate the remaining unacknowledged inputs in order and call
   `Player::update(replay_dt, level, &replay_input)` with `shoot: false` (D7)
   for each. Use a fixed `replay_dt = 1.0 / 60.0`.
5. **Continue:** the player now runs forward from the corrected position with
   the re-simulated state.

**Rejected alternative:** simple snap-back with no replay (position-only
correction). This causes visible teleporting when the error is large (packet
loss). The re-simulation cost is trivial (~64 iterations max, <1ms).

**Rejected alternative:** error smoothing (lerp toward authoritative position over
several frames). This is visually gentler but adds persistent drift; rewind +
replay is the gold standard for FPS/platformer netcode and is what the owner wants
to learn.

### D4. Input buffer: ring buffer of 64 entries

**Decision:** the client stores a fixed-capacity ring buffer of `(seq: u16,
input: PlayerInput)` entries. 64 entries at 60 Hz sending covers ~1 second of
inputs — ample headroom for a few hundred ms of worst-case burst loss. When the
buffer is full, the oldest entry is evicted (the server would have acknowledged a
much older seq by then, so this is safe).

**Wrapping sequence comparison:** `seq_a` is newer than `seq_b` when
`seq_a.wrapping_sub(seq_b) > 0 && seq_a.wrapping_sub(seq_b) < 32768`. This
handles the u16 wraparound correctly.

### D5. Entity interpolation (in scope)

**Decision:** each remote `Nemesis` stores the two most recent `PlayerPos`
snapshots (each with a local arrival timestamp via `Instant::now()`). Every frame
the client:

1. Computes `elapsed = now - older_snapshot.timestamp`.
2. Computes `interval = newer_snapshot.timestamp - older_snapshot.timestamp`
   (clamped to `tick_interval` minimum to avoid divide-by-zero).
3. Lerps position: `pos = older + (newer - older) * (elapsed / interval)`.
4. Snaps `dir` to the newer value.

If only one snapshot exists: snap to it (no extrapolation). If no snapshot
arrives for > 200ms: snap to the latest (stale guard against disconnected
players).

This is purely visual — the interpolated position is computed at draw time and
does not mutate `Nemesis.pos`.

### D6. Input transport: switch to unreliable

**Decision:** the client sends `PlayerInput` on the **unreliable** channel
(`laminar::Packet::unreliable`). The sequence number + re-simulation handle
dropped detection and correction; reliable-ordered would cause head-of-line
blocking that defeats the purpose of prediction.

**Edge-flag redundancy:** because individual `jump`/`shoot` edges can now be
silently dropped, the client holds each edge flag `true` for **3 consecutive
sent frames** after the initial press (a "sticky edge" window). This gives the
server 3 chances to receive it; the server's OR-latch deduplicates naturally. The
sticky edge is implemented as a small counter on the client: on
`is_key_pressed`, set `sticky = 3`; each frame, send `jump: true` (or `shoot:
true`) while `sticky > 0`, decrementing each send. This is an existing technique
(Quake 3, Source engine) and keeps the unreliable channel viable for edge actions.

The `replay_dt` during reconciliation is fixed at `1.0 / 60.0`, matching the
approximate client frame rate. The integration error from replaying with a
slightly different dt than the original frame is a sub-pixel drift that the
next server correction absorbs.

### D7. Movement-only replay

**Decision:** during reconciliation re-simulation, `shoot` is forced `false` in
every replayed input. Own-bullet spawning remains exclusively in
`World::update` (the one-time `is_key_pressed(KeyCode::Z)` check). This avoids
double-spawning or phantom bullets during corrections.

### D8. Client-side state: prediction fields

**Decision:** the client `World` (or a new `ClientPrediction` struct) gains:

- `input_seq: u16` — next sequence number to send.
- `last_acked_seq: u16` — highest `last_seq` received from the server.
- `input_buffer: InputRingBuffer` — ring buffer of 64 unacknowledged inputs.

These are purely client-side prediction bookkeeping fields and are never
serialized or sent to the server.

### D9. Separate tick-rate constants for server and client

**Decision:** the server uses `game::config::TICK_HZ` (20) for its fixed-step tick
loop. The client adds a new constant `game::config::CLIENT_HZ = 60` for prediction
replay. Reusing `TICK_HZ` for the client would mean replaying inputs with a 50 ms
dt instead of the ~16.7 ms dt used during live prediction — producing 3× the
integration error and causing visible position jitter on reconciliation. The two
constants serve different purposes: `TICK_HZ` governs the server's simulation
cadence; `CLIENT_HZ` governs the client's frame-rate-relative dt.

## Goals / non-goals

### Goals

1. **Local prediction restored:** the client calls `Player::update` locally for
   its own player every frame, showing immediate movement response.
2. **Input sequencing:** each `PlayerInput` carries a `seq`; the server echoes
   `last_seq` in `PlayerPos`.
3. **Reconciliation:** on authoritative `PlayerPos`, rewind to server state and
   re-simulate unacknowledged inputs (movement only).
4. **Unreliable input transport:** `PlayerInput` switches from reliable-ordered
   to unreliable, with sticky edge flags for jump/shoot.
5. **Entity interpolation:** remote `Nemesis` players are drawn at an
   interpolated position between their last two snapshots, eliminating the 20 Hz
   stepping artifact.
6. **Behavior parity:** local movement feel is identical to pre-2b (immediate
   response); remote players appear smooth.

### Non-goals

- **No server-side input validation beyond existing `addr → id` check.** The
  server trusts the `seq` value (for echo) but does not use it for anti-cheat.
- **No dead reckoning / extrapolation** for remote players beyond interpolation.
- **No authoritative bullets, enemies, room transitions, or death** — scope
  boundary unchanged from 2b.
- **No tick-rate change** (stays at 20 Hz).
- **No snapshot delta compression.** `PlayerPos` remains a full state packet.

## Behavior

### Client: sending inputs

Each frame, once `world.id` is `Some`:

1. Increment `input_seq` (wrapping).
2. Build a `PlayerInput` from the keyboard (same mapping as today):
   - `move_left` / `move_right`: held (`is_key_down`).
   - `jump`: `true` if the sticky edge counter > 0, else `false`.
   - `shoot`: `true` if the sticky edge counter > 0, else `false`.
3. Track sticky edge counters:
   - On `is_key_pressed(KeyCode::Space)`, set `jump_sticky = 3`.
   - On `is_key_pressed(KeyCode::Z)`, set `shoot_sticky = 3`.
   - Decrement each counter after sending.
4. Push `(input_seq, input)` into the ring buffer.
5. Send `GamePacket::PlayerInput { id, level_x, level_y, move_left, move_right, jump, shoot, seq: input_seq }` on the **unreliable** channel.

Edge case: if the ring buffer is full (64 entries), evict the oldest entry before
pushing. This only happens under catastrophic packet loss (>1 second of silence);
the evicted input is permanently lost (the server would have acknowledged it by
then under normal conditions).

### Client: prediction (local self-simulation)

Restored in `World::update`, before `update_network`:

1. Build the local `PlayerInput` from keys (same sampling, but no sticky edge —
   use raw `is_key_pressed` for local simulation so the player responds on the
   exact frame).
2. Call `self.player.update(dt, level, &input)`.
3. Show the predicted position immediately.

The order is: **predict → send → receive**. Prediction runs first so the frame
shows an up-to-date position; then `update_network` drains received packets
(which may correct the position via reconciliation); then the corrected state is
visible on the next frame's draw.

### Client: reconciliation (`handle_packet`)

When `PlayerPos { id, .., last_seq }` arrives and `id == world.id`:

1. **Stale check:** if `last_seq` is not newer than `last_acked_seq` (using
   wrapping comparison with threshold 32767), ignore this packet. The server may
   send duplicate or reordered `PlayerPos` on unreliable; only the latest ack
   matters.
2. Update `last_acked_seq = last_seq`.
3. **Discard acknowledged inputs:** remove all entries from the ring buffer
   whose `seq ≤ last_seq` (wrapping-aware).
4. **Save authoritative snapshot:** record `(pos, dir)` from the server's
   `PlayerPos`. `vel` and `grounded` are not in the packet; replay recomputes them.
5. **Rewind:** overwrite `player.pos` and `player.dir` from the snapshot.
6. **Replay:** for each remaining entry `(seq, input)` in the ring buffer (in
   oldest-to-newest order):
   - Build a replay input: `move_left`, `move_right`, `jump` from the stored
     input; `shoot: false`.
   - Call `player.update(replay_dt, level, &replay_input)` with
     `replay_dt = 1.0 / 60.0`.

Edge case — room mismatch during replay: when the `PlayerPos` carries a
`level_x`/`level_y` different from the client's `current_coords`, the replay
uses the **client's current room's collision level** (`levels_matrix[current_coords]`),
not the server's reported room. This is because room transitions are
client-authoritative (scope boundary) and the collision geometry for the
player's actual room is what matters for re-simulation. The server's
`level_x`/`level_y` is recorded but not acted upon for collision during replay.

Edge case — first `PlayerPos` after join: `last_acked_seq` starts at 0
(`u16::MAX` for wrapping safety). The first `PlayerPos` sets `last_acked_seq` and
clears any inputs the client sent before receiving the first authoritative state.
This is a one-time snap to server state.

### Client: entity interpolation (`handle_packet`)

When `PlayerPos { id, x, y, dir, level_x, level_y, .. }` arrives and `id != world.id`:

1. Look up or create the `Nemesis` for `id`.
2. Push a `Snapshot { pos: (x, y), dir, level_x, level_y, timestamp: Instant::now() }`
   into a per-Nemesis ring buffer of 2 entries (newest wins, oldest evicted).
3. At draw time (`Nemesis::draw` or a new `draw_interpolated` method):
   - If 2 snapshots exist: lerp position between them based on elapsed time
     since the older timestamp (clamped to [0, 1]).
   - If 1 snapshot: draw at that exact position (snap).
   - If the newest snapshot's timestamp is >200ms old: draw at the latest
     position (stale guard — the player may have disconnected).

Edge case — no snapshots yet (new player): do not draw. The `Nemesis` is created
on the first `PlayerPos` arrival and will be drawn starting from the next frame.

Edge case — room change: the interpolation crosses room boundaries visually. When
the remote player changes rooms (different `level_x`/`level_y`), the interpolation
uses the newer snapshot's position regardless of room, producing a smooth
transition. The `coords` field on `Nemesis` is updated to the newer snapshot's
coords so the draw/no-draw gating by room still works for the post-transition
state.

### Server: sequence tracking

The server gains minimal new state in `ServerWorld`:

- A `HashMap<u8, u16>` mapping player id → highest received `seq`.

On `apply_input`: update `max_seq[id] = max(max_seq[id], input.seq)` (wrapping
comparison). This is a one-line addition.

On each tick's `PlayerPos` broadcast: include `last_seq: max_seq[id]` in the
outgoing packet (0 for players who have not sent any inputs yet).

The server does **not** use `seq` for ordering, deduplication, or anti-cheat.
The existing input accumulator (held-overwrite, edge-OR-latch) works unchanged.
The `seq` is purely an echo field.

Edge case — new player: `max_seq` starts at 0. The first `PlayerPos` broadcasts
`last_seq: 0`, which the client's wrapping comparison handles correctly
(`0.wrapping_sub(u16::MAX) > 32768` → not newer → the client treats it as the
initial state regardless).

## Data model

### `protocol/src/lib.rs`

Two existing variants gain fields.

```rust
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    // ... existing variants unchanged: JoinRequest, JoinResponse, Action, Leave, Hit

    PlayerPos {
        id: u8,
        // --- existing fields, unchanged ---
        x: f32,
        y: f32,
        dir: f32,
        level_x: u8,
        level_y: u8,
        // --- new field ---
        last_seq: u16,     // highest input seq processed by the server for this player
    },

    PlayerInput {
        id: u8,
        level_x: u8,
        level_y: u8,
        move_left: bool,
        move_right: bool,
        jump: bool,
        shoot: bool,
        // --- new field ---
        seq: u16,          // monotonically-increasing input sequence number
    },
}
```

### `server/src/world.rs` — `ServerWorld`

Add one field:

```rust
pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords:  HashMap<u8, (u8, u8)>,
    levels:  Vec<Vec<Level>>,
    // --- new ---
    max_seq: HashMap<u8, u16>,  // highest seq received per player
}
```

`apply_input` updates `max_seq`:

```rust
pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16) {
    // ... existing accumulator logic unchanged ...

    // Update highest seen sequence number.
    let current = self.max_seq.entry(id).or_insert(0);
    if seq.wrapping_sub(*current) > 0 && seq.wrapping_sub(*current) < 32768 {
        *current = seq;
    }
}
```

`update` reads `max_seq` during broadcast construction (in `main.rs`):

```rust
let last_seq = world.max_seq().get(&id).copied().unwrap_or(0);
```

Expose a read-only accessor:

```rust
pub fn max_seq(&self) -> &HashMap<u8, u16> { &self.max_seq }
```

### `client/src/world/game.rs` — `World`

Add prediction fields:

```rust
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
    pub jump_sticky: u8,                         // remaining frames to send jump: true (D6)
    pub shoot_sticky: u8,                        // remaining frames to send shoot: true (D6)
}
```

### `client/src/network/prediction.rs` (new file)

```rust
/// Entry stored in the input ring buffer.
#[derive(Debug, Clone)]
pub struct InputEntry {
    pub seq: u16,
    pub move_left: bool,
    pub move_right: bool,
    pub jump: bool,
    // shoot is intentionally NOT stored — it is not replayed.
}

/// Fixed-capacity ring buffer for unacknowledged inputs.
pub struct InputRingBuffer {
    entries: [Option<InputEntry>; 64],
    head: usize,  // insertion index
    len: usize,   // number of valid entries
}

impl InputRingBuffer {
    pub fn new() -> Self;
    pub fn push(&mut self, entry: InputEntry);   // evicts oldest if full
    pub fn drain_acknowledged(&mut self, last_seq: u16);  // remove seq ≤ last_seq
    pub fn replay_order(&self) -> impl Iterator<Item = &InputEntry>;  // oldest → newest
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
}

Interpolation state is split between `Nemesis` (positions, in `game`) and
`RemoteInterp` (timestamps, in `client`). See the Nemesis section below and the
new `client/src/network/interpolation.rs` for details.

### `client/src/world/game.rs` — `World::update` (modified)

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
            shoot: false,                            // bullet spawn is separate
        };
        self.player.update(dt, level, &local_input);

        // Own-bullet spawn (unchanged, still once per Z press).
        if is_key_pressed(KeyCode::Z) {
            self.bullets.push(Bullet::new(/* ... */));
        }

        // Own-bullet update and collision (unchanged).
        // Enemy update (unchanged).
        // Camera update (unchanged).
    }

    update_network(self);  // sends inputs, receives + reconciles

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
        // Sticky edge handling.
        if is_key_pressed(KeyCode::Space) { world.jump_sticky = 3; }
        if is_key_pressed(KeyCode::Z)    { world.shoot_sticky = 3; }

        let jump = world.jump_sticky > 0;
        let shoot = world.shoot_sticky > 0;
        if world.jump_sticky > 0 { world.jump_sticky -= 1; }
        if world.shoot_sticky > 0 { world.shoot_sticky -= 1; }

        world.input_seq = world.input_seq.wrapping_add(1);
        let seq = world.input_seq;

        let input_msg = GamePacket::PlayerInput {
            id,
            level_x: world.current_coords.0 as u8,
            level_y: world.current_coords.1 as u8,
            move_left: is_key_down(KeyCode::Left),
            move_right: is_key_down(KeyCode::Right),
            jump,
            shoot,
            seq,
        };

        // Store for reconciliation replay (shoot is NOT stored).
        world.input_buffer.push(InputEntry {
            seq,
            move_left: input_msg.move_left,
            move_right: input_msg.move_right,
            jump: input_msg.jump,
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
        GamePacket::PlayerPos {
            id, x, y, dir, level_x, level_y, last_seq,
        } => {
            if Some(id) == world.id {
                // --- RECONCILIATION for own player ---
                // Stale check: only process if last_seq is newer.
                if is_newer_seq(last_seq, world.last_acked_seq) {
                    world.last_acked_seq = last_seq;

                    // Discard acknowledged inputs.
                    world.input_buffer.drain_acknowledged(last_seq);

                    // Rewind to server state.
                    world.player.pos = vec2(x, y);
                    world.player.dir = dir;
                    // (vel and grounded are set by replay; if buffer empty,
                    //  they are implicitly server-authoritative. For safety,
                    //  we zero vel and let replay or next frame's gravity handle it.
                    //  Actually, we can't know vel from PlayerPos, so we keep
                    //  the predicted vel and let the replay correct it.)
                    //
                    // IMPORTANT: PlayerPos does not carry vel/grounded.
                    // The re-simulation replays all unacknowledged inputs
                    // starting from (x, y, dir) with the current predicted
                    // vel/grounded. This means re-simulation starts from the
                    // server position but with client velocity. The accumulated
                    // physics over the replayed inputs converges to the correct
                    // value because the server's position IS the ground truth
                    // at that tick, and replaying unacknowledged inputs from
                    // that point produces a consistent prediction.

                    // Replay unacknowledged inputs.
                    let level = &world.levels_matrix[world.current_coords.1]
                                                  [world.current_coords.0];
                    let replay_dt = 1.0 / 60.0;
                    for entry in world.input_buffer.replay_order() {
                        let replay_input = game::entities::player::PlayerInput {
                            move_left: entry.move_left,
                            move_right: entry.move_right,
                            jump: entry.jump,
                            shoot: false,  // D7: never re-spawn bullets
                        };
                        world.player.update(replay_dt, level, &replay_input);
                    }
                }
            } else {
                // --- Remote player: update interpolation buffer ---
                let snap = SnapshotData { x, y, dir, level_x, level_y };
                if let Some(nemesis) = world.other_players.get_mut(&id) {
                    nemesis.interp_buffer.push(snap);
                    nemesis.coords = (level_x as usize, level_y as usize);
                } else {
                    let mut new_nemesis = Nemesis::new(id, vec2(x, y));
                    new_nemesis.last_dir = dir;
                    new_nemesis.coords = (level_x as usize, level_y as usize);
                    new_nemesis.interp_buffer.push(snap);
                    world.other_players.insert(id, new_nemesis);
                }
            }
        }
        // ... other variants unchanged ...
    }
}
```

**Important trade-off (explicit):** `PlayerPos` does not carry `vel` or
`grounded`. During reconciliation, the client resets `pos` and `dir` from the
server, keeps its own predicted `vel` and `grounded`, then replays unacknowledged
inputs. This is a pragmatic choice: adding `vel`/`grounded` to `PlayerPos` would
bloat the unreliable broadcast (8 extra bytes per player per tick). The
replay-integrated velocity converges to the correct value because `Player::update`
recomputes velocity from gravity + jump force and recomputes `grounded` from the
current position + collision level each step. The position — which IS corrected —
drives these computations.

### `game/src/entities/nemesis.rs` — updated

`Nemesis` stays in the `game` crate (shared). Interpolation data that needs
`std::time::Instant` is kept in the client.

```rust
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,              // raw latest position (used for bullet spawn compat)
    pub w: f32,
    pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
    // --- new ---
    /// Interpolation snapshots (positions and dirs, stored oldest→newest).
    /// Timestamps and Instant-based logic live in the client, not here.
    pub interp_snapshots: [Option<(f32, f32, f32)>; 2],  // [(x, y, dir), …]
    pub interp_len: u8,         // number of valid snapshots (0, 1, or 2)
}
```

**Why `[Option<(f32,f32,f32)>; 2]` instead of an `InterpBuffer` struct:**
`InterpBuffer` needs `Instant` timestamps (from `std::time`), but the `game`
crate should remain timing-agnostic. The raw position data lives on `Nemesis`
for simplicity; the client-side `interpolation.rs` module owns the timestamps
(see below) and calls into `Nemesis` to read/write snapshots.

### `client/src/network/interpolation.rs` (new file)

Keeps per-remote-player timestamps alongside a reference to the player's
`Nemesis` (or co-owns them via a small wrapper):

```rust
use std::time::Instant;

/// Per-remote-player interpolation state (client-only, not in `game`).
pub struct RemoteInterp {
    /// When each snapshot arrived (parallels nemesis.interp_snapshots).
    pub timestamps: [Option<Instant>; 2],
}

impl RemoteInterp {
    pub fn new() -> Self;

    /// Push a new snapshot into the Nemesis and record its arrival time.
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32);

    /// Compute interpolated (x, y, dir) or None if not enough data.
    pub fn interpolate(&self, nemesis: &Nemesis) -> Option<(f32, f32, f32)>;
}
```

The `World` struct holds `other_interp: HashMap<u8, RemoteInterp>` alongside
`other_players: HashMap<u8, Nemesis>`. At draw time, for each remote player in
the same room, compute `interpolated = interp.interpolate(nemesis)` and draw at
that position/dir, falling back to `nemesis.pos`/`nemesis.last_dir` if
`interpolated` is `None`.

`client/src/network/mod.rs` gains `pub mod interpolation;`.

## Network

### Channel changes

| Packet | Direction | Channel (before) | Channel (after) |
|---|---|---|---|
| `PlayerInput` | client→server | `reliable_ordered` | **`unreliable`** |
| `PlayerPos` | server→client | `unreliable` | `unreliable` (unchanged) |
| `Action{shoot}` | server→client | `reliable_ordered` | `reliable_ordered` (unchanged) |
| `JoinRequest` / `JoinResponse` | both | `reliable_ordered` | `reliable_ordered` (unchanged) |
| `Leave` | server→client | `reliable_ordered` | `reliable_ordered` (unchanged) |
| `Hit` | client→server | `reliable_ordered` | `reliable_ordered` (unchanged) |

### Modified packets

- **`PlayerInput`**: adds `seq: u16` field.
- **`PlayerPos`**: adds `last_seq: u16` field.

Total size increase: 2 bytes per `PlayerInput` (~13→15 bytes at rest), 2 bytes
per `PlayerPos` (~17→19 bytes at rest). Negligible for a handful of players.

### New type (not a packet)

Interpolation data is stored as raw `(f32, f32, f32)` tuples in
`Nemesis.interp_snapshots` within the `game` crate — no new `protocol` type
needed.

## Acceptance criteria

### Protocol

1. `GamePacket::PlayerInput` gains `seq: u16`; existing fields unchanged.
2. `GamePacket::PlayerPos` gains `last_seq: u16`; existing fields unchanged.
3. A `protocol` round-trip test serializes/deserializes `PlayerInput` with a
   non-zero `seq` and `PlayerPos` with a non-zero `last_seq`; all fields
   round-trip correctly.
4. `protocol/Cargo.toml` declares no dependency on `game` or `glam`.
5. `PlayerInput` round-trip test covers `seq` wraparound: `seq = u16::MAX`
   serializes and deserializes to `u16::MAX`.

### Input buffer (`InputRingBuffer`)

6. `push` inserts an entry; `replay_order` yields entries in FIFO order
   (oldest → newest).
7. `drain_acknowledged(last_seq)` removes all entries where
   `seq.wrapping_sub(last_seq) == 0` or the seq is not newer (wrapping-aware).
   Entries with `seq` newer than `last_seq` are retained.
8. `drain_acknowledged` handles the empty-buffer case (no-op).
9. `push` with a full buffer (64 entries) evicts the oldest entry, increasing
   capacity for the new one; `len` stays at 64.
10. Sequence comparison helper `is_newer_seq(a, b)` returns `true` when `a` is
    ahead of `b` by 1..32767 (wrapping), `false` otherwise (including equal and
    behind).

### Reconciliation

11. When a `PlayerPos` arrives with `last_seq` newer than `last_acked_seq`,
    the client updates `last_acked_seq` and drains acknowledged inputs.
12. After drain, unacknowledged inputs are replayed in order with
    `Player::update(replay_dt, level, &input)` where `replay_input.shoot == false`.
13. A unit test constructs a player at position A, pushes 3 unacknowledged
    `move_right` inputs, replays them via `Player::update` at `1.0/60.0`, and
    verifies the player moved right from A.
14. A unit test verifies that a conflicting input (server says player is at X,
    unacknowledged inputs include a `move_left`) produces a corrected position
    that reflects the replayed `move_left` from server position X.
15. Stale `PlayerPos` (same `last_seq` as `last_acked_seq`, or behind) is
    silently ignored; no rewind or replay occurs.
16. The first `PlayerPos` after join sets `last_acked_seq` from 0 or `u16::MAX`
    and clears any pre-ack inputs from the buffer (snap to server state).

### Entity interpolation

17. `RemoteInterp::push` stores the snapshot data in the `Nemesis` and records
    the arrival timestamp; a third push evicts the oldest snapshot from both
    the `Nemesis.interp_snapshots` and the `RemoteInterp.timestamps`.
18. `RemoteInterp::interpolate` returns `None` when empty; returns `Some(pos)`
    equal to the single snapshot's position when only one exists; returns a lerp
    between two positions when both exist.
19. A unit test verifies that with two snapshots at positions (100,100) and
    (200,100), an elapsed time of 50% of the interval yields an interpolated
    position of approximately (150,100).
20. The interpolation lerp factor is clamped to `[0.0, 1.0]` so that
    elapsed > interval snaps to the newer position (no overshoot extrapolation).
21. At draw time, remote players use the interpolated position/dir returned by
    `RemoteInterp::interpolate`, not the raw `Nemesis.pos`/`Nemesis.last_dir`.
    If `interpolate` returns `None`, fall back to the raw position.

### Sticky edge flags

22. On `is_key_pressed(KeyCode::Space)`, the jump sticky counter is set to 3.
    It decrements each frame the input is sent; while >0, `PlayerInput.jump` is
    `true` in the sent packet.
23. On `is_key_pressed(KeyCode::Z)`, the shoot sticky counter is set to 3.
    Decrement and send behavior is the same as jump.
24. A unit test verifies that a single key press produces exactly 3 frames of
    `jump: true` in the input buffer entries (no more, no less).
25. During reconciliation replay, `jump` from the stored input is replayed
    but `shoot` is forced `false` regardless of the stored value.

### Unreliable transport

26. `grep "reliable_ordered" client/src/network/world.rs` finds zero matches
    for `PlayerInput` sending (the send call uses `Packet::unreliable`).
27. The server's `apply_input` signature gains a `seq: u16` parameter; the
    server updates `max_seq` on each call.
28. A server-side unit test verifies that `max_seq` is updated on
    `apply_input` with a wrapping-aware comparison.
29. The server includes `last_seq` from `max_seq` in every `PlayerPos`
    broadcast; players with no inputs yet get `last_seq: 0`.

### Client self-simulation restored

30. `World::update` calls `self.player.update(dt, level, &input)` for the
    local player with the raw keyboard input (not sticky edges).
31. The local player's position changes immediately on keypress (before
    `update_network`), visible on the same frame.
32. Own bullets still spawn via `is_key_pressed(KeyCode::Z)` in
    `World::update` (exactly once per press), not re-spawned during replay.
33. Enemy simulation, camera, level transitions, and death handling are
    unchanged from 2b.

### Behavior parity

34. Holding `Right` moves the player right immediately (local prediction) and
    the position is corrected by later `PlayerPos` arrivals (reconciliation).
35. Remote players appear with smooth interpolation (no 20 Hz stepping);
    subjectively, remote movement is continuous.
36. Inputs sent on unreliable with sticky edges: a single `Space` press
    produces a jump (3 chances to deliver to the server). Under normal network
    conditions (LAN, no artificial loss), jump and shoot are reliable.
37. Under simulated packet loss (e.g., 10% drop in a unit test via a mock
    channel), missing inputs are detected via seq gaps and corrected by the next
    `PlayerPos` reconciliation.

### Quality gates

38. `cargo build --workspace` succeeds.
39. `cargo test --workspace` passes (all new prediction/reconciliation/buffer
    tests green, plus all existing tests from 2a/2b/protocol).
40. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
41. `cargo fmt --check --all` reports no diffs.
42. A `grep -rn "unwrap()\|expect(" client/src/network/prediction.rs` finds
    zero uses (no panics in prediction code; all `Option` handling is explicit).