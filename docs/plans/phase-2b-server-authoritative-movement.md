# Phase 2b — Server-authoritative movement — Implementation Plan

- Status: implemented
- Spec: `docs/specs/phase-2b-server-authoritative-movement.md`
- Date: 2026-11-24
- Branch: `feat/server-authoritative-movement`

## Overview

Make the server the authority on **player movement** (ADR-001 step 2). The client
stops sending its position (`PlayerPos`) and the client→server `Action{shoot}`;
it sends a new `GamePacket::PlayerInput` (held move flags + edge jump/shoot +
room coords) every frame on the reliable-ordered channel. The server drains
events non-blockingly (`try_recv()`), accumulates inputs per player (D4), and
every `1.0 / TICK_HZ` seconds simulates each player through
`game::entities::player::Player::update` and broadcasts authoritative `PlayerPos`
(unreliable) plus a server-generated `Action{shoot}` (reliable-ordered) when a
`shoot` edge was latched. The client renders its **own** player from the
authoritative broadcast ("dumb terminal") exactly as it already renders remote
`Nemesis` mirrors.

**Scope boundary (D6):** only *movement* is authoritative. Enemies and bullets
stay client-simulated; the server never simulates them and forces `shoot: false`
into its own `Player::update` (the server's only reaction to a latched `shoot` is
the `Action` rebroadcast). The client's own-bullet spawn/update/collision moves
out of `Player::update` into the client `World` layer.

### ⚠ This is a behavior-changing phase (not a refactor)

Unlike 2a, this phase changes how the game *feels* and *behaves*. The reviewer and
QA must check these explicitly:

1. **Client loses self-simulation (latency).** `World::update` stops calling
   `self.player.update(...)` for the local player, so a keypress now travels
   client → server → back before movement is visible (one round-trip). On
   localhost/LAN it is small but noticeable; over the internet it is laggy. This
   is the *accepted* cost of authority-without-prediction and is restored by 2c.
   Between broadcasts the player stands still (no extrapolation).
2. **Room-entry glitch is accepted.** Room transitions
   (`check_level_transitions`) still run client-side and reposition the player at
   the new room's entry; but because the server is authoritative, the client's
   entry teleport is overwritten by the next authoritative `PlayerPos`, which may
   briefly place the player near the boundary rather than the entry. Authoritative
   transitions fix this in a later phase (ADR-001 open question).
3. **Client→server `PlayerPos` / `Action{shoot}` are removed.** The server no
   longer accepts or rebroadcasts client positions or client `Action` messages;
   `Action` becomes server→client only. Any stale client that still sends them is
   ignored.
4. **Client-side respawn position is non-authoritative.** `handle_death` still
   sets `world.player.pos = (100,100)` client-side; the server's next broadcast
   overwrites it (same mechanism as the room-entry glitch). `Hit` handling itself
   is out of scope and unchanged.

### TDD / dependency ordering note

This phase is **not** a pure refactor, so there are no "moved" tests — only **new
RED** tests plus **unchanged** tests. Crucially, the new tests cannot even
**compile** until two things exist:

- the `GamePacket::PlayerInput` variant (protocol test), and
- a `ServerWorld` **skeleton** (struct + method signatures) in
  `server/src/world.rs` (server inline tests).

The tester authors the test *code* first against the public API below; the
workspace will fail to build until the implementer lands Box 1 (protocol variant)
and Box 2 (`ServerWorld` skeleton). From then on the assertions are genuinely RED
(they exercise accumulator / `update` behavior that is not yet implemented) until
Boxes 2–4 land. The protocol round-trip test is a special case: it is RED only as
a compile error; once the variant + derives exist it passes immediately (postcard
is declarative) — treat it as a wire-format canary, not a logic test.

### Server lib/bin split for tests — recommendation

The server is currently a **binary crate** (`server/src/main.rs` with `fn main`,
no `lib.rs`). The only pure logic to unit-test (the input accumulator,
`apply_input`, `update`) lives entirely inside `ServerWorld`, a self-contained
module with no I/O. **Recommendation: put the server buffer tests inline in
`server/src/world.rs` (`#[cfg(test)] mod tests`) and do NOT extract a server lib
in this phase.**

- Inline tests run under `cargo test -p metroidvania-server`, and because
  `#[cfg(test)] mod tests` is a child module of `world.rs` it has full access to
  private fields (e.g. `pending`) — the buffer tests can assert the accumulator
  directly. This mirrors how `game`'s 27 tests are already structured.
- The tick loop in `main.rs` is I/O (socket + `Instant`), which is **not**
  unit-tested (rendering/input/network loops are excluded per the project's
  "test the logic" rule). A lib/bin split would only buy `server/tests/`
  integration tests, adding no coverage over inline tests while forcing `main.rs`
  to split and the `SocketAddr ↔ u8` transport state to be relocated.

**Trade-off:** a lib/bin split would enable `server/tests/` integration tests and
a public `ServerWorld` API (mirroring the 2a decision to extract `game`). It is
worth doing **later** — when the server accumulates enough pure logic to justify
integration tests, or alongside the client lib/bin split already noted in
`docs/progress.md`. 2a extracted `game` because the simulation had to be *shared*
by two crates; here `ServerWorld` is server-only, so there is no sharing pressure
yet. Defer; do not over-engineer.

## Public API (targets for the tester)

### `protocol` (`protocol/src/lib.rs`) — one new variant

```rust
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    // ... existing variants (JoinRequest, JoinResponse, PlayerPos, Action, Leave, Hit)
    //     UNCHANGED in shape ...

    PlayerInput {
        id: u8,           // owner id (validated against the server's addr→id map)
        level_x: u8,      // room the client is in (grid column)
        level_y: u8,      // room the client is in (grid row)
        move_left: bool,  // held
        move_right: bool, // held
        jump: bool,       // pressed (edge)
        shoot: bool,      // pressed (edge)
    },
}
```

`PlayerPos` and `Action` keep their exact existing shapes. `protocol/Cargo.toml`
gains **no** `game`/`glam` dependency (serde only); `grep -rn "game" protocol/src`
must find nothing.

### `server` — revived `ServerWorld` (`server/src/world.rs`, declared `mod world;`)

```rust
use std::collections::HashMap;
use game::entities::player::{Player, PlayerInput};
use game::level::Level;
use game::maps::Map;   // NOT std::iter::Map

pub struct ServerWorld {
    players: HashMap<u8, Player>,           // keyed by id
    pending: HashMap<u8, PlayerInput>,      // per-player input accumulator (D4)
    coords:  HashMap<u8, (u8, u8)>,         // each player's reported room (level_x, level_y)
    levels:  Vec<Vec<Level>>,               // built once from Map::new()
}

impl ServerWorld {
    /// Build the levels matrix from Map::new() (same grids.zip(data) fold as the
    /// client's World::new); empty players/pending/coords maps.
    pub fn new() -> Self;

    /// players.insert(id, Player::new(id)); pending.entry(id).or_default();
    /// coords.entry(id).or_insert((0, 0));
    pub fn spawn(&mut self, id: u8);

    /// Remove id from players, pending, and coords.
    pub fn remove(&mut self, id: u8);

    /// Accumulate one input packet (D4):
    ///   - coords.insert(id, (x, y));  (record reported room: `coords` stores
    ///     `(level_x, level_y)` = `(x, y)` = (column, row))
    ///   - pending entry for id:
    ///       held flags (move_left, move_right) OVERWRITTEN by the latest input;
    ///       edge flags (jump, shoot) OR-accumulated (|=);
    ///       vacant entry → insert the input as-is.
    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput);

    /// Tick every player. For each id (snapshot the keys first to avoid a double
    /// borrow): select the collision `Level` at the player's reported coords —
    /// **`levels[y][x]`** where `(x, y) = coords[id]` (the matrix is row-major:
    /// indexed `[row_y][col_x]`, NOT `[x][y]`). Client-trusted coords may be out
    /// of range or missing, so resolve with `.get(y).and_then(|row| row.get(x))`
    /// and fall back to `levels[0][0]` for BOTH out-of-range and absent entries
    /// (otherwise a bogus `(255,255)` index-panics the server). Reconstruct a
    /// game::PlayerInput from the accumulator with `shoot` forced false (D6), call
    /// player.update(dt, &level, &input), then clear the latched jump/shoot edges.
    /// Returns the list of (id, dir) for players whose `shoot` edge was latched
    /// this tick — main.rs turns each into Action{shoot}. Held flags persist.
    pub fn update(&mut self, dt: f32) -> Vec<(u8, f32)>;

    // Read access for the authoritative PlayerPos broadcast (transport-agnostic:
    // ServerWorld never sees a SocketAddr). Added beyond the spec's five methods
    // because main.rs must read pos/dir/coords to broadcast and tests must read
    // the simulated position.
    pub fn players(&self) -> &HashMap<u8, Player>;
    pub fn coords(&self) -> &HashMap<u8, (u8, u8)>;
}
```

`update` return/emit decision: `Vec<(u8, f32)>` of `(id, dir)` — the spec leaves
"return/emit shoot flags" open; this is the smallest transport-agnostic shape that
lets `main.rs` build `Action { id, kind: "shoot".into(), dir }` without `ServerWorld`
knowing about `GamePacket`. The alternative (`Vec<u8>` of ids + a `dir` getter)
forces a second lookup in `main.rs`; rejected for the tuple.

The dead stub's `std::iter::Map`, `map: Map` field, and undefined `PlayerData` are
removed. `HashMap<SocketAddr, u8>` (addr→id) and `next_id` **stay in `main.rs`**.

### `server/src/main.rs` — fixed-step loop (D3, D8)

Not a "public API", but the exact shape the implementer must produce:

```rust
use std::time::{Duration, Instant};
use game::config::TICK_HZ;

// one place each; no 20 / 50 / 50.0 / 0.05 literals anywhere in server/src
let tick_interval = Duration::from_secs_f64(1.0 / TICK_HZ as f64);
let tick_dt       = 1.0 / TICK_HZ as f32;   // 0.05 at 20 Hz — fixed step, NOT wall clock
```

Loop body: (a) drain `event_receiver.try_recv()` (no blocking `recv()`), handling
`JoinRequest` → allocate id, `addr→id`, `ServerWorld::spawn(id)`,
`JoinResponse{assigned_id: id}`; `PlayerInput` → resolve id from the addr→id map
(ignore the packet `id` if it disagrees), `ServerWorld::apply_input(id, level_x,
level_y, PlayerInput{..})`; `Leave`/`SocketEvent::Timeout` → remove from both maps
+ `ServerWorld::remove(id)` + broadcast `Leave`; `Hit` unchanged. Then (b) if
`Instant::now() >= next_tick`, advance `next_tick += tick_interval` (small catch-up
loop if the drain overran), call `world.update(tick_dt)`, broadcast every player's
`PlayerPos` (unreliable, to **all** connected players **including the sender** —
the existing `if player_addr != addr` exclusion must be removed so the sender
receives its own echo) and, for each `(id, dir)` in the returned shooters,
`Action{shoot}` (reliable-ordered, to others — excluding the shooter, whose own
client spawns its bullet locally). (c) sleep a few ms when idle. Client-sent
`PlayerPos` / `Action` handling is removed.

### `client` — drop self-simulation, relocate own bullets (D5, D6)

`client/src/world/game.rs`:

```rust
pub struct World {
    pub id: Option<u8>,
    pub player: Player,             // now authoritative: pos/dir set from PlayerPos
    pub bullets: Vec<Bullet>,       // NEW — own bullets (moved out of Player::update)
    pub enemies: Vec<Enemy>,
    pub levels_matrix: Vec<Vec<Level>>,
    pub current_coords: (usize, usize),
    pub camera: GameCamera,
    pub network: NetworkClient,
    pub other_players: HashMap<u8, Nemesis>,
    pub enemy_bullets: Vec<Bullet>,
}
```

`World::update` no longer calls `self.player.update(...)`. It samples **only the
`Z` edge** (`is_key_pressed(KeyCode::Z)`) to spawn the local own bullet; it no
longer builds a full `PlayerInput` (there is nothing left to feed it to). The
outgoing `PlayerInput` — all four keys (`is_key_down(Left/Right)`,
`is_key_pressed(Space/Z)`) — is built and sent in `update_network`
(`client/src/network/world.rs`), the only place the full mapping is sampled. Own
bullets are spawned (`Bullet::new(player.pos + Vec2::new(w/2, h/2), player.dir,
player.id)` on the `Z` edge), updated each frame (`bullet.update(dt, level)`),
collided with local enemies, and retained (`bullets.retain(|b| b.active)`),
mirroring the old `Player::update` bullet block. Enemy simulation,
`update_enemy_bullets_position`, `check_player_death`, camera, and
`check_level_transitions` all still run. `draw` renders `self.bullets` (not
`self.player.bullets`).

`client/src/network/world.rs`:

- `update_network`: send a single `GamePacket::PlayerInput` (reliable-ordered)
  every frame once `world.id` is set, with `id`, `level_x`/`level_y` =
  `current_coords`, and the sampled held/edge flags. Send **no** `PlayerPos` and
  **no** `Action`.
- `handle_packet` on `PlayerPos`: **new own-player branch** when
  `Some(id) == world.id` → set `world.player.pos = vec2(x, y)` and
  `world.player.dir = dir` (ignore `level_x`/`level_y`; `current_coords` stays
  client-side). The `id != world.id` branch (update/create `Nemesis`, set
  `coords`) is **unchanged**. `Action{shoot}` handling unchanged.

## Test cases (tester writes these first)

Legend: **[NEW]** = genuinely new, RED until the noted box lands (the protocol
case is RED only as a compile error, then passes immediately). **[UNCHANGED]** =
already passing, must stay green. There are **no [MOVED]** tests — this phase
adds code and changes behavior; it relocates no test.

### [NEW] `protocol/tests/round_trip.rs` (1 test) — RED (compile) until Box 1

- `round_trip_player_input` — `round_trip(GamePacket::PlayerInput { id: 7, level_x:
  1, level_y: 2, move_left: true, move_right: false, jump: true, shoot: false })`
  and a second value with the complement bits / extremes (`id: 0` vs `id: 255`,
  all-bool-true vs all-false) to prove every bool and byte survives postcard
  round-trip.

### [NEW] `server/src/world.rs` inline `#[cfg(test)] mod tests` (4 required + 2 recommended) — RED until Boxes 2–4

Fixture note: `ServerWorld::new()` builds the real `Map::new()` levels (20×12
tiles = 800×480 px), so `Player::new(0)`'s spawn at `(100, 100)` with w=30/h=38
is fully in-bounds of room `(0,0)` (unlike the 2a player tests, no hand-built
12×12 fixture is needed). Use `let dt = 1.0 / game::config::TICK_HZ as f32;`
(=`0.05`). Because the tests are a child module of `world.rs`, they may read the
private `pending` accumulator directly.

- `input_held_flags_latest_wins` — `spawn(0)`; `apply_input(0, 0, 0,
  PlayerInput{move_left: true, ..Default::default()})` then `apply_input(0, 0, 0,
  PlayerInput{move_right: true, ..Default::default()})`; assert
  `world.pending[&0].move_right && !world.pending[&0].move_left` (held overwrite);
  then `world.update(dt)` and assert `world.players()[&0].pos.x > 100.0` (moved
  right, proving latest-wins holds through the tick).
- `input_edge_flags_latch_and_clear` — `spawn(0)`; apply two `shoot: true` inputs,
  then `let shots = world.update(dt)`; assert `shots == vec![(0, 1.0)]` (two edges
  → exactly one consumed) and `!world.pending[&0].shoot` (latch cleared); then a
  second `world.update(dt)` returns an empty vec (no re-fire without a new input).
- `input_edge_flag_not_lost_between_ticks` — `spawn(0)`; `world.update(dt)` once
  (tick 1, no inputs); then `apply_input(0, 0, 0, PlayerInput{shoot: true,
  ..Default::default()})`; then `world.update(dt)` (tick 2) → returns
  `vec![(0, 1.0)]`, proving the edge received *between* ticks fires on the next
  tick instead of being dropped.
- `apply_input_update_drives_authoritative_position` — `spawn(0)`; `apply_input(0,
  0, 0, PlayerInput{move_right: true, ..Default::default()})`; `world.update(dt)`;
  assert `world.players()[&0].pos.x > 100.0` and `world.players()[&0].dir == 1.0`
  (the same traversal the 2a `player_input_move_right_moves_right` test locked, now
  driven through `ServerWorld` → `Player::update`).
- [recommended] `apply_input_update_jump_moves_up_when_grounded` — `spawn(0)`;
  set `world.players.get_mut(&0).unwrap().grounded = true`; `apply_input(0, 0, 0,
  PlayerInput{jump: true, ..Default::default()})`; `world.update(dt)`; assert
  `world.players()[&0].pos.y < 100.0` (net `SALTO_FUERZA*dt + GRAVEDAD*dt` is
  negative on the empty floor).
- [recommended] `apply_input_out_of_range_coords_fall_back_to_origin` — `spawn(0)`;
  `apply_input(0, 255, 255, PlayerInput{move_right: true, ..Default::default()})`;
  `world.update(dt)`; assert no panic and `world.players()[&0].pos.x > 100.0`
  (the bogus `(255,255)` coords resolve via `.get(y).and_then(|row| row.get(x))`
  to the `(0,0)` level, where `(100,100)` is in-bounds and move-right advances).

### [UNCHANGED] tests — must stay green

- `protocol/tests/round_trip.rs`: the existing 6 tests
  (`round_trip_join_request`, `round_trip_join_response`, `round_trip_player_pos`,
  `round_trip_action`, `round_trip_leave`, `round_trip_hit`) are untouched.
- `game`: all **27** tests unchanged (types 2, maps 2, level 10, `enemies_from_data`
  3, rect 4, `player_input` 5, `enemy_set_rand_state` 1). The `PlayerInput`/`Player`
  logic they lock is the exact code the server now drives — it must not change.
- No client tests: rendering/input/network loops are not tested (project rule).

## Implementation checklist

Each box is a small, independently verifiable step in dependency order. The
`implementer` fills the `Explain` fields.

### Group 1 — Protocol (AC 1–3)

#### Box 1 — Add `GamePacket::PlayerInput`

- [x] done
- **Acceptance**: `protocol/src/lib.rs` gains `PlayerInput { id: u8, level_x: u8,
  level_y: u8, move_left: bool, move_right: bool, jump: bool, shoot: bool }`;
  `PlayerPos` and `Action` are byte-identical; `protocol/Cargo.toml` declares no
  `game`/`glam`; `grep -rn "game" protocol/src` finds nothing.
- **Test**: `cargo test -p protocol` (new `round_trip_player_input` green; 6
  existing tests green); `grep -rn "game" protocol/src` → no matches.
- **Explain**
  - What: New `GamePacket::PlayerInput` variant with 7 fields (id, level_x,
    level_y, move_left, move_right, jump, shoot). `PlayerPos` and `Action` keep
    their existing shapes unchanged.
  - Why: ADR-001 step 2b — the client now sends inputs (not positions) so the
    server can simulate authoritatively. The flat bool variant keeps the
    dependency direction clean (`protocol` stays a pure serde leaf crate).
  - Rust concepts: Serde derive on an enum variant; flat data struct in an enum.
  - Watch out: This `PlayerInput` (7 fields incl. id/level) is DIFFERENT from
    `game::PlayerInput` (4 fields: move/jump/shoot). Translation happens at the
    boundary (client builds one from keys, server reconstructs the other).

### Group 2 — ServerWorld + tick loop (AC 4–9)

#### Box 2 — Revive `ServerWorld` (`server/src/world.rs` + `mod world;`)

- [x] done
- **Acceptance**: `server/src/main.rs` declares `mod world;`; `world.rs` defines
  `ServerWorld` with the struct + `new`/`spawn`/`remove`/`apply_input`/`update`/
  `players`/`coords` signatures above, builds `levels` from `game::maps::Map::new()`,
  and imports `game::maps::Map` (not `std::iter::Map`),
  `game::entities::player::{Player, PlayerInput}`, `game::level::Level`,
  `std::collections::HashMap`; the stub's `PlayerData` and `map: Map` field are
  gone. The server now `use`s `game`, so the `unused dependency: game` warning is
  gone. This box delivers the **skeleton** the tester's inline tests compile
  against (method bodies may be `todo!()`/empty until Box 4).
- **Test**: `cargo build -p metroidvania-server` (no `unused dependency: game`
  manifest warning); `grep -n "iter::Map\|PlayerData" server/src/world.rs` → no
  matches; `grep -n "mod world" server/src/main.rs`.
- **Explain**
  - What: Revived `server/src/world.rs` from a dead stub into a real `ServerWorld`
    with authoritative `HashMap<u8, Player>` players, per-player `PlayerInput`
    accumulator, reported coords, and a level matrix built from `game::maps::Map`.
    The `unused dependency: game` on the server is gone.
  - Why: ADR-001 step 2b needs an authoritative simulation container. The
    accumulator design (held overwrite + edge OR-latch) decouples 60 Hz client
    sends from the 20 Hz server tick without losing edge-triggered events.
  - Rust concepts: `HashMap::entry().or_default()` for lazy per-player state;
    `.get().and_then()` safe nested Vec lookup with fallback; `Vec<Vec<Level>>`
    row-major indexing (`levels[cy][cx]`); private-field access from `#[cfg(test)]`
    child module.
  - Watch out: `levels` indexes `[row_y][col_x]`, not `[x][y]`. Client-trusted
    coords use `.get()` fallback to `levels[0][0]` — never index-panic.

#### Box 3 — Fixed-step tick loop + join/leave/input wiring (`server/src/main.rs`)

- [x] done
- **Acceptance**: `try_recv()` drain (no blocking `recv()`); tick interval =
  `Duration::from_secs_f64(1.0 / TICK_HZ as f64)` and `dt = 1.0 / TICK_HZ as f32`
  (fixed step); `Instant`-based `next_tick` advance with a small catch-up loop; a
  few-ms idle sleep. `JoinRequest` → allocate id + `addr→id` + `ServerWorld::spawn(id)`
  + `JoinResponse{assigned_id: id}`; `PlayerInput` → resolve id from the addr→id map
  (ignore packet `id` on mismatch) → `apply_input(id, level_x, level_y, input)`;
  tick → `world.update(dt)` → broadcast every player's `PlayerPos` (unreliable, to
  **all** connected players including the sender — change the existing
  `if player_addr != addr` loop to send to every player) and `Action{shoot}`
  (reliable-ordered, to others, excluding the shooter) for each returned
  `(id, dir)`; `Leave`/`Timeout` → remove from both maps + `ServerWorld::remove(id)`
  + broadcast `Leave`; client-sent `PlayerPos`/`Action` handling removed. No tick
  literals in `server/src`.
- **Test**: `grep -nE "\b20\b|50\.0|\b50\b|0\.05" server/src` → no matches;
  `grep -n "try_recv" server/src/main.rs`; `grep -n "TICK_HZ" server/src/main.rs`;
  `cargo build -p metroidvania-server`.
- **Explain**
  - What: Rewrote `server/src/main.rs` from a blocking `recv()` event loop to a
    non-blocking `try_recv()` + `Instant`-based fixed-step tick loop. The tick
    runs at `1.0/TICK_HZ` seconds (20 Hz). Each tick: `world.update(dt)` → broadcast
    authoritative `PlayerPos` to ALL players (including sender) on unreliable
    channel; rebroadcast `Action{shoot}` to others on reliable-ordered.
    Client→server `PlayerPos`/`Action` handling removed; Spanish prints translated.
  - Why: ADR-001 step 2b — the server must simulate at a fixed rate regardless of
    packet arrival patterns. Non-blocking drain prevents slow/malicious clients
    from stalling the tick. Broadcast-to-all (not broadcast-to-others) is required
    so each client's own-player branch receives its authoritative position.
  - Rust concepts: `Instant`-based fixed-step loop with `Duration::from_secs_f64`;
    `try_recv()` non-blocking channel drain; `checked_duration_since` for safe
    sleep calculation.
  - Watch out: The `shoot` rebroadcast loop uses `if player_id == shooter_id { continue; }`
    to exclude the shooter (their own client spawns the bullet locally). The
    `PlayerPos` broadcast does NOT exclude the sender — critical for the
    dumb-terminal model.

### Group 3 — Input buffering (AC 10–11)

#### Box 4 — Accumulator semantics + buffer tests green

- [x] done
- **Acceptance**: `apply_input` overwrites held flags and ORs edge flags (D4);
  `update` reconstructs a `PlayerInput` with `shoot` forced false, drives
  `Player::update`, clears latched edges, and returns `Vec<(u8, f32)>` shooters.
  The inline buffer/authoritative tests pass.
- **Test**: `cargo test -p metroidvania-server`
  (`input_held_flags_latest_wins`, `input_edge_flags_latch_and_clear`,
  `input_edge_flag_not_lost_between_ticks`,
  `apply_input_update_drives_authoritative_position` [+ recommended jump and
  out-of-range-coords tests]).
- **Explain**
  - What: Implemented the accumulator semantics: `apply_input` overwrites held
    flags (move_left/move_right = latest-wins) and OR-accumulates edge flags
    (jump/shoot `|=`). `update` reconstructs a `PlayerInput` with `shoot: false`
    (server never spawns bullets), drives `Player::update` (which uses the
    authoritative `game` traversal locked by existing tests), clears latched
    edges, and returns `Vec<(u8, f32)>` for main.rs to build `Action{shoot}`.
  - Why: The accumulator decouples the client's 60 Hz send rate from the server's
    20 Hz tick rate. Held flags must NOT accumulate (pressing Left then Right
    should move right, not cancel out). Edge flags MUST accumulate (pressing Z
    twice between ticks should only fire once). Forcing `shoot: false` in the
    server's `PlayerInput` keeps bullets client-side (D6 scope boundary).
  - Rust concepts: `|=` boolean OR-assign for edge accumulation; `HashMap::entry()`
    for lazy initialization of per-player state; `Vec<(u8, f32)>` return type
    as a transport-agnostic shoot event (id + dir tuple).
  - Watch out: The 6 server tests pass and directly validate the accumulator
    behavior (held overwrite, edge latch+clear, inter-tick edge preservation,
    authoritative position drive, jump-on-grounded, and out-of-range coords
    fallback to origin).

### Group 4 — Client (AC 12–15)

#### Box 5 — Client network: send inputs, apply authoritative pos (`client/src/network/world.rs`)

- [x] done
- **Acceptance**: `update_network` sends a single `GamePacket::PlayerInput`
  (reliable-ordered) each frame once `world.id` is set (id, current_coords as
  level_x/level_y, held move flags, edge jump/shoot); sends **no** `PlayerPos` and
  **no** `Action`. `handle_packet` `PlayerPos` gains the own-player branch
  (`Some(id) == world.id` → set `world.player.pos`/`dir`) and keeps the remote
  `Nemesis` branch unchanged; `Action{shoot}` handling unchanged. This own-player
  branch is reachable because the server (Box 3) echoes each player's own
  `PlayerPos` back to the sender (broadcast-to-all) — without that echo the local
  player would never move.
- **Test**: `grep -n "GamePacket::PlayerInput" client/src/network/world.rs`;
  `grep -n "GamePacket::PlayerPos" client/src/network/world.rs` (only inside
  `handle_packet`, not `update_network`); `grep -n "GamePacket::Action" client/src/network/world.rs`
  (only inside `handle_packet`); `cargo build -p metroidvania-rust`.
- **Explain**
  - What: `update_network` now sends a single `GamePacket::PlayerInput` each frame
    (reliable-ordered) with held Left/Right + edge Space/Z + room coords. No more
    `PlayerPos` or `Action` send. `handle_packet` inverts the `PlayerPos` branch:
    `Some(id) == world.id` (own player) applies authoritative `pos`/`dir` from the
    server; the else branch (remote player) updates/creates `Nemesis` as before.
    Translated remaining Spanish prints to English.
  - Why: The client is now a dumb terminal for movement — it reads macroquad keys,
    ships them to the server, and renders whatever the server says. The own-player
    branch replaces the old self-simulation. The shoot edge is sampled alongside
    the input so the server's edge-latch works correctly.
  - Rust concepts: Match-arm flow inversion (own-vs-remote player); `is_key_down`
    (held) vs `is_key_pressed` (edge) input encoding.
  - Watch out: The `Z` key is sampled in two places — once here for the outgoing
    `PlayerInput.shoot` (server edge-latch) and once in `World::update` for
    immediate local bullet spawn (responsive feel). Macroquad's edge state is
    stable within a frame so both reads agree. Phase 2c may consolidate this.
    Also: client feels latent until 2c (prediction); room-entry glitch accepted.

#### Box 6 — Client World: drop self-sim, relocate own bullets (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: `World::update` no longer calls `self.player.update(...)`; the
  key→`PlayerInput` mapping is unchanged; own bullets are spawned on the Z edge
  (from authoritative `pos`/`dir`), updated, collided with local enemies, and
  retained in a new `World.bullets` field; `draw` renders `self.bullets`; enemy
  sim, `update_enemy_bullets_position`, `check_player_death`, camera, and
  `check_level_transitions` still run. **Also update `client/src/world/lib.rs`:
  `load_current_level` must clear `world.bullets` (the new field) instead of
  `world.player.bullets`, so own bullets are dropped on death/respawn and room
  transition (`handle_death` and `check_level_transitions` both call
  `load_current_level` — the only call site that touches own bullets is
  `load_current_level`'s `world.player.bullets.clear()` at
  `client/src/world/lib.rs:54`). `game::Player.bullets` is no longer the source
  of truth for the own player — it is never populated server-side (`shoot: false`)
  and unused client-side once `World.bullets` takes over.**
- **Test**: `grep -n "player.update" client/src/world/game.rs` → no match;
  `grep -n "pub bullets" client/src/world/game.rs`; `grep -n "self.bullets" client/src/world/game.rs`;
  `grep -n "bullets.clear" client/src/world/lib.rs` (must show `world.bullets`,
  not `world.player.bullets`); `cargo build -p metroidvania-rust`.
- **Explain**
  - What: Removed `self.player.update(dt, level, &input)` from `World::update`
    (the local player no longer self-simulates). Own-bullet logic (spawn on Z
    edge, update, collide with local enemies, retain active) relocated from
    `player.bullets` to a new `World.bullets: Vec<Bullet>` field. `draw` renders
    `self.bullets`. `load_current_level` in `lib.rs` now clears `world.bullets`
    (not `world.player.bullets`). The unused `PlayerInput` import dropped.
  - Why: The server is now authoritative for movement, so the client's own player
    position comes from `handle_packet → PlayerPos` (Box 5). Own bullets must move
    to `World` because `Player.bullets` was updated inside the removed
    `player.update()` call. Client prediction (Phase 2c) will reconcile the
    authoritative position with local simulation.
  - Rust concepts: `Vec<Bullet>` field relocation between structs; retaining
    active bullets with `.retain(|b| b.active)`.
  - Watch out: Enemies are still client-simulated (not authoritative yet — scope
    boundary D6). The `Z` edge sampled for local bullet spawn uses the
    authoritative `player.dir` from the last `PlayerPos`, which may lag the
    server's dir by one round-trip on direction-change frames (documented,
    Phase 2c fix).

### Group 5 — Behavior parity + retune (AC 16–18)

#### Box 7 — Movement parity, broadcast, one-line retune

- [x] done
- **Acceptance**: holding `Right` moves the authoritative player right at
  `VELOCIDAD`, `Left` left, `Space` jumps with `SALTO_FUERZA`, gravity `GRAVEDAD`,
  `dir` follows the held key (same traversal, now server-driven); `PlayerPos`
  reaches **all** connected players each tick — the sender applies its own echo,
  remote players mirror via `Nemesis` — and a new joiner is reflected on the next
  tick via the existing "create `Nemesis` if unseen" path; changing `TICK_HZ` 20→60
  in `game/src/config.rs` (and nothing else) changes the tick rate — no other file
  references a hardcoded tick number.
- **Test**: code-read `grep -rnE "\b20\b|50\.0|\b50\b|0\.05" server/src` → no
  matches; `grep -rn "TICK_HZ" server/src`; manual two-client `cargo run` smoke
  (movement is smooth but latent, remote mirror moves, shoot spawns `enemy_bullet`).
  This box is behavioral (not unit-tested); the 2a `game` player tests already lock
  the constants.
- **Explain**
  - What: Verified behavioral parity: the server's `Player::update` call uses the
    exact same `game::config` constants (`VELOCIDAD`, `GRAVEDAD`, `SALTO_FUERZA`,
    `BULLET_SPEED`) as the old client-side simulation. `PlayerPos` broadcast
    reaches all connected players each tick (including sender — own echo applied;
    remotes mirror via `Nemesis`). `TICK_HZ` is the single source of truth: zero
    hardcoded `20`/`50`/`0.05` literals in `server/src/`, confirmed by grep.
  - Why: The existing `game` player tests (5 tests: move_left, move_right,
    jump_grounded, jump_airborne, shoot) already lock the traversal behavior.
    Since the server drives the same `Player::update` code path, the same behavior
    is guaranteed — these tests are the regression canary.
  - Rust concepts: Single-source-of-truth constants via `game::config`; `grep`
    absence proof of no tick-rate duplication.
  - Watch out: Movement is latent (one round-trip) until Phase 2c. At 20 Hz the
    tick step is 50 ms — smooth on localhost but noticeable. Room-entry glitch
    accepted (client-side `check_level_transitions` can be briefly overwritten by
    the next authoritative `PlayerPos`).

### Group 6 — Quality gates (AC 19–22)

#### Box 8 — Workspace build/test/clippy/fmt

- [ ] done
- **Acceptance**: `cargo build --workspace` succeeds (all four crates);
  `cargo test --workspace` passes (existing `game` 27 + `protocol` 7 + new server
  buffer tests green; no window/GPU code exercised); `cargo clippy --all-targets
  --workspace -- -D warnings` exits 0; `cargo fmt --check --all` reports no diffs.
- **Test**: `cargo build --workspace`; `cargo test --workspace`;
  `cargo clippy --all-targets --workspace -- -D warnings`; `cargo fmt --check --all`.
- **Explain**
  - What: All four quality gates pass on the workspace: 40 tests green (27 game
    + 7 protocol round-trip + 6 server accumulator), clippy `-D warnings` clean,
    `rustfmt` clean, `cargo build` zero errors and zero `unused dependency` warnings.
  - Why: The gates are the project's invariant per `CONTRIBUTING.md`. No Phase 2b
    change can regress existing tests or introduce new lints.
  - Rust concepts: Workspace-level `cargo build|test|clippy|fmt` for multi-crate
    validation; `#[derive(Default)]` for test fixture conciseness.
  - Watch out: The code review flagged three minor items (removed unused `serde`
    dep from server, two residual Spanish printlns translated) — all addressed in
    the cleanup commit `3cd345a`. The remaining silent-send and hot-loop-unwrap
    items are pre-existing patterns accepted for now.

## Open questions

- **`update` return type** — the spec leaves "return/emit shoot flags" open. This
  plan chose `Vec<(u8, f32)>` (id, dir). If the owner prefers `ServerWorld` to stay
  fully ignorant of `dir`, `Vec<u8>` (ids) + a `dir` read via `players()` is the
  fallback; the tuple is simpler for `main.rs`.
- **Own-bullet Z edge sampling** — the client samples `is_key_pressed(Z)` in two
  places (once to spawn the own bullet in `World::update`, once for the outgoing
  `PlayerInput.shoot` in `update_network`). This is identical to today's double
  sample (input vs `Action`); macroquad's edge state is stable within a frame.
  Note it in the `Explain` block rather than "fixing" it.
- **Server lib/bin split** — deferred (see Overview recommendation). Revisit in 2c
  or when the server grows more pure logic; the client lib/bin split (already noted
  in `docs/progress.md`) is the natural companion change.
- **Respawn/`handle_death` position** — client-side `world.player.pos = (100,100)`
  is now non-authoritative and overwritten by the next broadcast (accepted, same as
  the room-entry glitch). `Hit` authority is out of scope; flag for a later phase.
