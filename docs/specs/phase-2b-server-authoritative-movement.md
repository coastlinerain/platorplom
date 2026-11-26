# Phase 2b — Server-authoritative movement

- Status: accepted
- Date: 2026-11-24
- Branch: `feat/server-authoritative-movement`

## Summary

ADR-001 (Accepted) chose **server-authoritative movement**. Step 2a extracted the
shared `game` crate (levels, types, maps, collision, entity physics, and an
input-driven `Player::update`) so both sides run the *same* simulation code.
This phase (2b) makes the server the authority on **player movement**: the client
stops sending its position and instead sends **inputs**, the server simulates
authoritative player state at a **fixed 20 Hz tick** (`TICK_HZ`) and broadcasts
it, and the client renders whatever the server last broadcast.

Concretely:

- The client sends a new `GamePacket::PlayerInput` (held move flags + edge
  jump/shoot + the room it is in) every frame, and **no longer** sends
  `PlayerPos` or `Action{shoot}`.
- The server gains a real fixed-step tick loop (non-blocking `try_recv()`) that,
  every `1.0 / TICK_HZ` seconds, applies each player's buffered input through
  `game::entities::player::Player::update` and broadcasts the authoritative
  `PlayerPos` to **all** connected players (including the sender, so the sender
  can apply its own authoritative position).
- The client renders its **own** player from the authoritative `PlayerPos`
  (the "dumb terminal" model) as well as mirroring remote players — exactly what
  it already does for remote players today.
- **Scope boundary:** only *movement* (position, velocity, direction, grounded) is
  authoritative. Enemies and bullets stay client-simulated per client; the server
  does **not** simulate them (a follow-up, see Decisions 6).

This is a **behavior-changing phase** (unlike the 2a refactor): the client loses
its local self-simulation of movement, and there is an input → round-trip →
visible-movement latency until 2c (prediction + reconciliation) restores
responsiveness.

## Decisions

These are the product/trade-off calls the spec resolves. Each is stated with its
rationale so the planner does not have to re-derive them.

### D1. Protocol: a flat `PlayerInput` packet, not a `game` dependency

**Decision:** add a new `GamePacket::PlayerInput` variant with **flat fields**
(`id: u8, level_x: u8, level_y: u8, move_left: bool, move_right: bool, jump:
bool, shoot: bool`). Reuse the existing `PlayerPos` variant unchanged for the
authoritative broadcast (same shape: `id, x, y, dir, level_x, level_y`).

**Rejected alternative:** deriving `Serialize` on `game::PlayerInput` and making
`protocol` depend on `game`. This would couple the serde-only leaf crate to the
glam-only crate and force `game` to pull in `serde` (or `protocol` to pull in
`glam`), for zero benefit. The flat variant keeps the dependency direction clean
(`protocol` ← `game`? No: `protocol` stays a leaf, `server`/`client` depend on
both `protocol` and `game`). The 4-bool duplication of `PlayerInput`'s shape is
trivial and is translated to/from `game::PlayerInput` at the boundary (client
builds one from keys and sends its fields; server reconstructs one to pass to
`Player::update`).

**What happens to `Action { kind: "shoot" }`:** it is **subsumed** as a
*client→server* message. The client no longer sends `Action`; `shoot` rides in
`PlayerInput.shoot`. The server, on a latched `shoot` input, generates and
rebroadcasts `Action { id, kind: "shoot", dir }` to the other clients (reliable,
as today) so their `enemy_bullets` still spawn. `Action` is therefore retained as
a **server→client only** notification for this phase. (When bullet authority
lands in a later phase, `Action` will be removed or repurposed entirely.)

### D2. Input transport: reliable-ordered

**Decision:** the client sends `PlayerInput` on the **reliable-ordered** channel
(`laminar::Packet::reliable_ordered`).

**Rationale:** `PlayerInput` carries *edge-triggered* flags (`jump`, `shoot`).
On unreliable UDP an edge can be dropped silently and a jump/shot is lost; on
reliable-ordered it cannot. Ordering also prevents stale held flags from
reordering (a later `move_left` overwriting an earlier `move_right` out of
order). The cost — extra latency and head-of-line blocking under loss — is
acceptable at 20 Hz / a handful of players and is exactly what 2c (prediction
with input sequence numbers/acks and unreliable inputs) will address. This is the
simplest *correct* option; a separate reliable edge-event channel is premature.

### D3. Tick loop: single thread, `try_recv()` + `Instant` fixed step

**Decision:** replace the blocking `event_receiver.recv()` with a single-threaded
loop that (a) drains all pending events via `try_recv()`, then (b) runs the
fixed-step tick when `1.0 / TICK_HZ` seconds have elapsed since the last tick,
advancing a `next_tick: Instant` by the tick interval (catching up in a small
loop if the event drain overran), and (c) sleeps a few ms when idle to avoid
busy-spin.

**Rejected alternative:** a dedicated tick thread + channel. Unnecessary for a
learning project: laminar already runs its own polling thread, so the main thread
only needs to drain events and tick. One thread is simpler to reason about and
keeps the borrow of shared state trivial.

**Fixed step:** the `dt` passed to `Player::update` at tick time is the constant
`1.0 / TICK_HZ as f32` (0.05), **not** wall-clock elapsed time. This is what
makes the simulation deterministic and what makes a later 20 → 60 Hz change a
one-line edit (see D8).

### D4. Input buffering: latest-held + latched-edge

**Decision:** the server keeps, per player, a single `PlayerInput` accumulator:
held flags (`move_left`, `move_right`) are **overwritten** by the latest packet;
edge flags (`jump`, `shoot`) are **OR-accumulated** until a tick consumes them,
then cleared. No per-player queue.

**Rationale:** clients send ~60 Hz while the server ticks 20 Hz, so up to ~3
inputs can arrive per tick. Held flags are idempotent (latest-wins is correct);
edge flags must not be lost between ticks. A queue would re-apply stale held
states in order and is more machinery than needed. The accumulator is the
smallest structure that is both simple and correct.

### D5. Client self-simulation: dumb terminal for movement

**Decision:** in 2b the client does **not** simulate its own player's movement.
`World::update` stops calling `self.player.update(dt, level, &input)` for the
local player; instead `handle_packet` applies authoritative `PlayerPos` to the
local player (`id == world.id`) exactly as it already applies remote `PlayerPos`
to `Nemesis` mirrors. The client still *reads* the keyboard and forwards inputs.

**Latency consequence (explicit):** a keypress now travels client → server → back
before movement is visible (one round-trip). On localhost/LAN this is small but
noticeable; over the internet it is laggy. This is the accepted cost of
authority-without-prediction and is restored by 2c.

### D6. Scope: movement authority only — enemies/bullets stay client-side

**Decision:** 2b is **player-movement authority only**. The server does **not**
simulate enemies or bullets. Enemies, the shooter's own bullets, and remote
`enemy_bullets` all remain client-simulated exactly as today (each client
independent — acceptable for trusted co-op PvE). Enemy/bullet authority is a
**follow-up**, recorded below.

**Consequence handled here:** because the client no longer calls
`Player::update` (D5), the own-bullet spawn/update/collision that used to live
inside `Player::update` must be relocated into the client `World` layer (spawn a
`game::entities::bullet::Bullet` on the Z press from the authoritative `dir`,
update it each frame, collide it with local enemies, retain active ones). No
`game` API change is needed — `Bullet` is public. The server, for its part,
passes `shoot: false` into the authoritative `Player::update` so the server-side
`Player` never spawns a bullet; the server's only reaction to a latched `shoot`
is the `Action{shoot}` rebroadcast (D1).

**Follow-up note (from `docs/progress.md`):** when the server later simulates
enemies, it must **not** replicate today's once-per-frame random draw
(`macroquad::rand::gen_range(0, 4)` in `World::update`). The server must use its
own seedable RNG and draw deterministically — one draw per enemy per decision at
the fixed tick — or client/server will desync. Record as a follow-up, not done
here.

### D7. Spawn/join state: revive `ServerWorld`

**Decision:** revive `server/src/world.rs` (currently an orphan file, never
declared with `mod world;`, and not compiling: it imports `std::iter::Map` and
references an undefined `PlayerData`). Replace it with a real `ServerWorld` that
owns the authoritative simulation: a `HashMap<u8, Player>` keyed by player id,
the level matrix built once from `game::maps::Map::new()`, and the per-player
input accumulators (D4). Expose `update(dt)` (tick all players) and the input
accumulation / spawn / remove entry points.

- The `SocketAddr ↔ u8` id map **stays in `main.rs`** (it is transport state, not
  simulation state). `main.rs` resolves `SocketAddr → id`, then forwards inputs
  to `ServerWorld`; `ServerWorld` never sees a `SocketAddr`.
- On `JoinRequest`, `main.rs` allocates the id, inserts `addr → id`, and calls
  `ServerWorld::spawn(id)` which creates `game::entities::player::Player::new(id)`
  (spawn at `(100, 100)` — the same start position the client's `Player::new(0)`
  uses, so both sides agree on initial state before the first input).
- `main.rs` must add `mod world;` and the corrected imports (`game::maps::Map`,
  `game::entities::player::Player`, `std::collections::HashMap` — no
  `std::iter::Map`).

### D8. Tick-rate as a single named constant

**Decision:** the server derives everything from `game::config::TICK_HZ`
(`u32 = 20`). The tick interval is `Duration::from_secs_f64(1.0 / TICK_HZ as
f64)` and the tick `dt` is `1.0 / TICK_HZ as f32`. There are **no** `20.0`,
`50`, `50.0`, `0.05`, or `20` literals in server code. A 20 → 60 Hz retune is a
one-line change to `game/src/config.rs` only. This is an explicit acceptance
criterion (AC-4 / AC-19).

## Goals / non-goals

### Goals

1. New `GamePacket::PlayerInput` in `protocol`; client sends inputs, not
   positions, and the server applies them through `game::Player::update` at a
   fixed `TICK_HZ` tick.
2. A real fixed-step tick loop in `server/src/main.rs` (non-blocking), driven by
   `game::config::TICK_HZ`.
3. A revived `ServerWorld` (replacing the dead stub) that owns authoritative
   `Player` state and per-player input accumulators.
4. Authoritative `PlayerPos` broadcast every tick (unreliable, latest-wins).
5. The client renders its own player from authoritative state and keeps
   mirroring remote players.
6. `shoot` is folded into inputs; `Action{shoot}` becomes server→client only.
7. Behavior parity for movement: the same input mapping + constants reproduce
   the same traversal as before, now server-driven.
8. Clean quality gates (`build`/`test`/`clippy`/`fmt`) and the removal of the
   server's "unused dependency: `game`" warning (it now `use`s `game`).

### Non-goals (explicitly out of scope)

- **No client prediction / reconciliation** — that is 2c.
- **No anti-cheat** beyond the authority model itself (e.g. no server-side
  validation of input plausibility, no speed/jump rate limiting).
- **No enemy or bullet authority** — the server simulates player movement only;
  enemies/bullets remain client-simulated (follow-up).
- **No authoritative level transitions** — room coords (`level_x`/`level_y`) are
  client-trusted in 2b and used only to select the collision `Level` server-side
  (see Behavior / edge cases). Authoritative transitions are a follow-up (ADR-001
  "Open questions").
- **No level/state streaming** — levels stay hard-coded in `game::maps` and known
  to both sides.
- **No serialization of `game` state** — entities are not `Serialize`; only the
  `protocol` packets are.

## Behavior

### Server

- On start, the server binds the socket, spawns the laminar polling thread,
  constructs a `ServerWorld` (building the level matrix from `Map::new()`), and
  enters the fixed-step loop (D3).
- `JoinRequest` → allocate `id = next_id++`, record `addr → id`, spawn
  `Player::new(id)` in `ServerWorld`, and send `JoinResponse { assigned_id: id }`
  (reliable-ordered, as today).
- `PlayerInput { id, level_x, level_y, move_left, move_right, jump, shoot }` →
  resolve `id` from the `addr → id` map (do not trust the packet's `id` for
  state lookup; if the packet `id` disagrees with the map it is ignored), and
  accumulate into that player's buffer: held flags overwrite, edge flags OR
  (D4). Record the player's current room `(level_x, level_y)`.
- Each tick, for every player: select the `Level` at that player's reported
  coords, reconstruct a `game::PlayerInput` from the accumulator (with `shoot`
  forced `false` — D6), call `Player::update(dt, &level, &input)`, then clear the
  latched `jump`/`shoot` edge flags. Finally, broadcast `PlayerPos { id,
  x: player.pos.x, y: player.pos.y, dir: player.dir, level_x, level_y }`
  (unreliable) to **every** connected player **including the sender** — the
  sender needs its own echo so its own-player branch applies the authoritative
  position (otherwise the local player never moves). If `shoot` was latched,
  broadcast `Action { id, kind: "shoot", dir: player.dir }` (reliable-ordered) to
  the other players (excluding the shooter, whose own client spawns its bullet
  locally).
- `Leave { id }` and `SocketEvent::Timeout(addr)` → remove the player from both
  the `main.rs` `addr → id` map and `ServerWorld`, and broadcast `Leave { id }`
  (reliable-ordered) as today.
- The old handling of client-sent `PlayerPos` (rebroadcast) and client-sent
  `Action{shoot}` (rebroadcast) is **removed**; the server no longer accepts
  client positions or client `Action` messages.
- `Hit { entity_id }` handling is unchanged (out of scope).

### Client

- `update_network`: instead of sending `PlayerPos` + `Action{shoot}`, send a
  single `PlayerInput` (reliable-ordered) every frame once `world.id` is set,
  carrying the current `current_coords` as `level_x`/`level_y` and the sampled
  `PlayerInput` fields (same mapping as today: `Left`/`Right` held, `Space` jump
  edge, `Z` shoot edge). Do **not** send `PlayerPos` or `Action`.
- `handle_packet` on `PlayerPos`: apply to the **own** player when
  `id == world.id` (set `world.player.pos`/`dir`, and `current_coords` stays
  client-side), and to the `Nemesis` mirror when `id != world.id` (unchanged).
  The own-player branch is new; the remote-player branch is unchanged.
- `handle_packet` on `Action{shoot}`: unchanged (spawn an `enemy_bullet` from the
  shooting remote player's `Nemesis`, if same room).
- `World::update`: no longer calls `self.player.update(...)` for the local
  player. Enemy simulation, `update_enemy_bullets_position`, `check_player_death`,
  camera, and `check_level_transitions` still run (enemies/bullets and room
  transitions remain client-side). Own-bullet spawn/update/collision is
  relocated into the client `World` layer (D6) so shooting still works without
  `Player::update`. `World::update` samples only the `Z` edge
  (`is_key_pressed(KeyCode::Z)`) to spawn the local own bullet; the outgoing
  `PlayerInput` (all four keys) is built and sent in `update_network`.

### Edge cases

- **Edge input not lost between ticks.** Because edges OR-latch until consumed
  (D4) and inputs are reliable-ordered (D2), a `jump` or `shoot` pressed between
  two 20 Hz ticks is applied on the next tick. A `jump` arriving while airborne
  is still ignored by `Player::update` (`if input.jump && self.grounded`) — the
  latch is consumed and cleared regardless (no re-fire on later ticks), matching
  today's edge semantics.
- **Held-flag latest-wins.** Two `PlayerInput`s in one tick interval with
  `move_left` then `move_right` leave `move_right` (the latest) as the held state.
- **Own player position authority.** The client does not move its own player
  between broadcasts; if no `PlayerPos` arrives, the player stands still (no
  extrapolation in 2b).
- **Join ordering.** A new player's first authoritative `PlayerPos` is broadcast
  on the next tick, so existing clients learn of them via the normal
  "create `Nemesis` if unseen" path (no separate spawn packet). Symmetrically, the
  joiner learns existing players' positions on the next tick. The initial
  position is `(100, 100)` on both sides (server `Player::new(id)` and client
  `Player::new(0)` agree), so there is no initial teleport.
- **Room coords are client-trusted.** `level_x`/`level_y` in `PlayerInput` select
  the server's collision `Level`. The server does **not** verify or advance room
  transitions (D7/non-goals). `check_level_transitions` still runs client-side
  (it repositions the player at the entry of the new room and loads that room's
  enemies), but because the server is authoritative for position, the client's
  entry teleport is overwritten by the next authoritative `PlayerPos`, which may
  briefly place the player near the room boundary rather than the entry — a known
  visual glitch accepted for this phase; authoritative transitions fix it later.
- **Disconnect mid-simulation.** On `Leave`/`Timeout`, the player is removed from
  `ServerWorld` so no further `PlayerPos`/`Action` is broadcast for them; peers
  remove the `Nemesis` on `Leave`.
- **No bullet desync from `dir`.** The shoot notification uses the authoritative
  `dir` computed the same tick (which reflects the held `move_left`/`move_right`),
  so remote bullet direction matches the shooter's facing, exactly as today.
- **Client-trusted coords indexing.** `coords` stores `(level_x, level_y)` =
  `(x, y)`, but the level matrix is indexed **`levels[y][x]`** (row-major). The
  server resolves the collision `Level` with `.get(y).and_then(|row| row.get(x))`
  and falls back to `levels[0][0]` — a client-sent bogus `(255,255)` (or any
  out-of-range/missing entry) must not index-panic the server.

## Data model

### `protocol` (`protocol/src/lib.rs`)

Add one variant (existing variants unchanged):

```rust
GamePacket::PlayerInput {
    id: u8,          // owner id (validated against the server's addr→id map)
    level_x: u8,     // room the client is in (grid column)
    level_y: u8,     // room the client is in (grid row)
    move_left: bool, // held
    move_right: bool,// held
    jump: bool,      // pressed (edge)
    shoot: bool,     // pressed (edge)
}
```

`PlayerPos` and `Action` are **unchanged** in shape. `Action` becomes
server→client only (D1).

### `server` (`server/src/main.rs` + `server/src/world.rs`)

`ServerWorld` (revives the dead stub; declared via `mod world;`):

```rust
pub struct ServerWorld {
    players: HashMap<u8, Player>,        // game::entities::player::Player, keyed by id
    pending: HashMap<u8, PlayerInput>,   // game::PlayerInput accumulator (D4)
    coords:  HashMap<u8, (u8, u8)>,      // each player's reported room
    levels:  Vec<Vec<Level>>,            // built once from game::maps::Map::new()
}
impl ServerWorld {
    pub fn new() -> Self;                 // build levels matrix from Map::new()
    pub fn spawn(&mut self, id: u8);      // players.insert(id, Player::new(id)); pending/coords default
    pub fn remove(&mut self, id: u8);     // drop player + pending + coords
    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput);
    pub fn update(&mut self, dt: f32);    // tick: simulate each player, return/emit shoot flags
}
```

The per-player accumulator uses `game::PlayerInput` itself (held fields
overwritten, edge fields OR-ed then cleared). `main.rs` owns
`HashMap<SocketAddr, u8>` (addr → id) and `next_id`, and drives the loop (D3/D8).

The corrected imports in `world.rs`: `game::maps::Map` (not `std::iter::Map`),
`game::entities::player::{Player, PlayerInput}`, `game::level::Level`,
`std::collections::HashMap`. The undefined `PlayerData` and `map: Map` field of
the stub are removed.

### `client` (`client/src/network/world.rs`, `client/src/world/game.rs`)

- `update_network` builds/sends `GamePacket::PlayerInput` (reliable-ordered)
  instead of `PlayerPos`/`Action`.
- `handle_packet` applies `PlayerPos` to the own player when `id == world.id`.
- `World::update` drops the local `self.player.update(...)` call and owns
  own-bullet spawn/update/collision (D6). The keyboard → `PlayerInput` mapping is
  unchanged.

## Network (if any)

- **Added:** `GamePacket::PlayerInput` (client→server, reliable-ordered).
- **Removed (client→server):** `PlayerPos` and `Action` are no longer sent by the
  client.
- **Unchanged (server→client):** `PlayerPos` (unreliable, authoritative, latest
  wins, broadcast to **all** players including the sender), `Action{shoot}`
  (reliable-ordered, now generated by the server, sent to the *other* players —
  the shooter is excluded), `JoinRequest`/`JoinResponse` (reliable-ordered),
  `Leave` (reliable-ordered), `Hit` (reliable-ordered).
- Channel usage matches ADR-001: inputs/actions/join/leave/hit on reliable-ordered,
  position broadcasts on unreliable.

## Acceptance criteria

Each item is a checkable statement of "done"; not an implementation plan.

### Protocol

1. `protocol/src/lib.rs` defines `GamePacket::PlayerInput` with exactly
   `{ id: u8, level_x: u8, level_y: u8, move_left: bool, move_right: bool, jump:
   bool, shoot: bool }`; `PlayerPos` and `Action` are unchanged in shape.
2. A `protocol` round-trip test serializes a representative `PlayerInput` with
   postcard and deserializes it back to an equal value (all bools and bytes
   preserved).
3. `protocol/Cargo.toml` declares **no** dependency on `game` or `glam` (serde
   only); `grep -rn "game" protocol/src` finds no reference to the `game` crate.

### Server tick loop & authority

4. `grep -nE "\b20\b|50\.0|\b50\b|0\.05" server/src` finds **no** tick-rate
   literals; the tick interval and `dt` are derived from `game::config::TICK_HZ`.
5. `server/src/main.rs` uses `try_recv()` (no blocking `recv()`), drains events,
   and runs the tick on an `Instant`-based fixed step at `1.0 / TICK_HZ`.
6. The tick passes `dt = 1.0 / TICK_HZ as f32` (0.05 at 20 Hz) to
   `Player::update` — fixed step, not wall-clock elapsed.
7. `server/src/world.rs` defines `ServerWorld` (with `mod world;` in `main.rs`)
   holding `HashMap<u8, Player>` + the level matrix built from `game::maps::Map`,
   plus `spawn`/`remove`/`apply_input`/`update`; the stub's `std::iter::Map` and
   undefined `PlayerData` are gone.
8. `server/src/main.rs` no longer produces an "unused dependency: `game`" warning
   — the server `use`s `game` via `ServerWorld`/`Player`.
9. On `JoinRequest`, the server creates `Player::new(id)` in `ServerWorld` and
   replies `JoinResponse { assigned_id: id }`; the `SocketAddr ↔ u8` map remains
   in `main.rs`.

### Input buffering & edge semantics

10. A unit test (server-side) shows held flags overwrite (latest `move_left`
    then `move_right` leaves `move_right`) and edge flags latch (two `jump: true`
    inputs then a tick consume exactly one jump and clear the latch).
11. A unit test shows an edge flag received between ticks is not lost: with no
    intervening tick, `shoot: true` accumulates and is observed on the next tick.

### Client

12. `client/src/network/world.rs` sends `GamePacket::PlayerInput`
    (reliable-ordered) each frame and sends **no** `PlayerPos` or `Action`.
13. `handle_packet` applies `PlayerPos` to the own player when `id == world.id`
    (pos + dir) and still mirrors remote players as `Nemesis` otherwise.
14. `World::update` no longer calls `self.player.update(...)` for the local
    player (no self-simulation of movement). The outgoing `PlayerInput` — with the
    unchanged key mapping (`Left`/`Right` held, `Space`/`Z` edge) — is built and
    sent in `update_network` (all four keys); `World::update` samples only the `Z`
    edge to spawn the local own bullet.
15. Own bullets still work: the client spawns/updates/collides its own bullets
    (relocated to the `World` layer) and remote `Action{shoot}` still spawns
    `enemy_bullets` — bullets and enemies remain client-simulated.

### Behavior parity

16. Holding `Right` moves the (authoritative) player right at `VELOCIDAD`,
    `Left` left, `Space` jumps with `SALTO_FUERZA`, gravity is `GRAVEDAD`, and
    direction follows the held key — same traversal as before, now server-driven.
17. `PlayerPos` broadcasts reach **all** connected players each tick (unreliable),
    **including the sender** (each client applies its own echoed `PlayerPos` to its
    own player), and a newly joined player is reflected to existing clients on the
    next tick (via the existing "create `Nemesis` if unseen" path).

### Technical debt / retune

18. Changing `TICK_HZ` from 20 to 60 in `game/src/config.rs` and nothing else
    changes the server tick rate (verified by reading the server code for derived
    `Duration`/`dt`; no other file references a hardcoded tick number).

### Quality gates

19. `cargo build --workspace` succeeds (all four crates).
20. `cargo test --workspace` passes (existing `game`/`protocol` tests green plus
    the new input-buffer and `PlayerInput` round-trip tests; no window/GPU code
    is exercised).
21. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
22. `cargo fmt --check --all` reports no diffs.
