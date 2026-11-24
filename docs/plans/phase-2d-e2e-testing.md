# Phase 2d — End-to-End Testing Infrastructure — Implementation Plan

- Status: draft
- Spec: `docs/specs/phase-2d-e2e-testing.md`
- Date: 2026-11-24
- Branch: `feat/phase-2d-e2e-testing`

## Overview

Phase 2c shipped without multi-client integration tests, and two regressions were
found during manual 60 Hz testing (remote players invisible due to a broken
broadcast loop; players stuck in walls on room transitions). Three
owner-observed bugs remain.

This phase builds a deterministic, in-memory end-to-end testing framework so the
real server tick loop and client prediction/reconciliation can be exercised
inside a single `cargo test` process — no UDP, no timing races.

To make this possible, the **`client/` and `server/` crates are split into lib +
thin binary** targets. A new `PacketIO` trait abstracts network I/O so `World`
holds `Box<dyn PacketIO>` instead of `NetworkClient`. A new `e2e/` workspace
crate provides `TestHarness`, `TestServer`, `TestClient`, `LoopbackTransport`,
and `VirtualClock` helpers plus ~41 tests across 6 groups.

All 7 architectural decisions (D1–D7) are resolved in the spec; nothing is left
open for the implementer to decide. Seven additional design decisions are
documented in the implementation notes below (notes 8–14).

## Public API (targets for the tester)

These are the exact signatures the implementer must produce. The tester writes
tests against the `e2e/` crate's `TestHarness` public API (section 2) and the
existing unit test suites must keep passing (regression gate).

---

### 1. `PacketIO` trait (in `client/src/network/packet_io.rs` or `client/src/lib.rs`)

```rust
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
```

`try_recv` and `send` take `&mut self` because the real transport may need to
mutate internal buffer state (laminar's `Receiver` consumes on `try_recv`), and
the loopback transport consumes from the `mpsc::Receiver`.

### 2. `RealPacketIO` (binary-only; lives in `client/src/main.rs` or a new `client/src/network/real.rs`)

```rust
pub struct RealPacketIO {
    pub sender: crossbeam_channel::Sender<laminar::Packet>,
    pub receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    pub server_addr: std::net::SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<GamePacket> { /* drain laminar */ }

    /// Variant-aware send: JoinRequest and Hit use reliable_ordered (they are
    /// critical setup/teardown messages that must not be lost). All other
    /// packets use unreliable (movement, shoot, leave — fire-and-forget with
    /// redundancy via sticky edges and periodic broadcasts).
    /// See design decision note 8 below.
    fn send(&mut self, packet: GamePacket) {
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let laminar_packet = match &packet {
            GamePacket::JoinRequest | GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes)
            }
            _ => {
                laminar::Packet::unreliable(self.server_addr, bytes)
            }
        };
        let _ = self.sender.send(laminar_packet);
    }
}
```

### 3. `LoopbackTransport` (in `e2e/src/transport.rs`)

```rust
/// An in-memory PacketIO implementation backed by mpsc channels.
/// Carries serialized `Vec<u8>` (postcard) so the same serialization path is
/// exercised as in production — only laminar is replaced.
pub struct LoopbackTransport {
    /// Packets *from* the server arrive here (read by the client).
    pub rx: std::sync::mpsc::Receiver<Vec<u8>>,
    /// Packets *to* the server are sent here (read by the harness' server side).
    pub tx: std::sync::mpsc::Sender<Vec<u8>>,
}

impl client::PacketIO for LoopbackTransport {
    fn try_recv(&mut self) -> Option<GamePacket> { /* rx.try_recv() → postcard */ }
    fn send(&mut self, packet: GamePacket)       { /* postcard → tx.send() */ }
}
```

### 4. `World` changes (in `client/src/world/game.rs`)

```rust
pub struct World {
    // ... existing fields unchanged EXCEPT:

    // BEFORE (removed):
    // pub network: crate::network::client::NetworkClient,

    // AFTER:
    pub network: Box<dyn crate::network::PacketIO>,

    // NEW fields — programmatic input injection (set by World::update in binary
    // or by TestClient in e2e):
    pub move_left:  bool,
    pub move_right: bool,
}

impl World {
    // BEFORE:
    // pub fn new(matrix_data: Map, network: NetworkClient) -> Self

    // AFTER:
    pub fn new(matrix_data: Map, network: Box<dyn PacketIO>) -> Self
}
```

The `send_join_req` function signature changes from `&NetworkClient` to
`&mut dyn PacketIO`:

```rust
pub fn send_join_req(network: &mut dyn PacketIO);
```

### 5. `update_network` refactoring (in `client/src/network/world.rs`)

`update_network` must no longer call `macroquad` key functions directly.
Instead it reads `world.move_left`, `world.move_right`, `world.pressed_jump`,
`world.pressed_shoot` — all set by `World::update` before `update_network` is
called.

```rust
pub fn update_network(world: &mut World) {
    // Drain: non-blocking read from PacketIO trait.
    while let Some(decoded) = world.network.try_recv() {
        handle_packet(world, decoded);
    }

    if let Some(id) = world.id {
        // Sticky edge handling (unchanged).
        let (jump, jump_sticky) = prediction::advance_sticky(world.jump_sticky, world.pressed_jump);
        let (shoot, shoot_sticky) = prediction::advance_sticky(world.shoot_sticky, world.pressed_shoot);
        world.jump_sticky = jump_sticky;
        world.shoot_sticky = shoot_sticky;

        world.input_seq = world.input_seq.wrapping_add(1);
        let seq = world.input_seq;

        let input_msg = GamePacket::PlayerInput {
            id,
            level_x: world.current_coords.0 as u8,
            level_y: world.current_coords.1 as u8,
            move_left: world.move_left,
            move_right: world.move_right,
            jump,
            shoot,
            seq,
        };

        world.input_buffer.push(InputEntry {
            seq,
            move_left: world.move_left,
            move_right: world.move_right,
            jump,
        });

        world.network.send(input_msg);
    }
}
```

`handle_packet` becomes `pub` (currently `fn`, private to `world.rs`):

```rust
pub fn handle_packet(world: &mut World, decoded: GamePacket) { ... }
```

### 6. `World::update` changes

`World::update` now samples `is_key_down(Left)` / `is_key_down(Right)` and
stores them in `self.move_left` / `self.move_right` so `update_network` can
read them without calling macroquad:

```rust
pub fn update(&mut self, dt: f32) {
    check_level_transitions(self);
    update_enemy_bullets_position(self, dt);
    check_player_death(self);
    {
        let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];

        // Sample edge keys ONCE.
        let pressed_jump = is_key_pressed(KeyCode::Space);
        let pressed_shoot = is_key_pressed(KeyCode::Z);

        // Sample held keys — stored on World so update_network can read them.
        self.move_left  = is_key_down(KeyCode::Left);
        self.move_right = is_key_down(KeyCode::Right);

        // Local prediction (unchanged).
        let local_input = game::entities::player::PlayerInput {
            move_left:  self.move_left,
            move_right: self.move_right,
            jump:       pressed_jump,
            shoot:      false,
        };
        self.player.update(dt, level, &local_input);

        self.pressed_jump  = pressed_jump;
        self.pressed_shoot = pressed_shoot;

        // Bullet spawn & update (unchanged).
        if pressed_shoot { /* spawn bullet */ }
        for bullet in &mut self.bullets { bullet.update(dt, level); }
        for bullet in &mut self.bullets { /* collide with enemies */ }
        for enemy in &mut self.enemies { /* client-side update */ }

        self.camera.update(self.player.pos, vec2(self.player.w, self.player.h), level);
    }
    update_network(self);

    self.bullets.retain(|b| b.active);
    self.enemies.retain(|e| e.alive);
}
```

**`handle_death` refactoring:** `client/src/world/lib.rs:34-44` currently
accesses `world.network.sender.send(laminar::Packet::reliable_ordered(...))`.
After the refactor, it must call `world.network.send(GamePacket::Hit { entity_id: my_id })`
via the `PacketIO` trait instead:

```rust
fn handle_death(world: &mut World) {
    if let Some(my_id) = world.id {
        world.network.send(GamePacket::Hit { entity_id: my_id });
    }
    println!("You died! Returning to start...");
    world.current_coords = (0, 0);
    world.player.pos = vec2(100.0, 100.0);
    load_current_level(world);
}
```

### 7. Client lib target (`client/src/lib.rs`)

```rust
// Re-export everything the e2e crate needs.
pub mod network;
pub mod world;

// Re-export key types at crate root for ergonomic use:
pub use network::packet_io::PacketIO;
pub use network::world::{handle_packet, send_join_req, update_network};
pub use world::game::World;
pub use world::lib::check_level_transitions;
```

The `network` module gains a new submodule:

```
client/src/network/
├── client.rs           # NetworkClient (unchanged, binary-only usage)
├── interpolation.rs    # RemoteInterp (unchanged)
├── mod.rs              # pub mod packet_io; (NEW entry)
├── packet_io.rs        # NEW: PacketIO trait definition
├── prediction.rs       # InputRingBuffer, InputEntry (unchanged)
└── world.rs            # update_network, handle_packet, send_join_req (refactored)
```

Note: the `client::world::lib` module's `handle_death` function (which also calls
`world.network.sender`) is NOT re-exported — the e2e harness does not call it
directly. It is only refactored internally to use `PacketIO::send`.

### 8. Client binary target (`client/src/main.rs` — thin wrapper)

```rust
mod camera;
mod hud;
mod render;

use metroidvania_rust::network::PacketIO;
use metroidvania_rust::world::game::World;
use metroidvania_rust::network::world::send_join_req;
use game::maps::Map;
use macroquad::prelude::*;
use std::net::SocketAddr;

// RealPacketIO stays in the binary target.
struct RealPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                postcard::from_bytes(p.payload()).ok()
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            protocol::GamePacket::JoinRequest | protocol::GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes)
            }
            _ => {
                laminar::Packet::unreliable(self.server_addr, bytes)
            }
        };
        let _ = self.sender.send(pkt);
    }
}

#[macroquad::main("Rustvania")]
async fn main() {
    // Same laminar setup as before — now wrapped in RealPacketIO.
    let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
    let sender = socket.get_packet_sender();
    let receiver = socket.get_event_receiver();
    let server_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    std::thread::spawn(move || socket.start_polling());

    let io = RealPacketIO { sender, receiver, server_addr };
    let mut boxed: Box<dyn PacketIO> = Box::new(io);
    send_join_req(&mut *boxed);

    let world_data = Map::new();
    let mut game_world = World::new(world_data, boxed);

    loop {
        if is_key_pressed(KeyCode::P) {
            println!("{:#?}", game_world);
        }
        clear_background(BLACK);
        game_world.update(get_frame_time());
        game_world.draw();
        next_frame().await
    }
}
```

### 9. Server lib target (`server/src/lib.rs`)

```rust
mod world;
pub use world::ServerWorld;
```

`server/src/world.rs` is unchanged except:
- Its `mod world;` declaration moves to `lib.rs`.
- The binary `main.rs` changes `mod world; use world::ServerWorld;` to
  `use metroidvania_server::ServerWorld;`.
- **New public accessor for the e2e harness:** `ServerWorld` gains
  `pub fn players_mut(&mut self) -> &mut HashMap<u8, Player>` so that
  `TestHarness` can set `player.grounded` for jump tests (see note 9).

### 10. New crate: `e2e/`

```
e2e/
├── Cargo.toml
└── src/
    ├── lib.rs              # empty (or re-exports for clarity)
    ├── harness.rs          # TestHarness, TestServer, TestClient, Transport
    ├── transport.rs        # LoopbackTransport
```

**`e2e/Cargo.toml`:**

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

**Key types in `e2e/src/harness.rs`:**

```rust
use std::collections::HashMap;

/// Opaque handle for a connected test client.
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

/// Owns the authoritative simulation and routes packets via loopback channels.
pub struct TestServer {
    pub world: server::ServerWorld,
    /// client→server receivers (indexed by ClientHandle.0).
    pub from_clients: Vec<std::sync::mpsc::Receiver<Vec<u8>>>,
    /// server→client senders (indexed by ClientHandle.0).
    pub to_clients: Vec<std::sync::mpsc::Sender<Vec<u8>>>,
    /// Maps ClientHandle → internal player id.
    pub clients: HashMap<ClientHandle, u8>,
    pub next_handle: u8,
    pub next_player_id: u8,
    /// Shots emitted during the last tick (for assertions).
    pub last_shots: Vec<(u8, f32)>,
    /// PlayerPos broadcasts from the last tick: (dest_handle, player_id, x, y).
    pub last_broadcasts: Vec<(ClientHandle, u8, f32, f32)>,
    /// Action packets from the last tick.
    pub last_actions: Vec<(ClientHandle, String, f32)>,
    pub last_leaves: Vec<(ClientHandle, u8)>,
}

/// Headless client wrapper for a single player.
pub struct TestClient {
    pub handle: ClientHandle,
    pub world: client::World,
    pub player_id: Option<u8>,
    pub queued_input: QueuedInput,
}

/// Controllable clock for deterministic interpolation tests.
pub struct VirtualClock {
    pub us: u64,
    pub anchor: std::time::Instant,
}

impl VirtualClock {
    pub fn new() -> Self { Self { us: 0, anchor: std::time::Instant::now() } }
    pub fn now(&self) -> std::time::Instant {
        self.anchor + std::time::Duration::from_micros(self.us)
    }
    pub fn advance(&mut self, ms: u64) { self.us += ms * 1000; }
}

/// Top-level test orchestrator.
pub struct TestHarness {
    pub server: TestServer,
    pub clients: Vec<TestClient>,
    pub clock: VirtualClock,
}
```

**`TestHarness` public API (used in every test):**

```rust
impl TestHarness {
    /// Creates ServerWorld, empty client list, VirtualClock at epoch 0.
    pub fn new() -> Self;

    /// Connects a new client: creates mpsc channels, builds a TestClient with a
    /// LoopbackTransport-wrapped World, sends JoinRequest, processes the
    /// JoinResponse, and returns the ClientHandle.
    pub fn connect_client(&mut self) -> ClientHandle;

    /// Disconnects a client: removes from server state, broadcasts Leave, drops
    /// channels. Panics if handle is not found.
    pub fn disconnect_client(&mut self, handle: ClientHandle);

    /// Drain queued inputs → server.update() → broadcast PlayerPos/Action/Leave
    /// → each client drains its inbound channel through handle_packet.
    /// Advances VirtualClock by 16_667 µs (~60 Hz).
    pub fn tick(&mut self);

    /// Calls `tick()` n times.
    pub fn tick_n(&mut self, n: u32);

    // --- Input queuing (sets QueuedInput for the given client) ---
    pub fn hold_right(&mut self, handle: ClientHandle, frames: u32);
    pub fn hold_left(&mut self, handle: ClientHandle, frames: u32);
    pub fn press_jump(&mut self, handle: ClientHandle);
    pub fn press_shoot(&mut self, handle: ClientHandle);
    pub fn release_all(&mut self, handle: ClientHandle);

    // --- State manipulation (needed by specific test groups) ---
    /// Sets the server-side player position. Used by room transition tests (B1-B4)
    /// to position the player at the correct Y-coordinate for exit gaps, and by
    /// movement tests that need a known starting position.
    pub fn set_server_player_pos(&mut self, handle: ClientHandle, x: f32, y: f32);

    /// Sets `player.grounded` on the server-side player. Required by jump tests
    /// (A3, A4) because the spawn position (100,100) is over empty tiles — the
    /// player falls for ~330 ticks before touching ground. See note 9.
    pub fn set_server_player_grounded(&mut self, handle: ClientHandle, grounded: bool);

    /// Sets the client-side predicted position. Used by reconciliation test E1
    /// to create a deliberate prediction-server mismatch. Directly modifies
    /// `self.clients[handle.0].world.player.pos`.
    pub fn set_client_player_pos(&mut self, handle: ClientHandle, x: f32, y: f32);

    // --- State inspection ---
    pub fn server_player_pos(&self, handle: ClientHandle) -> (f32, f32, f32, u8, u8);
    // Returns (x, y, dir, level_x, level_y) from ServerWorld.

    pub fn client_player_pos(&self, handle: ClientHandle) -> (f32, f32);
    // Returns the client World's player.pos (predicted position).

    pub fn client_room(&self, handle: ClientHandle) -> (usize, usize);
    // Returns client World's current_coords.

    pub fn remote_player_pos(&self, viewer: ClientHandle, target: ClientHandle) -> Option<(f32, f32)>;
    // Returns what `viewer` sees for `target` — either the interpolated position
    // (if RemoteInterp has snapshots) or the raw Nemesis.pos.

    pub fn remote_interp_has_snapshots(&self, viewer: ClientHandle, target: ClientHandle) -> bool;

    pub fn client_bullet_count(&self, handle: ClientHandle) -> usize;
    pub fn client_enemy_bullet_count(&self, handle: ClientHandle) -> usize;

    pub fn server_shots_this_tick(&self) -> &[(u8, f32)];

    pub fn advance_clock(&mut self, ms: u64);
    pub fn clock_now(&self) -> std::time::Instant;
}
```

**Tick logic (what `harness.tick()` does — implementer must follow exactly):**

```
1. For each TestClient c:
   a. Read c.queued_input:
      - If hold_right frames > 0: set world.move_right = true; decrement frames
      - If hold_left  frames > 0: set world.move_left  = true; decrement frames
      - If jump queued: set world.pressed_jump = true (one-frame edge)
      - If shoot queued: set world.pressed_shoot = true (one-frame edge)
   b. Clear one-shot flags from queued_input (jump, shoot)
2. Drain each client→server channel; for each Vec<u8> received:
   a. Deserialize GamePacket
   b. If PlayerInput: call server.world.apply_input(...)
3. Call server.world.update(tick_dt):
   a. Store returned shots in server.last_shots
4. Broadcast PlayerPos: for each destination client d and each player p:
   a. Build GamePacket::PlayerPos { id, x, y, dir, level_x, level_y, last_seq }
   b. Serialize, push to to_clients[d].send()
5. Broadcast Action: for each shot (shooter_id, dir) and each client d ≠ shooter:
   a. Build GamePacket::Action { id: shooter_id, kind: "shoot", dir }
   b. Serialize, push to to_clients[d].send()
6. Broadcast Leave (if any disconnected this tick): for each remaining client d:
   a. Build GamePacket::Leave { id: disconnected_id }
   b. Serialize, push to to_clients[d].send()
7. For each TestClient c:
   a. Drain server→client channel: deserialize each Vec<u8> → GamePacket
   b. Call handle_packet(&mut c.world, packet)
   c. Build local input from c.world.move_left/right, c.world.pressed_jump/shoot
   d. Run c.world.player.update(dt, level, &local_input)
   e. Call check_level_transitions(&mut c.world)
   f. Update c.world.bullets, c.world.enemy_bullets, etc.
8. Advance clock by 16_667 µs
```

### 11. Root workspace `Cargo.toml` change

```toml
[workspace]
members = ["server", "client", "protocol", "game", "e2e"]
resolver = "3"
```

### 12. Dead code removal

Three annotations are removed:

| Location | Annotation | Reason |
|---|---|---|
| `client/src/network/prediction.rs:16` | `#[expect(dead_code)]` on `InputEntry::from_game_input` | Now used by e2e tests |
| `client/src/network/prediction.rs:85` | `#[allow(dead_code)]` on `InputRingBuffer::is_empty` | Now used by e2e tests |
| `client/src/network/prediction.rs:90` | `#[allow(dead_code)]` on `InputRingBuffer::len` | Now used by e2e tests |

---

## Test cases (tester writes these first)

The tester writes 41 `#[test]` functions across 6 modules in `e2e/tests/`
(Rust integration-test convention). Each function creates a `TestHarness`,
connects clients, queues inputs, calls `tick_n()`, and asserts against the
harness inspection methods.

### Group A — Basic movement (7 tests)

**File: `e2e/tests/movement.rs`**

| # | Test name | What it asserts |
|---|---|---|
| A1 | `hold_right_moves_server_player_right` | `h = new(); p0 = connect_client(); hold_right(p0, 10); tick_n(10);` → `server_player_pos(p0).0 > 100.0` |
| A2 | `hold_left_moves_server_player_left` | `hold_left(p0, 10); tick_n(10);` → `server_player_pos(p0).0 < 100.0` |
| A3 | `jump_when_grounded_moves_player_up` | `set_server_player_grounded(p0, true); press_jump(p0); tick_n(2);` → `server_player_pos(p0).1 < 100.0` |
| A4 | `jump_ignored_when_airborne` | Player starts airborne (grounded=false, default). `press_jump(p0); tick_n(10);` → `pos.y > 100.0` (gravity dominates; jump ignored) |
| A5 | `no_input_applies_gravity_only` | `tick_n(10);` → `y` increases (gravity); `x` unchanged from 100.0 |
| A6 | `simultaneous_left_right_latest_wins` | Queue move_left then move_right on same tick. After `tick_n(1)` → player moves right (held overwrite wins) |
| A7 | `client_predicted_position_tracks_server` | `hold_right(p0, 10); tick_n(10);` → `abs(client_player_pos(p0).0 - server_player_pos(p0).0) < 5.0` |

### Group B — Room transitions (9 tests)

**File: `e2e/tests/rooms.rs`**

Room transition gap coordinates (from `game/src/maps.rs`):

| Transition | Source room | Dest room | Gap location | Required player Y |
|---|---|---|---|---|
| Right | (0,0) | (1,0) | Tile row 8, exit-right gap at x=795 | y ≈ 320 (tile rows 8–9) |
| Left | (1,0) | (0,0) | Tile row 8, exit-left gap at x=−5 | y ≈ 320 |
| Down | (0,0) | (0,1) | Tile cols 7–8, exit-down gap at y=475 | x ≈ 280–320 |
| Up | (0,1) | (0,0) | Tile cols 7–8, exit-up gap at y=−5 | x ≈ 280–320 |

Tests use `set_server_player_pos` to position the player at the correct Y (or X)
before queuing horizontal (or vertical) movement:

| # | Test name | What it asserts |
|---|---|---|
| B1 | `transition_right_to_room_1_0` | `set_server_player_pos(p0, 790.0, 320.0); hold_right(p0, 5); tick_n(5);` → `client_room(p0) == (1,0)`, `client_player_pos(p0).0 ≈ 10.0` |
| B2 | `transition_left_to_room_0_0` | From (1,0) at x≈0, `hold_left(p0, 5); tick_n(5);` → `client_room(p0) == (0,0)`, player.x ≈ level0_pixel_width − w − 10 |
| B3 | `transition_down_to_room_0_1` | From (0,0), `set_server_player_pos(p0, 300.0, 470.0); tick_n(10);` (gravity drops through) → `client_room(p0) == (0,1)`, player.y ≈ 10.0 |
| B4 | `transition_up_to_room_0_0` | From (0,1), `set_server_player_pos(p0, 300.0, 5.0); set_server_player_grounded(p0, true); press_jump(p0); tick_n(2);` → `client_room(p0) == (0,0)`, player.y ≈ level1_pixel_height − h − 10 |
| B5 | `player_not_in_wall_after_transition_right` | After B1, `level.colision(x, y, w, h)` is false |
| B6 | `player_not_in_wall_after_transition_left` | After B2, not in a wall |
| B7 | `player_not_in_wall_after_transition_down` | After B3, not in a wall |
| B8 | `player_not_in_wall_after_transition_up` | After B4, not in a wall |
| B9 | `transition_at_map_edge_is_noop` | Move right from (1,0) (last column) → coords stay (1,0), no panic |

### Group C — Multi-player visibility (7 tests)

**File: `e2e/tests/multiplayer.rs`**

| # | Test name | What it asserts |
|---|---|---|
| C1 | `two_clients_receive_join_responses` | `p0 = connect_client(); p1 = connect_client();` → `p0.id = Some(1)`, `p1.id = Some(2)` |
| C2 | `client2_sees_client1_moving_right` | Client 1 holds right 10 ticks → Client 2's `other_players[1].pos.x > 100.0` |
| C3 | `remote_interp_has_snapshots_after_playerpos` | After 2+ ticks, `remote_interp_has_snapshots(p1, p0)` is true and `remote_player_pos(p1, p0)` is `Some` |
| C4 | `late_join_client_sees_existing_player` | p0 moves, then p1 joins → p1's `other_players` contains p0's position |
| C5 | `client_receives_leave_for_disconnected_player` | p0 connects, p1 connects, `disconnect_client(p0)` → p1's `other_players` no longer contains p0's id |
| C6 | `remote_player_in_different_room_not_in_other_players_position` | p0 transitions to (1,0); p1 stays in (0,0). p1's `other_players[p0].coords != (0,0)` — p1 sees different coords |
| C7 | `broadcast_includes_all_players_to_all_clients` | After 1 tick with 2 clients, `last_broadcasts` contains PlayerPos for both players to both clients (4 entries) |

### Group D — Shoot (6 tests)

**File: `e2e/tests/shoot.rs`**

| # | Test name | What it asserts |
|---|---|---|
| D1 | `single_z_press_produces_one_server_shot` | `press_shoot(p0); tick_n(1);` → `server_shots_this_tick().len() == 1` |
| D2 | `single_z_press_produces_one_client_bullet` | `press_shoot(p0); tick_n(1);` → `client_bullet_count(p0) == 1` |
| D3 | `three_rapid_presses_produce_three_shots` | Press shoot on ticks 0, 2, 4 → 3 shots total over 5 ticks |
| D4 | `shoot_latch_clears_after_consumption` | `press_shoot(p0); tick_n(2);` → only first tick has `server_shots_this_tick().len() == 1` |
| D5 | `shoot_direction_matches_player_direction` | Facing right (default): shot dir == 1.0. After `hold_left`, shot dir == −1.0 |
| D6 | `remote_client_receives_action_packet_for_shot` | p0 shoots → `last_actions` contains entry for p1 with kind `"shoot"` |

### Group E — Reconciliation jitter / smoothness (6 tests)

**File: `e2e/tests/reconciliation.rs`**

| # | Test name | What it asserts |
|---|---|---|
| E1 | `reconciliation_corrects_position_toward_server` | `set_client_player_pos(p0, 200.0, 100.0);` (≠ server 100.0) → after tick, `abs(client.x − server.x)` is closer to 0 than 100 |
| E2 | `reconciliation_does_not_produce_large_jumps` | After 60 ticks normal play, record client positions → max single-frame delta < 20.0 px |
| E3 | `stale_playerpos_does_not_trigger_reconciliation` | Server sends `last_seq` ≤ client `last_acked_seq` → client position unchanged |
| E4 | `first_playerpos_snaps_client_to_server` | Initial `last_acked_seq = u16::MAX` before any PlayerPos → first PlayerPos sets position to server values |
| E5 | `reconciliation_replay_uses_client_room_collision` | During room transition, client's room differs from server's reported room → replay uses client's room, no panic |
| E6 | `prediction_error_stays_bounded_over_100_ticks` | Run 100 ticks with movement → `abs(client.x − server.x)` never exceeds 50.0 px, no monotonic growth |

### Group F — Edge cases (6 tests)

**File: `e2e/tests/edge_cases.rs`**

| # | Test name | What it asserts |
|---|---|---|
| F1 | `server_handles_bogus_coords_without_panic` | Client sends coords (255, 255) → server falls back to origin level, no panic |
| F2 | `player_at_room_corner_not_stuck` | Player at (0, 0) near room origin corner can still move (pos changes after hold_right + tick) |
| F3 | `two_players_same_room_bullets_dont_collide_shooter` | p0 shoots → p0's bullet does NOT kill p0 (owner id check on collision) |
| F4 | `max_players_eight_no_panic` | Connect 8 clients → all receive JoinResponse, server handles without panic |
| F5 | `disconnect_during_room_transition_no_panic` | p0 starts transition, `disconnect_client(p0)` mid-sequence → no panic |
| F6 | `zero_length_input_buffer_replay_is_noop` | Reconcile with empty input buffer → player snapped to server position, no crash |

---

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields. The order respects dependency edges:

```
Box 1 (PacketIO trait)
  ↓
Box 2 (client lib/bin split) ──→ Box 3 (server lib/bin split + players_mut)
  ↓                                    ↓
Box 4 (World + network + handle_death refactor)  Box 5 (dead_code removal)
  ↓
Box 6 (e2e crate skeleton)
  ↓
Box 7 (LoopbackTransport)
  ↓
Box 8 (TestHarness core: TestServer + TestClient + tick loop + position/grounded helpers)
  ↓
Boxes 9–14 (test implementation, one per group)
  ↓
Box 15 (CI update)
  ↓
Box 16 (quality gates)
```

---

### Box 1 — `PacketIO` trait definition

- [ ] done
- **Acceptance**: `cargo build -p client` compiles the new `client/src/network/packet_io.rs` module. The trait has two methods: `try_recv(&mut self) -> Option<GamePacket>` and `send(&mut self, packet: GamePacket)`.
- **Test**: No dedicated test — this is a pure trait definition. Verified by Box 2 compiling against it.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 2 — Client lib/bin split

- [ ] done
- **Acceptance**:
  1. `client/Cargo.toml` has a `[lib]` section (name matches the crate: `metroidvania-rust`).
  2. `client/src/lib.rs` exists and re-exports: `pub mod network; pub mod world;` plus key types (`World`, `PacketIO`, `handle_packet`, `send_join_req`, `update_network`, `check_level_transitions`).
  3. `client/src/main.rs` is a thin wrapper: uses `use metroidvania_rust::...` to import `World`, defines `RealPacketIO` locally, calls the lib's `World::new` with `Box::new(RealPacketIO { ... })`.
  4. `cargo build -p client` compiles both lib and bin targets.
  5. `cargo test -p client` passes all 73 existing tests.
- **Test**: All existing client unit tests (73) still pass.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 3 — Server lib/bin split

- [ ] done
- **Acceptance**:
  1. `server/Cargo.toml` has a `[lib]` section (name matches: `metroidvania-server`).
  2. `server/src/lib.rs` exists, declares `mod world;`, and re-exports `pub use world::ServerWorld;`.
  3. `server/src/main.rs` uses `use metroidvania_server::ServerWorld;` instead of a local `mod world;`.
  4. `ServerWorld` gains `pub fn players_mut(&mut self) -> &mut HashMap<u8, Player>` so the e2e harness can set `player.grounded` for jump tests (A3, A4) without making `players` field public.
  5. `cargo build -p server` compiles both targets.
  6. `cargo test -p server` passes all existing tests.
- **Test**: All existing server unit tests still pass.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 4 — World refactoring: `Box<dyn PacketIO>` + programmatic input fields

- [ ] done
- **Acceptance**:
  1. `World.network` field type changes from `NetworkClient` to `Box<dyn PacketIO>`.
  2. `World` gains `move_left: bool` and `move_right: bool` fields (default `false`).
  3. `World::new(map, network)` takes `Box<dyn PacketIO>`.
  4. `send_join_req` signature becomes `pub fn send_join_req(network: &mut dyn PacketIO)`.
  5. `update_network` reads `world.move_left`/`world.move_right` instead of calling `is_key_down`. It drains via `world.network.try_recv()` and sends via `world.network.send()`.
  6. `World::update` stores `self.move_left = is_key_down(Left)` and `self.move_right = is_key_down(Right)`.
  7. `handle_packet` is `pub` (not `pub(crate)`) so the e2e crate can call it.
  8. `handle_death` in `client/src/world/lib.rs` calls `world.network.send(GamePacket::Hit { entity_id: my_id })` via the `PacketIO` trait instead of the old laminar-specific `world.network.sender.send(laminar::Packet::reliable_ordered(...))`.
  9. `RealPacketIO` is defined in `client/src/main.rs` and impls `PacketIO`. Its `send` method uses variant-aware reliability: `JoinRequest` and `Hit` packets use `laminar::Packet::reliable_ordered`; all other packets use `laminar::Packet::unreliable`. See design decision note 8 below for rationale.
  10. `cargo build -p client` compiles cleanly.
  11. `cargo test -p client` passes all 73 unit tests.
- **Test**: All existing client tests pass. No new test yet (tests come in Boxes 9–14).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 5 — Dead code removal

- [ ] done
- **Acceptance**:
  1. `#[expect(dead_code)]` removed from `InputEntry::from_game_input` (line 16 of `prediction.rs`).
  2. `#[allow(dead_code)]` removed from `InputRingBuffer::is_empty` (line 85).
  3. `#[allow(dead_code)]` removed from `InputRingBuffer::len` (line 90).
  4. `cargo build -p client` compiles with zero dead-code warnings.
- **Test**: `cargo clippy -p client -- -D warnings` passes.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 6 — e2e crate skeleton + workspace registration

- [ ] done
- **Acceptance**:
  1. `e2e/` directory created with `Cargo.toml` as specified (depends on `game`, `protocol`, `client`, `server`, `postcard`; no `macroquad`/`laminar`/`crossbeam-channel`).
  2. `e2e/src/lib.rs` exists (can be empty or with a `pub mod` declaration).
  3. `e2e/src/harness.rs` exists with stub `TestHarness`, `TestClient`, `TestServer` struct definitions (fields as specified, methods can be `todo!()`).
  4. `e2e/src/transport.rs` exists with an empty `LoopbackTransport` struct.
  5. Root `Cargo.toml` workspace members include `"e2e"`.
  6. `cargo check -p e2e` succeeds (all types resolve).
- **Test**: `cargo check -p e2e` passes.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 7 — `LoopbackTransport` implementation

- [ ] done
- **Acceptance**:
  1. `LoopbackTransport` has `rx: mpsc::Receiver<Vec<u8>>` and `tx: mpsc::Sender<Vec<u8>>`.
  2. `impl PacketIO for LoopbackTransport`:
     - `try_recv`: calls `rx.try_recv()`, deserializes with `postcard::from_bytes`. Returns `Some(packet)` on success, `None` on `TryRecvError::Empty` or deserialization failure.
     - `send`: serializes with `postcard::to_allocvec`, sends via `tx.send()`. Logs error on send failure (channel closed).
  3. Unit test in `e2e/src/transport.rs` (or a `#[cfg(test)]` module in the same file):
     - Create a pair of channels, wrap in `LoopbackTransport`.
     - `send(GamePacket::JoinRequest)` → `try_recv()` returns `Some(GamePacket::JoinRequest)`.
     - Round-trip test: serialize a `GamePacket::PlayerPos { ... }` through `send` → `try_recv` → assert all fields match.
- **Test**: `cargo test -p e2e` passes the loopback round-trip test.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 8 — `TestHarness` core: constructors + tick loop + position helpers

- [ ] done
- **Acceptance**:
  1. `TestHarness::new()` creates `ServerWorld`, empty client/transport vectors, `VirtualClock::new()`.
  2. `connect_client()`:
     - Allocates `ClientHandle(next_handle++)` and `next_player_id++`.
     - Creates mpsc channel pair (`tx_to_server`, `rx_from_client`) for client→server.
     - Creates mpsc channel pair (`tx_to_client`, `rx_from_server`) for server→client.
     - Builds `LoopbackTransport { rx: rx_from_server, tx: tx_to_server }`.
     - Builds `World::new(Map::new(), Box::new(loopback))`.
     - Calls `server.world.spawn(player_id)`.
     - Sends `GamePacket::JoinResponse { assigned_id: player_id }` via `tx_to_client.send()`.
     - Calls `handle_packet(&mut world, JoinResponse { ... })` to set `world.id`.
     - Stores `TestClient`, `from_clients`, `to_clients` entries.
     - Returns `ClientHandle`.
  3. `disconnect_client(handle)`:
     - Removes player from `server.world`.
     - Broadcasts `GamePacket::Leave { id }` to all remaining `to_clients`.
     - Removes client from the vectors.
  4. `tick()`:
     - Step 1: Apply queued inputs to each TestClient's `world.move_left/right/pressed_jump/pressed_shoot`.
     - Step 2: Drain all `from_clients` channels, deserialize, call `server.world.apply_input`.
     - Step 3: Call `server.world.update(tick_dt)`, store returned shots.
     - Step 4: Broadcast `PlayerPos` for every player to every client (own-echo included). Store in `last_broadcasts`.
     - Step 5: Broadcast `Action` for each shot to other clients. Store in `last_actions`.
     - Step 6: For each client, drain `to_clients` channel, call `handle_packet`.
     - Step 7: For each client, run local prediction (`player.update`, `check_level_transitions`).
     - Step 8: Advance `VirtualClock` by 16_667 µs.
  5. Input queuing methods (`hold_right`, `hold_left`, `press_jump`, `press_shoot`, `release_all`) set `QueuedInput` fields.
  6. Inspection methods (`server_player_pos`, `client_player_pos`, `client_room`, `remote_player_pos`, `remote_interp_has_snapshots`, `client_bullet_count`, `client_enemy_bullet_count`, `server_shots_this_tick`) return correct values.
  7. `set_server_player_pos(handle, x, y)` modifies the server-side `Player.pos` via `server.world.players_mut()`.
  8. `set_server_player_grounded(handle, bool)` modifies `player.grounded` via `server.world.players_mut()`.
  9. `set_client_player_pos(handle, x, y)` modifies the client-side `self.clients[handle.0].world.player.pos`.
  10. `cargo test -p e2e` compiles; the harness is usable for test authors.
- **Test**: A minimal smoke test: `TestHarness::new(); h.connect_client(); h.tick_n(1);` — no panic, tick completes.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 9 — Group A tests (movement, 7 tests)

- [ ] done
- **Acceptance**: Tests A1–A7 compile and pass. `cargo test -p e2e movement` is green.
- **Test**: A1–A7 themselves.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 10 — Group B tests (room transitions, 9 tests)

- [ ] done
- **Acceptance**: Tests B1–B9 compile and pass. `cargo test -p e2e rooms` is green.
- **Test**: B1–B9.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 11 — Group C tests (multi-player visibility, 7 tests)

- [ ] done
- **Acceptance**: Tests C1–C7 compile and pass. `cargo test -p e2e multiplayer` is green.
- **Test**: C1–C7.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 12 — Group D tests (shoot, 6 tests)

- [ ] done
- **Acceptance**: Tests D1–D6 compile and pass. `cargo test -p e2e shoot` is green.
- **Test**: D1–D6.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 13 — Group E tests (reconciliation jitter, 6 tests)

- [ ] done
- **Acceptance**: Tests E1–E6 compile and pass. `cargo test -p e2e reconciliation` is green.
- **Test**: E1–E6.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 14 — Group F tests (edge cases, 6 tests)

- [ ] done
- **Acceptance**: Tests F1–F6 compile and pass. `cargo test -p e2e edge_cases` is green.
- **Test**: F1–F6.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 15 — CI update

- [ ] done
- **Acceptance**:
  1. `.github/workflows/ci.yml` is unchanged (`cargo test --workspace` already covers `e2e`).
  2. `cargo test --workspace` passes including all e2e tests.
  3. `cargo clippy --workspace --all-targets -- -D warnings` passes.
  4. `cargo fmt --check --all` passes.
- **Test**: CI pipeline (all three steps) is green.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 16 — Quality gates (full workspace)

- [ ] done
- **Acceptance**:
  1. `cargo test --workspace` — all existing 73 unit tests + all ~41 e2e tests pass (~114 total).
  2. `cargo clippy --workspace --all-targets -- -D warnings` — zero warnings.
  3. `cargo fmt --check --all` — zero diffs.
  4. `cargo build --workspace` — all crates compile.
- **Test**: The gates themselves.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

## Non-obvious implementation notes

1. **`test` directory vs inline `#[cfg(test)]`.** The test modules live in
   `e2e/tests/` (sibling to `src/`), not `e2e/src/tests/`. Each file
   (e.g. `e2e/tests/movement.rs`) is a separate integration-test binary that
   can `use e2e::harness::TestHarness`. Each file needs `use e2e::harness::TestHarness;`
   and must be a separate crate root (no `mod` declarations). The `e2e/src/`
   directory contains only the harness library code (`harness.rs`, `transport.rs`,
   `lib.rs`).

2. **`&mut self` vs `&self` on `PacketIO`.** We use `&mut self` because both
   `mpsc::Receiver::try_recv` (consumes the message) and
   `crossbeam_channel::Receiver::try_recv` require `&mut self`.
   `Box<dyn PacketIO>` will need `&mut` wherever `try_recv`/`send` are called —
   this affects `World::update`, `update_network`, `send_join_req`, and `handle_death`.

3. **`World` holding `Box<dyn PacketIO>` and the borrow checker.**
   `update_network` takes `&mut World`. When it calls `world.network.try_recv()`
   and `world.network.send()`, Rust will see two mutable borrows (`world` and
   `world.network`). Since `network` is a field, the compiler will handle this
   via split borrowing (fields are disjoint). This works as long as we don't try
   to pass `&mut world` and `&mut world.network` simultaneously as separate
   function arguments. The implementer should call the trait methods directly on
   `world.network` rather than passing it as a separate argument.

4. **`VirtualClock` and `Instant` anchoring.** The `VirtualClock` creates an
   `Instant::now()` anchor at construction. `clock.now()` returns
   `anchor + Duration::from_micros(us)`. This means the returned `Instant` is
   always in the past or present relative to real time, but tests never call the
   real `Instant::now()` — they always use `clock.now()`. The `interpolate_at`
   method on `RemoteInterp` already accepts an explicit `Instant`, so the
   harness can feed virtual timestamps.

5. **Broadcast semantics.** The server tick loop in the harness must replicate
   the real server's broadcast behavior exactly:
   - `PlayerPos` goes to ALL clients including the sender (own-echo).
   - `Action` (shoot) goes to all clients EXCEPT the shooter.
   - `Leave` goes to all remaining clients after removal.
   - `JoinResponse` goes only to the joining client.

6. **Client preview area.** The client test files live under `e2e/tests/` as
   integration test binaries. The `e2e/src/lib.rs` must re-export `TestHarness`,
   `TestServer`, `TestClient`, `ClientHandle`, `LoopbackTransport`, and
   `VirtualClock` so test files can import them.

7. **Server-side collision is off by default in e2e tick step 7.**
   The `c.world.player.update(dt, level, &local_input)` call in tick step 7
   uses `Player::update` which applies collision against the level. The server's
   tick (step 3) does the same. This means both server and client simulations
   run the same physics code — only the timing of when inputs arrive differs.

8. **Design decision — `RealPacketIO::send` variant-aware reliability.**
   The `PacketIO::send` trait method takes `GamePacket` by value and has no
   `reliable: bool` parameter. This keeps the trait simple and the loopback
   transport agnostic to reliability semantics. `RealPacketIO::send` inspects
   the packet variant and chooses the laminar delivery mode:
   - `JoinRequest` and `Hit` → `reliable_ordered` (critical setup/teardown
     messages that must not be lost — a lost JoinRequest means the player
     never gets an ID, and a lost Hit means death is never broadcast).
   - All other packets (`PlayerInput`, `Action`, `Leave`) → `unreliable`
     (fire-and-forget; redundancy comes from sticky edges and periodic
     broadcasts).
   This is the simplest approach that preserves correctness without bloating
   the trait. The alternative of adding a `reliable: bool` parameter forces the
   loopback transport to carry a flag it never uses, and the alternative of
   keeping `send_join_req`/`handle_death` as free functions in the binary target
   creates a split code path that the e2e harness cannot exercise.

9. **Design decision — spawn at (100,100) is NOT on solid ground.**
   The tile at row 2, col 2 (covering pixels y=80–120, x=80–120) is tile type 0
   (Empty) in LVL_0_0. The player spawns at (100,100) over empty space and falls
   due to gravity for approximately 330 ticks before hitting the first solid
   surface below. This means:
   - Tests A1/A2 (hold left/right) work: the player falls while moving sideways.
   - Tests A3/A4 (jump) require `set_server_player_grounded` — the player must
     be artificially set as grounded, otherwise jump is ignored.
   - Tests A5 (gravity only) works: y increases as player falls.
   - Tests B1-B4 (room transitions) need `set_server_player_pos` to place the
     player at the correct Y (or X) coordinate for the room exit gap — the
     player cannot reach the gap by natural movement from (100,100) in a
     reasonable number of ticks.

10. **Room transition gap coordinates.** From `game/src/maps.rs` LVL_0_0:
    - Exit-right: tile row 8 (pixels y = 320-360), tile type 0 (Empty).
      First wall tile in row 8 is at index 19 (pixel x = 760). The level width
      is 20 tiles × 40 = 800px. `check_level_transitions` triggers when
      `x + w > 795` (800 − 5). So set player at x ≈ 790, y ≈ 320 for B1.
    - Exit-down: tile cols 7-8 (pixels x = 280-360), tile type 0 (Empty) in
      row 11. Trigger is `y + h > 475` (480 − 5). Set player at x ≈ 300,
      y ≈ 470 for B3.

11. **Server `players_mut()` accessor.** `ServerWorld::players_mut()` returns
    `&mut HashMap<u8, Player>`. This is intentionally broad — it allows the
    e2e harness to set any player field. The harness only uses it for
    `player.pos` and `player.grounded` in `set_server_player_pos` and
    `set_server_player_grounded`. The alternative of adding individual setter
    methods (`set_pos`, `set_grounded`) adds boilerplate to `ServerWorld` for
    fields the server itself never needs to mutate externally. Since
    `ServerWorld` is only depended on by the server binary and the e2e crate
    (which is test-only), the accessor approach is pragmatic.

12. **Client position setting for test E1.** `set_client_player_pos` directly
    modifies `self.clients[handle.0].world.player.pos`. This is intentionally
    direct field access rather than a method on `World` — `World` should not
    expose a public position-setter for production use. The harness already
    owns the `TestClient` vec, so field access is ergonomic and clearly scoped
    to test code.

13. **Dead code removal timing.** The three `#[allow(dead_code)]` /
    `#[expect(dead_code)]` annotations on `InputRingBuffer::is_empty`,
    `InputRingBuffer::len`, and `InputEntry::from_game_input` are removed in
    Box 5. However, until the e2e tests use these functions (Boxes 9–14), the
    compiler will warn about dead code again. This is acceptable — the warnings
    will be temporary and resolve when the test boxes land. The alternative
    (removing the annotations in Box 14) would mean the e2e tests can't call
    these functions until the annotations are gone, which is wrong (the
    annotations suppress warnings, not errors). If CI gates require zero
    warnings at every commit, add temporary `#[allow(dead_code)]` annotations
    back in Box 5 and remove them in Box 14 instead.

14. **Module path for `world::lib`.** The `client::world::lib` module contains
    `check_level_transitions` (used by the harness) and `handle_death`
    (refactored internally). It also imports `macroquad::prelude::*` (for
    `vec2` in `handle_death`). When `World` is used from the e2e crate (which
    has no macroquad dependency), the `vec2` import in `lib.rs` will fail to
    resolve. **Solution:** Move `handle_death` into `world/game.rs` (as a
    private method on `World`, or a free function in that module) so that
    `world/lib.rs` only contains `check_level_transitions` and
    `update_enemy_bullets_position` — both of which can be written without
    macroquad. Alternatively, gate `handle_death` behind `#[cfg(feature =
    "macroquad")]`, but that adds complexity. The cleaner approach is to inline
    `vec2(100.0, 100.0)` as `glam::Vec2::new(100.0, 100.0)` in `handle_death`
    and keep it in `world/lib.rs`. Either way, the implementer must ensure
    `world/lib.rs` compiles without macroquad when used from the e2e crate.

## Open questions

None. The spec resolves all 7 architectural decisions (D1–D7). The plan provides
exact signatures, test names, and acceptance criteria for all 16 boxes, plus 14
implementation notes covering all design decisions and edge cases.