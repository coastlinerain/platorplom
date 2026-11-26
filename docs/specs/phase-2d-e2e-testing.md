# Phase 2d — End-to-End Testing Infrastructure

- Status: implemented
- Date: 2026-11-24
- Branch: `feat/phase-2d-e2e-testing`

## Summary

Establish an in-memory, deterministic end-to-end testing framework that runs the
authoritative server tick loop and one or more headless client prediction loops
inside the same process, communicating through fake loopback channels. The
framework catches multi-client regressions (broadcast, reconciliation, room
transitions, shoot) before they ship.

Phase 2c shipped without any multi-client integration tests. Two regressions were
found during manual 60 Hz testing — remote players invisible (broken broadcast
loop) and players stuck in walls on room transitions (reconciliation rewinding
into wrong coordinate space). Three owner-observed bugs remain:

1. **Room-change glitches** — multiple transition issues beyond the wall-stuck
   fix. The player sometimes teleports to wrong coordinates or lands in a wall
   after a room change.
2. **Remote player movement is smoother than local** — a paradox: the local
   player should feel most responsive (zero-latency prediction), but remote
   players (interpolated at 20 → 60 Hz with lerp) appear smoother. This
   suggests prediction/reconciliation jitter.
3. **Shoot multi-fire** — `is_key_pressed(Z)` triggers multiple bullets per
   press because the server latches the `shoot` bit via OR-accumulation and
   the edge doesn't clear properly between ticks without a per-tick
   consumption guarantee.

This spec covers building the harness and a first test suite of ~35–40 tests
that guard movement, room transitions, multi-player visibility, shoot, and
reconciliation correctness.

## Goals / non-goals

### Goals

- Build a deterministic, in-memory loopback transport so e2e tests exercise the
  real server tick loop + client prediction/reconciliation without UDP or timing
  races.
- Split `client/` and `server/` into lib + thin binary so test code can import
  `World`, `ServerWorld`, prediction, and interpolation.
- Create a new `e2e/` workspace crate with `TestHarness`, `TestServer`, and
  `TestClient` helpers.
- Write a first suite of ~35–40 tests covering: basic movement, room transitions
  (all 4 directions), 2-player visibility, shoot (single + multi-fire regression),
  reconciliation jitter, and key edge cases.
- Run e2e tests in CI (`cargo test -p e2e`).

### Non-goals

- Testing the laminar UDP layer itself (that stays in unit tests or manual QA).
- BDD/cucumber framework — plain `#[test]` functions with a shared harness.
- Rendering or audio tests — the headless client skips macroquad entirely.
- Enemy AI, bullet collision with enemies, or death/respawn flows (follow-up specs).
- External script-based testing (bash/Python).

## D1 — Test harness approach

### Decision: In-memory fake loopback transport

The e2e harness wires server and client code together through `std::sync::mpsc`
channels that carry serialized `GamePacket` bytes. No real UDP sockets, no port
binding, no timing races. Each test tick is deterministic:

```
Client(s) → mpsc → ServerWorld::apply_input → tick → PlayerPos broadcast → mpsc → Client(s)
```

**Advantages:**

- **Deterministic.** Every tick produces the same output given the same inputs.
  No `Instant::now()` dependency in the test path (the harness provides a
  controllable clock for interpolation tests).
- **Fast.** Hundreds of ticks complete in <10 ms — the entire suite stays under
  2 seconds.
- **CI-safe.** No port binding, no firewall issues, no thread-timing flakiness.
- **Exercises real logic.** The same `ServerWorld::update`, `handle_packet`,
  `reconcile`, and `interpolate` code paths that run in production are reached —
  only the laminar transport layer is replaced.
- **Inspectable state.** Tests can read `ServerWorld.players()`,
  `World.other_players`, `World.input_buffer`, etc. after any tick.

**Trade-off:** Does not test laminar serialization, socket binding, or UDP
packet loss/ordering. Those remain the domain of protocol round-trip unit tests
(already present in `protocol/`) and manual QA.

### Architecture

```
┌──────────────────────────────────────────────────────────┐
│ e2e crate                                                │
│                                                          │
│  TestHarness                                             │
│  ├── TestServer (owns ServerWorld + Transport tx/rx)    │
│  ├── Vec<TestClient> (each owns World + Transport tx/rx)│
│  └── Transport (mpsc channels per client→server +       │
│                 server→each client)                      │
│                                                          │
│  Server lib         Client lib                           │
│  ├── ServerWorld    ├── World                            │
│  ├── apply_input    ├── handle_packet                    │
│  └── update         ├── reconcile                        │
│                     ├── interpolate                      │
│                     └── check_level_transitions           │
│                                                          │
│  game lib           protocol lib                         │
│  ├── Player, Level  ├── GamePacket                       │
│  ├── config          └── postcard                        │
│  └── Map                                                 │
└──────────────────────────────────────────────────────────┘
```

The `TestHarness` owns the simulation clock and drives ticks in lockstep:
`harness.tick()` sends queued client inputs to the server, runs one server
tick, distributes resulting `PlayerPos`/`Action`/`Leave` packets back to each
client (broadcast semantics preserved), and advances each client's World by
one frame.

## D2 — Crate location

### Decision: New `e2e/` crate

- **Workspace member:** `e2e/` added to root `Cargo.toml` as a workspace member.
- **Dependencies:** `game`, `protocol`, `client` (lib target), `server` (lib target).
- **No new public dependency crates.** Uses `std::sync::mpsc` for the loopback
  transport (no external channel crate needed).
- **Dev-dependencies only:** The `e2e/` crate itself is a test-only crate
  (contains only `#[test]` functions and harness code in `src/`). It is never
  published or depended on by other crates.

The `e2e/` crate does NOT depend on `macroquad` or `laminar`. The headless
`TestClient` and `TestServer` wrappers import the core game types and
packet-handling functions from the lib targets, which are already free of
rendering/network I/O at the tested abstraction level.

## D3 — Client and server lib/bin split

### Required before e2e tests can link

**Client (`metroidvania-rust`):**

```
client/
├── Cargo.toml          # add [lib] section
├── src/
│   ├── lib.rs           # NEW: re-exports World, handle_packet, reconcile,
│   │                    #   interpolate, check_level_transitions, InputRingBuffer,
│   │                    #   advance_sticky, RemoteInterp, etc.
│   ├── main.rs          # thin: macroquad::main + World::new + loop { update(); draw() }
│   ├── camera.rs
│   ├── hud.rs
│   ├── network/         # unchanged (prediction.rs, interpolation.rs, world.rs, client.rs)
│   ├── render/
│   └── world/           # unchanged (game.rs, lib.rs)
```

The current `client/src/network/client.rs` (`NetworkClient`) is a laminar
wrapper. The e2e harness does NOT use it. Instead, the e2e crate defines its
own `Transport` struct. `NetworkClient` stays in the binary target path;
`World` and packet-handling functions are in the lib target.

The `#[allow(dead_code)]` annotations on `InputRingBuffer::is_empty`/`len` and
`InputEntry::from_game_input` are removed — they were dead only because the
binary crate couldn't see test usage.

**Server (`metroidvania-server`):**

```
server/
├── Cargo.toml          # add [lib] section
├── src/
│   ├── lib.rs           # NEW: re-exports ServerWorld, broadcast_leave
│   ├── main.rs          # thin: Socket::bind + tick loop + event drain
│   └── world.rs         # unchanged (ServerWorld, its tests)
```

The server lib target is minimal — `ServerWorld` is the primary export. The
laminar socket binding, polling thread, and `broadcast_leave` function stay in
the binary target unless the e2e harness needs them (it does not — the harness
reimplements broadcast via mpsc).

## D4 — Test framework

### Decision: Plain `#[test]` + `TestHarness` helper

No new test framework dependency. The test pattern is:

```rust
#[test]
fn player_moves_right_on_server() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_right(p0, 10);  // 10 frames of move_right=true
    h.tick_n(10);

    let pos = h.server_player_pos(p0);
    assert!(pos.x > 100.0); // moved right from spawn
    assert_eq!(pos.room, (0, 0));
}
```

The `TestHarness` struct lives in `e2e/src/harness.rs` and provides:

| Method | Purpose |
|--------|---------|
| `new()` | Creates `ServerWorld`, empty client list, loopback channels |
| `connect_client()` | Returns a client handle, sends JoinRequest, processes JoinResponse |
| `disconnect_client(handle)` | Removes client, triggers Leave broadcast |
| `tick()` | 1 tick: drain inputs → server.update → broadcast → client frames |
| `tick_n(n)` | Calls `tick()` n times |
| `hold_right(h, frames)` | Sets `move_right=true` for `frames` upcoming ticks |
| `hold_left(h, frames)` | Sets `move_left=true` for `frames` |
| `press_jump(h)` | One-frame jump edge (consumed after 1 tick) |
| `press_shoot(h)` | One-frame shoot edge |
| `release_all(h)` | Clears all held inputs |
| `server_player_pos(h)` | Returns `(x, y, dir, level_x, level_y)` for player h |
| `client_player_pos(h)` | Returns the client's local predicted position |
| `remote_player_pos(viewer, target)` | Returns what `viewer` sees for `target` (interpolated or raw) |
| `remote_interp_has_snapshots(viewer, target)` | True if interp ring has entries |
| `client_room(h)` | Returns `(col, row)` for client h |
| `advance_clock(ms)` | Advances the virtual clock (for interpolation lerp tests) |
| `server_shots_this_tick()` | Returns shots emitted by the last server.update |
| `client_bullet_count(h)` | Returns `world.bullets.len()` for client h |
| `client_enemy_bullet_count(h)` | Returns `world.enemy_bullets.len()` |

The harness itself is NOT macroquad-dependent and NOT laminar-dependent.

## D5 — Headless mode

### Decision: TestClient + TestServer wrappers in the e2e crate

**TestServer** wraps `server::ServerWorld` plus:

- A per-client input queue (what `apply_input` will consume each tick)
- An output vector of `(dest_client_handle, GamePacket)` produced each tick
- `tick(dt)` → calls `world.update(dt)`, collects shots, builds `PlayerPos`
  broadcasts for all connected clients, pushes `Action` packets for shots.

**TestClient** wraps a minimal "headless World" that:

- Owns a `World` struct constructed with the real `Map::new()` (but no
  `NetworkClient` — the loopback transport replaces it).
- Receives packets via a channel (the harness pushes them after each server tick).
- On `frame(dt)`: drains received packets through the same `handle_packet`
  path, runs local prediction via `player.update(dt, level, &local_input)`,
  calls `check_level_transitions`.
- Skips all rendering, camera update, HUD, and `macroquad::is_key_pressed`
  calls. Input is injected programmatically by the harness.

The `World` struct currently requires `NetworkClient` in its `new()` constructor
and holds `network: NetworkClient` as a field. To support the headless path
without duplicating World:

- `World::new()` is refactored to accept a trait or enum for the network
  transport. Minimal approach: a `NetworkTransport` enum:

  ```rust
  pub enum NetworkTransport {
      Real(NetworkClient),
      Loopback {
          sender: std::sync::mpsc::Sender<Vec<u8>>,
          receiver: std::sync::mpsc::Receiver<Vec<u8>>,
      },
  }
  ```

  The `update_network` function is refactored to read from `world.network`
  via a `receive_packets()` and `send_packet()` trait method, so the loopback
  variant uses mpsc channels instead of laminar. The trait approach is
  preferred (cleaner, extensible):

  ```rust
  pub trait PacketIO {
      fn try_recv(&self) -> Option<GamePacket>;
      fn send(&self, packet: GamePacket);
  }

  // impl PacketIO for NetworkClient { ... }   // binary target
  // impl PacketIO for LoopbackTransport { ... } // e2e crate
  ```

  `World` holds `Box<dyn PacketIO>` instead of `NetworkClient`.

- `send_join_req` becomes a method on `PacketIO`.
- The real `main.rs` constructs `World::new(map, Box::new(NetworkClient::new(...)))`.
- The e2e `TestClient` constructs `World::new(map, Box::new(LoopbackTransport { tx, rx }))`.

**Scope note:** If the `PacketIO` trait + `Box<dyn PacketIO>` refactor is too
invasive for this phase, an acceptable fallback is: the e2e `TestClient` does
NOT use `World` as-is. Instead it manually wires `handle_packet`, prediction,
and `check_level_transitions` against a simplified state struct. This is less
desirable (code drift risk) but keeps the scope smaller. The spec recommends
the trait approach; the planner will make the final call.

## D6 — First test suite scope

The test suite targets ~35–40 tests organized in 6 groups. Each test is a
`#[test]` function in `e2e/tests/` (or `e2e/src/` with `#[cfg(test)]`).

### Group A — Basic movement (7 tests)

| # | Test | What it verifies |
|---|------|------------------|
| A1 | `hold_right_moves_server_player_right` | Server authoritative position.x > 100 after 10 ticks of move_right |
| A2 | `hold_left_moves_server_player_left` | Server authoritative position.x < 100 after 10 ticks of move_left |
| A3 | `jump_when_grounded_moves_player_up` | Server position.y < 100 after jump (player set grounded before input) |
| A4 | `jump_ignored_when_airborne` | Server position.y > 100 after jump while airborne (gravity dominates) |
| A5 | `no_input_applies_gravity_only` | With no input, y increases due to gravity; x unchanged |
| A6 | `simultaneous_left_right_latest_wins` | Apply move_left then move_right in same tick → player moves right (held overwrite) |
| A7 | `client_predicted_position_tracks_server` | After 10 ticks, `|client.x - server.x| < 5.0` (prediction is close to authority) |

### Group B — Room transitions (9 tests)

| # | Test | What it verifies |
|---|------|------------------|
| B1 | `transition_right_to_room_1_0` | Move right from (0,0): client coords become (1,0), player.x ≈ 10.0 |
| B2 | `transition_left_to_room_0_0` | Move left from (1,0) at x<0: client coords become (0,0), player.x ≈ pixel_width − w − 10 |
| B3 | `transition_down_to_room_0_1` | Fall through bottom of (0,0): coords become (0,1), player.y ≈ 10.0 |
| B4 | `transition_up_to_room_0_0` | Jump through top of (0,1): coords become (0,0), player.y ≈ pixel_height − h − 10 |
| B5 | `player_not_in_wall_after_transition_right` | After B1, `level.colision(player.x, player.y, w, h)` is false (not inside solid) |
| B6 | `player_not_in_wall_after_transition_left` | After B2, not in a wall |
| B7 | `player_not_in_wall_after_transition_down` | After B3, not in a wall |
| B8 | `player_not_in_wall_after_transition_up` | After B4, not in a wall |
| B9 | `transition_at_map_edge_is_noop` | Move right from (1,0) (last column) → coords stay (1,0) or game does not panic |

### Group C — Multi-player visibility (7 tests)

| # | Test | What it verifies |
|---|------|------------------|
| C1 | `two_clients_receive_join_responses` | Both get assigned IDs (1 and 2) after connecting |
| C2 | `client2_sees_client1_moving_right` | Client 1 holds right for 10 ticks → Client 2's `other_players[1].pos.x > 100` |
| C3 | `remote_interp_has_snapshots_after_playerpos` | After 2+ PlayerPos packets, `other_interp[id]` has timestamps and interpolate returns Some |
| C4 | `late_join_client_sees_existing_player` | Client 2 joins after Client 1 has moved → Client 2 immediately receives Client 1's current position |
| C5 | `client_receives_leave_for_disconnected_player` | Client 1 disconnects → Client 2's `other_players` no longer contains ID 1 |
| C6 | `remote_player_in_different_room_not_drawn` | Client 1 moves to room (1,0); Client 2 stays in (0,0) → Client 2's remote player coords filter excludes Client 1 from rendering (test reads `coords != current_coords`) |
| C7 | `broadcast_includes_all_players_to_all_clients` | Server sends PlayerPos for ALL players to ALL connected clients each tick (not just own-echo) |

### Group D — Shoot (6 tests)

| # | Test | What it verifies |
|---|------|------------------|
| D1 | `single_z_press_produces_one_server_shot` | Press shoot once → `server_shots_this_tick()` returns exactly 1 entry |
| D2 | `single_z_press_produces_one_client_bullet` | Press shoot once → `client_bullet_count()` = 1 (not 3 from sticky) |
| D3 | `three_rapid_presses_produce_three_shots` | Press shoot on 3 separate ticks → 3 shots total (not 9) |
| D4 | `shoot_latch_clears_after_consumption` | Press shoot once, tick twice → only the first tick produces a shot |
| D5 | `shoot_direction_matches_player_direction` | Player facing right shoots right; player facing left shoots left |
| D6 | `remote_client_receives_action_packet_for_shot` | Client 1 shoots → Client 2 receives `GamePacket::Action { kind: "shoot", ... }` |

### Group E — Reconciliation jitter / smoothness (6 tests)

| # | Test | What it verifies |
|---|------|------------------|
| E1 | `reconciliation_corrects_position_toward_server` | After a deliberate prediction-server mismatch, reconcile brings `|client.x − server.x|` closer than before |
| E2 | `reconciliation_does_not_produce_large_jumps` | After 60 ticks of normal play, max single-frame position delta < 20.0 px |
| E3 | `stale_playerpos_does_not_trigger_reconciliation` | Server sends `last_seq` that is NOT newer → position unchanged |
| E4 | `first_playerpos_snaps_client_to_server` | Initial `last_acked_seq = u16::MAX`, first PlayerPos sets position to server values |
| E5 | `reconciliation_replay_uses_client_room_collision` | During room transition, replay uses client's current room, not server's stale room coords, and does not panic |
| E6 | `prediction_error_stays_bounded_over_100_ticks` | Run for 100 ticks with movement → `|client.x − server.x|` never exceeds 50.0 px and does not grow unbounded |

### Group F — Edge cases (6 tests)

| # | Test | What it verifies |
|---|------|------------------|
| F1 | `server_handles_bogus_coords_without_panic` | Client reports (255, 255) → server falls back to origin level, no panic |
| F2 | `player_at_room_corner_not_stuck` | Player at (0, 0) near room origin corner can still move |
| F3 | `two_players_same_room_bullets_dont_collide_shooter` | Player 1's bullet does NOT kill Player 1 (owner check) |
| F4 | `max_players_eight_no_panic` | Connect 8 clients → server handles all without panic or slowdown |
| F5 | `disconnect_during_room_transition_no_panic` | Client disconnects mid-transition → server removes without panic |
| F6 | `zero_length_input_buffer_replay_is_noop` | Reconcile with empty input buffer → player snapped to server position, no crash |

### Owner-observed bug regression tests (covered by groups above)

| Bug | Covered by | Rationale |
|-----|-----------|-----------|
| Remote players invisible (broken broadcast loop) | C2, C7 | C2 verifies visibility; C7 verifies all-player broadcast |
| Players stuck in walls on room transitions | B5–B8, E5 | Verifies not-in-wall after every direction; E5 verifies replay uses correct room |
| Room-change glitches (other transition bugs) | B1–B4, F2 | Exact entry coordinates for all 4 directions |
| Remote movement smoother than local (prediction jitter) | E1, E2, E6 | Measures prediction error, max single-frame delta, bounded growth |
| Shoot multi-fire | D1–D4 | Single press = single shot; 3 presses = 3 shots; latch clears |

## Data model

### Workspace changes

```
[workspace]
members = ["server", "client", "protocol", "game", "e2e"]  # add e2e
```

### New crate: `e2e/`

```
e2e/
├── Cargo.toml
└── src/
    ├── lib.rs              # empty or re-exports
    ├── harness.rs          # TestHarness, TestServer, TestClient
    ├── transport.rs        # PacketIO trait, LoopbackTransport
    └── tests/              # test modules (or inline #[cfg(test)])
        ├── movement.rs
        ├── rooms.rs
        ├── multiplayer.rs
        ├── shoot.rs
        ├── reconciliation.rs
        └── edge_cases.rs
```

### `e2e/Cargo.toml`

```toml
[package]
name = "e2e"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
game = { path = "../game" }
protocol = { path = "../protocol" }
client = { path = "../client" }
server = { path = "../server" }
postcard = { version = "1.1.3", features = ["alloc"] }

[lints]
workspace = true
```

No `macroquad`, `laminar`, or `crossbeam-channel` dependencies.

### Key types in the test harness

```rust
/// Unique handle for a connected test client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientHandle(u8);

/// Input queued for the next tick.
#[derive(Debug, Default, Clone)]
pub struct QueuedInput {
    pub move_left: bool,
    pub move_right: bool,
    pub jump: bool,
    pub shoot: bool,
}

/// The in-memory loopback transport.
///
/// Each client has a dedicated sender to the server and receiver from the
/// server. The harness routes packets to simulate broadcast semantics
/// (own-echo + cross-client visibility).
pub struct Transport {
    /// client → server: one sender per client
    pub to_server: Vec<std::sync::mpsc::Sender<Vec<u8>>>,
    /// server → each client: one sender per client
    pub to_clients: Vec<std::sync::mpsc::Sender<Vec<u8>>>,
    /// Corresponding receivers owned by the server side.
    pub server_rx: std::sync::mpsc::Receiver<(ClientHandle, Vec<u8>)>,
    /// Receivers owned by each client.
    pub client_rxs: Vec<std::sync::mpsc::Receiver<Vec<u8>>>,
}

/// Owns the authoritative simulation and routes packets.
pub struct TestServer {
    pub world: server::ServerWorld,
    pub transport: Transport,
    /// Maps ClientHandle → internal player id.
    pub clients: HashMap<ClientHandle, u8>,
    pub next_handle: u8,
    pub next_player_id: u8,
    /// Shots emitted during the last tick (for assertions).
    pub last_shots: Vec<(u8, f32)>,
}

/// Headless client wrapper for a single player.
pub struct TestClient {
    pub handle: ClientHandle,
    pub world: client::World,
    pub player_id: Option<u8>,
    pub queued_input: QueuedInput,
}

/// Top-level test orchestrator.
pub struct TestHarness {
    pub server: TestServer,
    pub clients: Vec<TestClient>,
    pub clock: VirtualClock,
}
```

### `PacketIO` trait (added to `client/` lib)

```rust
/// Abstracts network I/O so the same World can run against real laminar or a
/// loopback transport.
pub trait PacketIO {
    /// Try to receive one GamePacket. Returns None if no packet is available.
    fn try_recv(&self) -> Option<GamePacket>;

    /// Send one GamePacket to the server (or loopback channel).
    fn send(&self, packet: GamePacket);
}

/// The real laminar transport (in the binary target).
pub struct RealPacketIO {
    pub sender: crossbeam_channel::Sender<laminar::Packet>,
    pub receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    pub server_addr: std::net::SocketAddr,
}
impl PacketIO for RealPacketIO { ... }
```

`World` changes from holding `network: NetworkClient` to holding
`network: Box<dyn PacketIO>`. `update_network` uses `world.network.try_recv()`
instead of the laminar-specific `world.network.receiver.try_recv()`. This is
a zero-cost abstraction in practice (the real path still dispatches through
the same laminar channel — the trait method is statically dispatched via
monomorphization in the binary, and dynamically dispatched only in tests).

### `VirtualClock` (for deterministic interpolation tests)

```rust
/// Controllable monotonic clock for tests.
///
/// Defaults to epoch 0. Each `tick()` advances by 16_667 µs (~60 Hz frame).
/// `interpolate_at` accepts an `Instant`, so we provide one via
/// `clock.now()` that returns a fixed-offset Instant.
pub struct VirtualClock {
    /// Microseconds since epoch.
    us: u64,
    /// The `Instant` anchor: now = anchor + Duration::from_micros(us).
    anchor: std::time::Instant,
}
```

## Network (if any)

No new network packets. The loopback transport carries the existing `GamePacket`
enum via postcard serialization over `mpsc::channel<Vec<u8>>`. The server's
broadcast semantics are preserved:

1. Server tick produces `PlayerPos` for EVERY player → pushed to EVERY client's inbound channel (own-echo included).
2. Shoot `Action` packets go to all clients EXCEPT the shooter.
3. `Leave` packets go to all remaining clients.
4. `JoinResponse` goes only to the joining client.

## Acceptance criteria

### Infrastructure acceptance

1. **AC-H1:** `cargo test -p e2e` compiles and runs without errors.
2. **AC-H2:** `cargo test -p client` still passes all existing 73 unit tests.
3. **AC-H3:** `cargo test -p server` still passes all existing tests.
4. **AC-H4:** `cargo test --workspace` passes everything (including e2e).
5. **AC-H5:** `client/` has a `[lib]` target; `client/src/main.rs` is a thin
   wrapper (<40 lines excluding the `macroquad::main` attribute).
6. **AC-H6:** `server/` has a `[lib]` target; `server/src/main.rs` is a thin
   wrapper.
7. **AC-H7:** `World` holds `Box<dyn PacketIO>`, not `NetworkClient`.
8. **AC-H8:** The `#[allow(dead_code)]` annotations on `InputRingBuffer::is_empty`,
   `InputRingBuffer::len`, and `InputEntry::from_game_input` are removed.
9. **AC-H9:** `cargo clippy --workspace -- -D warnings` passes with zero warnings.
10. **AC-H10:** `cargo fmt --check` passes across the workspace.
11. **AC-H11:** CI workflow (`cargo test --workspace`) includes the e2e tests.

### Test suite acceptance

12. **AC-A1–A7:** All Group A (movement) tests pass.
13. **AC-B1–B9:** All Group B (room transitions) tests pass.
14. **AC-C1–C7:** All Group C (multi-player visibility) tests pass.
15. **AC-D1–D6:** All Group D (shoot) tests pass.
16. **AC-E1–E6:** All Group E (reconciliation jitter) tests pass.
17. **AC-F1–F6:** All Group F (edge cases) tests pass.

### Bug regression acceptance

18. **AC-BUG-1:** Test C2 (remote visibility) fails if the broadcast loop skips
    any connected player (catches the "remote players invisible" regression).
19. **AC-BUG-2:** Tests B5–B8 (not-in-wall after transition) fail if room
    transitions place the player inside a solid tile.
20. **AC-BUG-3:** Tests D1–D4 fail if a single keypress produces more than one
    bullet (catches the shoot multi-fire regression).
21. **AC-BUG-4:** Tests E2 and E6 fail if local prediction error grows unbounded
    or produces large single-frame jumps (catches the "remote smoother than
    local" paradox — the test asserts local prediction is smooth and bounded).

## Design decision summary

| Decision | Choice | Rationale |
|----------|--------|-----------|
| D1 — Harness | In-memory loopback (mpsc) | Deterministic, fast, CI-safe, exercises real logic |
| D2 — Crate | New `e2e/` workspace member | Clean separation, clear dependency graph |
| D3 — Lib split | Both client and server | Required so e2e crate can import World and ServerWorld |
| D4 — Framework | Plain `#[test]` + `TestHarness` | Consistent with existing 73 tests, no new deps |
| D5 — Headless | `PacketIO` trait + `Box<dyn PacketIO>` in World | Reuses real logic, minimal code duplication |
| D6 — Scope | ~35–40 tests in 6 groups | Covers all owner-requested scenarios plus edge cases |
| D7 — CI | Yes, `cargo test -p e2e` in CI | No port binding or timing concerns with in-memory harness |

## D7 — CI integration

The e2e tests run in CI as `cargo test -p e2e` (or as part of
`cargo test --workspace`). No special setup is required:

- **No port binding.** The in-memory loopback uses mpsc channels only.
- **No timing races.** The virtual clock is deterministic.
- **Expected runtime.** ~35–40 tests completing in <2 seconds on CI hardware.
- **No new CI dependencies.** The existing `actions-rs/toolchain` + `cargo test`
  workflow already covers it.

The e2e crate is NOT gated behind a feature flag — it always builds and tests
as part of the workspace. Since it has no heavy dependencies (no macroquad, no
laminar), compilation overhead is negligible.