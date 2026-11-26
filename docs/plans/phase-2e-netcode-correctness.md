# Phase 2e — Netcode Correctness: Server-Authoritative Bullet Spawn & Enemy Death

- Status: draft
- Spec: `docs/specs/phase-2e-netcode-correctness.md`
- Date: 2026-11-25

## Overview

This phase fixes the "hit reverts" bug and related netcode correctness issues by making bullet spawn and enemy death **server-authoritative**.

**Fix 1 — Server-Authoritative Bullet Spawn:**
- Server confirms each shot with `BulletSpawn { bullet_id, x, y, dir, owner_id, seq }` broadcast to **all** clients (including shooter).
- Shooter client **waits for `BulletSpawn`** before spawning its local bullet — no longer spawns on `pressed_shoot`.
- Replaces the `Action { kind: "shoot" }` rebroadcast (which went to other clients only).
- Fixes "hit reverts" — reconciliation now uses server-confirmed bullet timing, not local speculative timing.

**Fix 2 — Server-Authoritative Enemy Death:**
- Server validates hits: on `BulletSpawn`, raycasts bullet path against server-side enemy hitboxes in that room.
- Server tracks enemy alive/dead state per room.
- Server broadcasts `EnemyDied { enemy_id, room_x, room_y }` to all clients in the room.
- On room load, client requests `EnemyState` from server and spawns **only alive** enemies.
- Fixes "enemy respawns on room change" and "hit reverts".

The phase adds four new packet types (`BulletSpawn`, `EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse`), removes `Action { kind: "shoot" }`, introduces server-side enemy state with raycast hit validation, and rewires client bullet/enemy flows to be server-driven.

---

## Public API (targets for the tester)

### `protocol/src/lib.rs` — New Packet Variants

```rust
// GamePacket enum additions (Action variant removed)
GamePacket::BulletSpawn {
    bullet_id: u16,
    x: f32,
    y: f32,
    dir: f32,
    owner_id: u8,
    seq: u16,
}

GamePacket::EnemyDied {
    enemy_id: u8,
    room_x: u8,
    room_y: u8,
}

GamePacket::EnemyStateRequest {
    room_x: u8,
    room_y: u8,
}

GamePacket::EnemyStateResponse {
    room_x: u8,
    room_y: u8,
    enemies: Vec<(u8, bool, f32, f32)>, // (enemy_id, alive, spawn_x, spawn_y)
}
```

### `server/src/world.rs` — ServerWorld API

```rust
use std::collections::HashMap;
use game::level::Level;
use game::maps::Map;
use game::entities::player::{Player, PlayerInput};
use game::rect::Rect;

pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords: HashMap<u8, (u8, u8)>,
    levels: Vec<Vec<Level>>,
    max_seq: HashMap<u8, u16>,
    shoot_cooldown: HashMap<u8, u8>,
    // NEW
    enemies: HashMap<(u8, u8), Vec<EnemyState>>,
    next_bullet_id: u16,
}

pub struct EnemyState {
    pub id: u8,
    pub alive: bool,
    pub spawn_pos: (f32, f32), // tile center (x * TILE_SIZE + TILE_SIZE/2, y * TILE_SIZE + TILE_SIZE/2)
}

pub struct BulletSpawnInfo {
    pub bullet_id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: f32,
    pub owner_id: u8,
    pub seq: u16,
}

pub struct EnemyDiedInfo {
    pub enemy_id: u8,
    pub room_x: u8,
    pub room_y: u8,
}

impl ServerWorld {
    pub fn new() -> Self;
    pub fn spawn(&mut self, id: u8);
    pub fn remove(&mut self, id: u8);
    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16);
    /// Returns (shots, enemy_deaths)
    pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>);
    pub fn get_enemy_state(&self, room_x: u8, room_y: u8) -> Vec<(u8, bool, f32, f32)>;
    pub fn players(&self) -> &HashMap<u8, Player>;
    pub fn coords(&self) -> &HashMap<u8, (u8, u8)>;
    pub fn max_seq(&self) -> &HashMap<u8, u16>;
}
```

### `client/src/network/world.rs` — Packet Handlers

```rust
pub fn handle_packet(world: &mut World, packet: GamePacket, socket: &mut PacketSocket) {
    // NEW cases:
    // GamePacket::BulletSpawn { ... } => { /* spawn local bullet, dedupe, track confirmed */ }
    // GamePacket::EnemyDied { enemy_id, room_x, room_y } => { /* find enemy, die() */ }
    // GamePacket::EnemyStateResponse { room_x, room_y, enemies } => { /* spawn alive enemies */ }
}
```

### `client/src/world/game.rs` — World Changes

```rust
pub struct World {
    // ... existing fields ...
    pub spawned_bullet_ids: std::collections::HashSet<u16>, // NEW
    pub confirmed_bullets: std::collections::HashMap<u16, ConfirmedBullet>, // NEW: seq -> confirmed bullet data
}

pub struct ConfirmedBullet {
    pub bullet_id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: f32,
}

// In World::update:
// - REMOVED: if pressed_shoot { spawn bullet }
// - REMOVED: own-bullet vs enemy collision loop (lines that called enemy.die())
// - Bullet physics and wall collision remain
```

### `client/src/world/lib.rs` — Room Load Flow

```rust
pub fn load_current_level(world: &mut World) {
    world.enemies.clear();
    world.bullets.clear();
    world.enemy_bullets.clear();
    world.spawned_bullet_ids.clear(); // NEW: clear on room transition
    world.confirmed_bullets.clear(); // NEW
    let (rx, ry) = world.current_coords;
    let req = GamePacket::EnemyStateRequest {
        room_x: rx as u8,
        room_y: ry as u8,
    };
    world.network.send(req);
    // Response handled in handle_packet
}
```

### `game/src/entities/enemy.rs` — Enemy ID Field

```rust
pub struct Enemy {
    pub pos: Vec2,
    pub vel: Vec2,
    pub w: f32,
    pub h: f32,
    pub grounded: bool,
    pub state: EnemyState,
    pub timer: f32,
    pub alive: bool,
    pub kind: EnemyKind,
    pub id: u8, // NEW: stable ID (room_x * 16 + spawn_index)
}
```

---

## Test Cases (tester writes these first)

Organized by the 32 acceptance criteria from the spec. Each test case maps to one or more ACs.

### Protocol Tests (Unit — `protocol/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_bulletspawn_roundtrip` | 1, 3 | Serialize/deserialize `BulletSpawn` with representative values; all fields preserved. |
| `test_enemydied_roundtrip` | 1, 3 | Serialize/deserialize `EnemyDied`; all fields preserved. |
| `test_enemystaterequest_roundtrip` | 1, 3 | Serialize/deserialize `EnemyStateRequest`; all fields preserved. |
| `test_enemystateresponse_roundtrip` | 1, 3 | Serialize/deserialize `EnemyStateResponse` with multiple enemies; all fields preserved. |
| `test_action_variant_removed` | 2 | Compile-fail test: `GamePacket::Action { .. }` does not exist. |
| `test_no_game_or_glam_deps` | 4 | `cargo tree -p protocol` shows no `game` or `glam` dependencies. |

### Server Enemy State & Hit Validation (Unit — `server/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_serverworld_builds_enemies_all_rooms` | 5 | `ServerWorld::new()` builds `enemies` map for all 2×2 rooms (room_x 0..1, room_y 0..1); iterates `Map::new().data` in row-major room order (room_y outer, room_x inner); for each `EntityType::EnemySpawn` or `PotionSpawn`, creates `EnemyState { id: room_x * 16 + spawn_index, alive: true, spawn_pos: (x * TILE_SIZE + TILE_SIZE/2, y * TILE_SIZE + TILE_SIZE/2) }` (tile center); stores in `enemies[(room_x, room_y)]`. |
| `test_raycast_hits_enemy` | 6 | Given a room with enemy at known tile, raycast from left traveling right hits that enemy → `alive=false`. Server hitbox = `Rect::new(spawn_x - 15.0, spawn_y - 19.0, 30.0, 38.0)` (centered on tile center `spawn_pos`, matches client `Enemy::get_rect()`). |
| `test_raycast_miss_above_below` | 7 | Raycast passing above/below enemy rect does NOT hit; `alive` remains `true`. |
| `test_raycast_first_hit_wins` | 7 (2nd) | Two enemies in line horizontally; only closer one hit; farther remains alive. |
| `test_update_returns_shots_and_deaths` | 8 | `update()` returns `Vec<BulletSpawnInfo>` and `Vec<EnemyDiedInfo>`; main.rs broadcasts both. |

### Server Bullet Confirmation (Unit — `server/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_shoot_emits_bulletspawninfo` | 9 | Confirmed `shoot` input (cooldown 0) → `update()` emits `BulletSpawnInfo` with incrementing `bullet_id`, correct spawn pos from player center, `dir`, `owner_id`, input `seq`. |
| `test_bulletspawn_broadcast_all_clients` | 10 | `main.rs` broadcasts `BulletSpawn` (reliable-ordered) to ALL connected clients including shooter. |
| `test_action_shoot_removed` | 11 | Server no longer sends `Action { kind: "shoot" }` (code path removed). |
| `test_shoot_cooldown_enforced` | 12 | Two `shoot` inputs within 5 ticks → only first produces `BulletSpawnInfo`; second is consumed but no spawn. |

### Client Bullet Spawn Flow (E2E — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_no_local_bullet_on_pressed_shoot` | 13 | `World::update` does NOT spawn bullet when `pressed_shoot=true` (code removed). |
| `test_bulletspawn_handler_spawns_own_bullet` | 14 | `handle_packet` on `BulletSpawn` with `owner_id == world.id` spawns `Bullet` in `world.bullets` at server `(x,y)` with server `dir`. |
| `test_bulletspawn_handler_spawns_remote_bullet` | 14 | `handle_packet` on `BulletSpawn` with `owner_id != world.id` spawns `Bullet` in `world.enemy_bullets`. |
| `test_bulletspawn_dedupe` | 15 | Duplicate `BulletSpawn` with same `bullet_id` ignored (client tracks `spawned_bullet_ids`). |
| `test_e2e_bulletspawn_both_clients` | 16 | Client A presses Z → server broadcasts `BulletSpawn` → Client A and Client B both have bullet at same server-authoritative position. |

### Client Enemy Death Flow (E2E — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_enemy_has_id_field` | 17 | `Enemy` struct has `id: u8`; `enemies_from_data` assigns sequential IDs per room matching server scheme (`room_x * 16 + spawn_index`). |
| `test_enemydied_handler_kills_enemy` | 18 | `handle_packet` on `EnemyDied` finds enemy by `enemy_id` (room match) and calls `die()`. |
| `test_own_bullet_enemy_collision_removed` | 19 | `World::update` own-bullet vs enemy collision loop removed; no local `enemy.die()` on bullet collision. |
| `test_e2e_shoot_enemy_both_clients` | 20 | Client A shoots enemy → server validates hit → `EnemyDied` broadcast → enemy dies on Client A AND Client B. |

### Room Load Sync (E2E — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_load_level_sends_request` | 21 | `load_current_level` clears enemies/bullets/enemy_bullets, clears `spawned_bullet_ids` and `confirmed_bullets`, sends `EnemyStateRequest` (reliable-ordered) for current coords. |
| `test_server_enemystate_response` | 22 | Server handles `EnemyStateRequest`, replies with `EnemyStateResponse` containing `(enemy_id, alive, spawn_x, spawn_y)` for all enemies in room. |
| `test_client_spawns_only_alive` | 23 | `handle_packet` on `EnemyStateResponse` re-runs `enemies_from_data` for the room, matches by spawn order, sets `enemy.id`, spawns only `alive` enemies. |
| `test_e2e_enemy_stays_dead_across_room_transition` | 24 | Client A kills enemy in (0,0) → transitions to (1,0) and back → enemy stays dead (not respawned). |

### Reconciliation Integration (E2E — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_reconciliation_uses_confirmed_bullets` | 25 | On `BulletSpawn` receipt: `confirmed_bullets.insert(seq, ConfirmedBullet { ... })`. During reconciliation replay: when replaying input `seq`, check `confirmed_bullets.get(&seq)`. If present, **do not** use local `pressed_shoot` for that frame; bullet spawn is authoritative. If absent, use local prediction (for inputs that had `shoot` but no server confirmation yet). |
| `test_e2e_no_position_snap_on_shoot` | 26 | Client A moves right, shoots (seq 10) → server confirms `BulletSpawn(seq=10)` → reconciliation on `PlayerPos(last_seq=10)` shows no position correction related to shot. |

### Regression (E2E — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_no_duplicate_join_request` | 27 | Phase 2d fix: `World::new` does not send duplicate `JoinRequest`. |

### Edge-Case E2E Tests (New — `e2e/tests/`)

| Test | AC | Description |
|------|----|-------------|
| `test_e2e_bullet_spawn_during_room_transition` | — | Shooter fires, transitions room before `BulletSpawn` arrives → bullet spawns in new room but server raycast was in old room; client ignores or handles gracefully (bullet despawns on wall). |
| `test_e2e_enemy_died_before_room_load` | — | Client receives `EnemyDied` for enemy not yet in local list (room mismatch) → ignore silently; room load `EnemyStateResponse` shows `alive: false`. |
| `test_e2e_shoot_cooldown_edge` | — | Two shoots with 4-tick gap (cooldown=5) → second fires on tick when cooldown expires (first at tick 0, second at tick 5). |

### Quality Gates (CI — run on all crates)

| Test | AC | Description |
|------|----|-------------|
| `cargo_build_workspace` | 28 | `cargo build --workspace` succeeds (protocol, game, server, client). |
| `cargo_test_workspace` | 29 | `cargo test --workspace` passes (all new unit tests + e2e tests + existing tests). |
| `cargo_clippy_workspace` | 30 | `cargo clippy --all-targets --workspace -- -D warnings` exits 0. |
| `cargo_fmt_check` | 31 | `cargo fmt --check --all` reports no diffs. |
| `test_no_unwrap_in_new_logic` | 32 | `grep -rn "unwrap()\|expect("` on new logic files finds zero uses (all `Option`/`Result` handling explicit). |

---

## Implementation Checklist

Each box is a small, independently verifiable step. The `implementer` fills the `Explain` fields.

**New dependency order:** A → D → B → C → E → F → G → H → I → J

### Group A: Protocol — New Packet Types

#### Box A1 — Add `BulletSpawn`, `EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse` to `GamePacket`

- [ ] done
- **Acceptance**: `protocol/src/lib.rs` compiles; four new variants exist with exact fields from spec D1; `Action` variant removed.
- **Test**: `test_bulletspawn_roundtrip`, `test_enemydied_roundtrip`, `test_enemystaterequest_roundtrip`, `test_enemystateresponse_roundtrip`, `test_action_variant_removed`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box A2 — Protocol round-trip tests for new variants

- [ ] done
- **Acceptance**: Each new variant serializes/deserializes with postcard; fields preserved; tests in `protocol/tests/` pass.
- **Test**: `test_bulletspawn_roundtrip`, `test_enemydied_roundtrip`, `test_enemystaterequest_roundtrip`, `test_enemystateresponse_roundtrip`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box A3 — Verify `protocol` crate has no `game` or `glam` dependencies

- [ ] done
- **Acceptance**: `cargo tree -p protocol` shows no `game` or `glam`; `protocol/Cargo.toml` unchanged for deps.
- **Test**: `test_no_game_or_glam_deps`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group D: Game Crate — Enemy ID Field (MOVED BEFORE B)

#### Box D1 — Add `id: u8` field to `Enemy` struct in `game/src/entities/enemy.rs`

- [ ] done
- **Acceptance**: `Enemy` has `pub id: u8`; all construction sites updated.
- **Test**: `test_enemy_has_id_field`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box D2 — `enemies_from_data` assigns sequential IDs per room (0, 1, 2... matching server's `room_x * 16 + index`)

- [ ] done
- **Acceptance**: Iterates spawn cells in deterministic order (row-major y, then x); assigns `id = room_x * 16 + local_index`; server and client use same logic.
- **Test**: `test_enemy_has_id_field`, `test_e2e_enemy_stays_dead_across_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group B: Server — Enemy State & Hit Validation

#### Box B1 — Add `EnemyState`, `BulletSpawnInfo`, `EnemyDiedInfo` structs to `server/src/world.rs`

- [ ] done
- **Acceptance**: Structs defined with exact fields from spec; `EnemyState { id: u8, alive: bool, spawn_pos: (f32, f32) }` where `spawn_pos` is tile center `(x * TILE_SIZE + TILE_SIZE/2, y * TILE_SIZE + TILE_SIZE/2)`.
- **Test**: (struct definitions enable B2–B5)
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box B2 — `ServerWorld::new()` builds `enemies: HashMap<(u8,u8), Vec<EnemyState>>` for all rooms

- [ ] done
- **Acceptance**: On `new()`, iterates `Map::new().data` (EntityType) in row-major room order: `for room_y in 0..2 { for room_x in 0..2 { for y in 0..MAP_HEIGHT { for x in 0..MAP_WIDTH } } }`; for each `EntityType::EnemySpawn` or `PotionSpawn`, creates `EnemyState { id: room_x * 16 + spawn_index, alive: true, spawn_pos: (x as f32 * TILE_SIZE + TILE_SIZE as f32 / 2.0, y as f32 * TILE_SIZE + TILE_SIZE as f32 / 2.0) }` (tile center); stores in `enemies[(room_x, room_y)]`.
- **Test**: `test_serverworld_builds_enemies_all_rooms`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box B3 — Implement raycast hit validation in `ServerWorld::update`

- [ ] done
- **Acceptance**: For each player with latched `shoot` and `cooldown==0`: computes spawn from player center + dir; raycasts horizontal segment (range 400) against alive enemies in player's room; enemy hitbox = `Rect::new(spawn_x - 15.0, spawn_y - 19.0, 30.0, 38.0)` (centered on `spawn_pos`, matches client `Enemy::get_rect()`); first hit wins; sets `enemy.alive=false`; emits `BulletSpawnInfo` and (if hit) `EnemyDiedInfo`.
- **Test**: `test_raycast_hits_enemy`, `test_raycast_miss_above_below`, `test_raycast_first_hit_wins`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box B4 — `ServerWorld::update` returns `(Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)`

- [ ] done
- **Acceptance**: Signature changed; collects shot confirmations and enemy deaths per tick; returns both vectors.
- **Test**: `test_update_returns_shots_and_deaths`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box B5 — Add `ServerWorld::get_enemy_state(room_x, room_y)` for room load sync

- [ ] done
- **Acceptance**: Returns `Vec<(u8, bool, f32, f32)>` for all enemies in that room (id, alive, spawn_x, spawn_y).
- **Test**: `test_server_enemystate_response` (via main.rs handler)
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box B6 — Server unit test: shoot cooldown enforced (only first of two rapid shoots spawns)

- [ ] done
- **Acceptance**: Two `shoot` inputs within 5 ticks → only first produces `BulletSpawnInfo`.
- **Test**: `test_shoot_cooldown_enforced`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group C: Server — Main Loop & Broadcasting

#### Box C1 — `server/src/main.rs`: broadcast `BulletSpawn` to ALL clients (reliable-ordered)

- [ ] done
- **Acceptance**: On `update()` returning shots, for each `BulletSpawnInfo`, constructs `GamePacket::BulletSpawn` and sends via `reliable_ordered` to every connected client (including shooter).
- **Test**: `test_bulletspawn_broadcast_all_clients`, `test_e2e_bulletspawn_both_clients`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box C2 — `server/src/main.rs`: remove `Action { kind: "shoot" }` send code path

- [ ] done
- **Acceptance**: No code constructs or sends `GamePacket::Action { kind: "shoot" }`; any reference removed.
- **Test**: `test_action_shoot_removed`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box C3 — `server/src/main.rs`: broadcast `EnemyDied` to clients in same room (reliable-ordered)

- [ ] done
- **Acceptance**: On `update()` returning enemy deaths, for each `EnemyDiedInfo`, sends `GamePacket::EnemyDied` via `reliable_ordered` to clients whose `coords` match `room_x, room_y`.
- **Test**: `test_e2e_shoot_enemy_both_clients`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box C4 — `server/src/main.rs`: handle `EnemyStateRequest` → reply `EnemyStateResponse` (reliable-ordered)

- [ ] done
- **Acceptance**: On receiving `EnemyStateRequest { room_x, room_y }`, calls `world.get_enemy_state(room_x, room_y)`, sends `EnemyStateResponse` to requesting client via `reliable_ordered`.
- **Test**: `test_server_enemystate_response`, `test_e2e_enemy_stays_dead_across_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group E: Client — Network Packet Handlers

#### Box E1 — `client/src/network/world.rs`: handle `BulletSpawn` — spawn own bullet, dedupe, track confirmed

- [ ] done
- **Acceptance**: On `BulletSpawn { bullet_id, x, y, dir, owner_id, seq }`: if `bullet_id` in `spawned_bullet_ids` return; insert; `confirmed_bullets.insert(seq, ConfirmedBullet { bullet_id, x, y, dir })`; if `owner_id == world.id` spawn `Bullet::new(Vec2::new(x,y), dir, owner_id)` into `world.bullets`; else spawn into `world.enemy_bullets`.
- **Test**: `test_bulletspawn_handler_spawns_own_bullet`, `test_bulletspawn_handler_spawns_remote_bullet`, `test_bulletspawn_dedupe`, `test_reconciliation_uses_confirmed_bullets`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box E2 — `client/src/network/world.rs`: handle `EnemyDied` — find enemy by id, call `die()`

- [ ] done
- **Acceptance**: On `EnemyDied { enemy_id, room_x, room_y }`: if room matches `world.current_coords`, find enemy in `world.enemies` with `enemy.id == enemy_id`, call `enemy.die()`.
- **Test**: `test_enemydied_handler_kills_enemy`, `test_e2e_shoot_enemy_both_clients`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box E3 — `client/src/network/world.rs`: handle `EnemyStateResponse` — spawn only alive enemies

- [ ] done
- **Acceptance**: On `EnemyStateResponse { room_x, room_y, enemies }`: if room matches, re-run `enemies_from_data` for that room; iterate response `enemies` in order; for each `(enemy_id, alive, spawn_x, spawn_y)` where `alive && i < template.len()`, set `template[i].id = enemy_id`, push to `world.enemies`.
- **Test**: `test_client_spawns_only_alive`, `test_e2e_enemy_stays_dead_across_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box E4 — Add `spawned_bullet_ids: HashSet<u16>` and `confirmed_bullets: HashMap<u16, ConfirmedBullet>` to `World` in `client/src/world/game.rs`

- [ ] done
- **Acceptance**: `World` struct has `pub spawned_bullet_ids: HashSet<u16>` and `pub confirmed_bullets: HashMap<u16, ConfirmedBullet>`; `World::new()` initializes both empty.
- **Test**: `test_bulletspawn_dedupe`, `test_reconciliation_uses_confirmed_bullets`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group F: Client — Game Logic Changes

#### Box F1 — `client/src/world/game.rs`: remove local bullet spawn on `pressed_shoot` in `World::update`

- [ ] done
- **Acceptance**: The `if pressed_shoot { world.bullets.push(Bullet::new(...)) }` block (approx lines 136–142) is deleted; `pressed_shoot` still sampled for input sending but no bullet spawn.
- **Test**: `test_no_local_bullet_on_pressed_shoot`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box F2 — `client/src/world/game.rs`: remove own-bullet vs enemy collision loop (no local `enemy.die()`)

- [ ] done
- **Acceptance**: The loop iterating `world.bullets` vs `world.enemies` calling `enemy.die()` (approx lines 150–158) is deleted; bullet physics and wall collision remain; enemy AI movement remains.
- **Test**: `test_own_bullet_enemy_collision_removed`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group G: Client — Room Load Flow

#### Box G1 — `client/src/world/lib.rs`: `load_current_level` sends `EnemyStateRequest` instead of local spawn; clears `spawned_bullet_ids` and `confirmed_bullets`

- [ ] done
- **Acceptance**: `load_current_level` clears enemies/bullets/enemy_bullets, clears `spawned_bullet_ids` and `confirmed_bullets`, constructs `EnemyStateRequest` for `current_coords`, sends via `world.network.send(req)` (reliable-ordered); does NOT call `enemies_from_data`.
- **Test**: `test_load_level_sends_request`, `test_e2e_enemy_stays_dead_across_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group H: Reconciliation Integration

#### Box H1 — `client/src/network/world.rs`: reconciliation uses `confirmed_bullets` for bullet timing verification

- [ ] done
- **Acceptance**: In `handle_packet` on `BulletSpawn`, insert into `world.confirmed_bullets` keyed by `seq`. During reconciliation replay (in `handle_packet` on `PlayerPos`): when replaying input `seq`, check `world.confirmed_bullets.get(&seq)`. If `Some(confirmed)`, **do not** use local `pressed_shoot` for that frame (bullet spawn is authoritative); if `None`, use local prediction (for inputs that had `shoot` but no server confirmation yet — e.g., before first `BulletSpawn` arrives). No position snap on shoot.
- **Test**: `test_reconciliation_uses_confirmed_bullets`, `test_e2e_no_position_snap_on_shoot`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group I: E2E Tests (New Tests in `e2e/tests/`)

#### Box I1 — E2E: `bulletspawn_both_clients` — shooter and remote see same bullet

- [ ] done
- **Acceptance**: Two clients connect; Client A presses shoot; `TestHarness` advances ticks; both clients have bullet at server-authoritative position.
- **Test**: `test_e2e_bulletspawn_both_clients`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I2 — E2E: `shoot_enemy_both_clients` — server-validated hit kills enemy on all clients

- [ ] done
- **Acceptance**: Client A shoots enemy in room; server raycast hits; `EnemyDied` broadcast; enemy `alive=false` on Client A and Client B.
- **Test**: `test_e2e_shoot_enemy_both_clients`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I3 — E2E: `enemy_stays_dead_across_room_transition` — dead enemy stays dead after room change

- [ ] done
- **Acceptance**: Client A kills enemy in (0,0); transitions to (1,0) and back; `EnemyStateResponse` shows `alive=false`; enemy not spawned.
- **Test**: `test_e2e_enemy_stays_dead_across_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I4 — E2E: `no_position_snap_on_shoot` — reconciliation doesn't snap position due to bullet timing

- [ ] done
- **Acceptance**: Client A moves right, shoots (seq 10); server confirms `BulletSpawn(seq=10)`; on `PlayerPos(last_seq=10)`, reconciliation shows no position correction attributable to the shot.
- **Test**: `test_e2e_no_position_snap_on_shoot`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I5 — E2E: `no_duplicate_join_request` — Phase 2d regression test still passes

- [ ] done
- **Acceptance**: Client connects once; only one `JoinRequest` sent.
- **Test**: `test_no_duplicate_join_request`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I6 — E2E: `bullet_spawn_during_room_transition` — shooter fires, transitions before `BulletSpawn` arrives

- [ ] done
- **Acceptance**: Client A shoots in room (0,0), immediately transitions to (1,0) before `BulletSpawn` arrives; `BulletSpawn` carries server-authoritative position from room (0,0); client spawns bullet in current room (1,0) but bullet despawns on wall collision; no crash.
- **Test**: `test_e2e_bullet_spawn_during_room_transition`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I7 — E2E: `enemy_died_before_room_load` — `EnemyDied` for enemy not in local list

- [ ] done
- **Acceptance**: Client receives `EnemyDied` for room not currently loaded (room mismatch) → ignores silently; later room load `EnemyStateResponse` shows `alive: false` for that enemy.
- **Test**: `test_e2e_enemy_died_before_room_load`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box I8 — E2E: `shoot_cooldown_edge` — two shoots with 4-tick gap (cooldown=5)

- [ ] done
- **Acceptance**: First shoot at tick 0 produces `BulletSpawn`; second shoot input at tick 4 (cooldown still >0) is latched; at tick 5 cooldown expires, second shoot fires and produces `BulletSpawn`.
- **Test**: `test_e2e_shoot_cooldown_edge`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group J: Quality Gates

#### Box J1 — `cargo build --workspace` passes

- [ ] done
- **Acceptance**: All four crates (protocol, game, server, client) build without errors.
- **Test**: `cargo_build_workspace`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box J2 — `cargo test --workspace` passes (all new + existing tests)

- [ ] done
- **Acceptance**: Unit tests (protocol, server, game) + e2e tests all pass; no regressions.
- **Test**: `cargo_test_workspace`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box J3 — `cargo clippy --all-targets --workspace -- -D warnings` passes

- [ ] done
- **Acceptance**: Zero clippy warnings across workspace.
- **Test**: `cargo_clippy_workspace`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box J4 — `cargo fmt --check --all` passes

- [ ] done
- **Acceptance**: No formatting diffs.
- **Test**: `cargo_fmt_check`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box J5 — No `unwrap()`/`expect()` in new logic files

- [ ] done
- **Acceptance**: `grep -rn "unwrap()\|expect(" server/src/world.rs client/src/network/world.rs client/src/world/game.rs client/src/world/lib.rs` returns zero matches in new code paths.
- **Test**: `test_no_unwrap_in_new_logic`
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

## Open Questions

The spec resolves D1–D6 and D7–D8. The following are confirmations needed before implementation:

1. **BulletSpawn reliability channel**: Spec says "reliable-ordered". Laminar has `reliable_ordered` (per-stream ordering) and `reliable_unordered`. Since each bullet is independent, `reliable_unordered` might suffice and avoid head-of-line blocking. However, ordering relative to `PlayerPos` might matter for reconciliation. **Recommendation**: use `reliable_ordered` with a dedicated stream ID for game events (or the default stream). Confirm with owner.

2. **Enemy ID scheme exact mapping**: Spec says `enemy_id = room_x * 16 + spawn_index` where `spawn_index` is 0-based row-major within room. The client's `enemies_from_data` iterates the level grid in row-major (y then x). Need to ensure both use **exact same iteration order**. The server iterates `Map::new().data` in row-major room order (room_y outer, room_x inner) — confirm this matches client's `enemies_from_data(&level.data)`.

3. **EnemyKind in `EnemyStateResponse`**: Spec D6 note says server could include `kind` in response. Currently `EnemyState` doesn't store `kind`. Client infers by matching `spawn_pos` to template enemies. This works if positions are unique. **Recommendation**: keep inference for now (simpler, no protocol change); add `kind` to `EnemyState`/`EnemyStateResponse` only if inference proves fragile. Confirm with owner.

4. **`spawned_bullet_ids` persistence**: Client `HashSet<u16>` grows unbounded. Bullet IDs wrap at `u16::MAX`. Should we clear old IDs periodically, or is the space large enough (65535 bullets) that it doesn't matter? **Recommendation**: clear on room transition (since bullets are cleared anyway) and/or when set size > 1000. Confirm with owner.

5. **Client bullet physics vs server raycast discrepancy**: Client bullet moves at `BULLET_SPEED` over time; server raycast is instant hit-check at spawn tick. This means client bullet may visually pass through an enemy that the server already killed (or vice versa). Spec D4 edge case "Bullet passes through enemy (client prediction)" acknowledges this: client collision is purely visual; `enemy.die()` only on `EnemyDied`. **Confirm** this is acceptable — no interpolation or client-side hit prediction.

6. **Server `next_bullet_id` wrap behavior**: `u16` wraps at 65535. On wrap, client `spawned_bullet_ids` may have old IDs that collide with new ones. **Recommendation**: on wrap, server could send a `BulletIdReset` packet, or client clears `spawned_bullet_ids` on wrap detection (if `bullet_id < last_seen` and `last_seen > 60000`). For now, assume wrap won't happen in testing. Confirm if mitigation needed.

7. **`EnemyStateRequest` reliability**: Spec says reliable-ordered. Since it's a request-response, `reliable_unordered` would also work. **Recommendation**: keep `reliable_ordered` for consistency with other state-sync packets. Confirm.

---

Let me know if you need any clarification on the plan before the tester starts writing tests.