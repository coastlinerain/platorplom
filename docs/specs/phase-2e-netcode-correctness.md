# Phase 2e — Netcode Correctness: Server-Authoritative Bullet Spawn & Enemy Death

- Status: draft
- Date: 2026-11-25
- Branch: `feat/netcode-correctness`

## Summary

The owner found four bugs during manual testing that the e2e harness missed. The fourth and most critical — **"hit reverts"** (enemy appears to die on the shooter's screen, then comes back) — is the focus of this phase.

**Root cause:** enemies and bullets are client-authoritative (ADR-001 D9 scope boundary). The server never simulates bullets or enemies, so it cannot validate hits or track enemy death state. The client that fires a bullet locally decides "hit", kills the enemy locally, but the server has no concept of the bullet or the enemy's alive state. When the client later transitions rooms and re-loads enemies from map data, the dead enemy respawns. Worse, if the server ever gains authority over bullets, the local bullet's timing differs from what the server would confirm — causing the "hit reverts" visual glitch during reconciliation.

This phase introduces two server-authoritative fixes:

### Fix 1: Server-Authoritative Bullet Spawn
- The server confirms each shot with `BulletSpawn { bullet_id, x, y, dir, owner_id, seq }` broadcast to **ALL** clients (including the shooter).
- The shooter client **waits for `BulletSpawn`** before spawning its local bullet — it no longer spawns on `pressed_shoot`.
- Replaces the `Action { kind: "shoot" }` rebroadcast (which went to OTHER clients only).
- Fixes "hit reverts" — reconciliation now reconciles against server-confirmed bullet timing, not local speculative timing.

### Fix 2: Server-Authoritative Enemy Death
- The server validates hits: on `BulletSpawn`, the server raycasts the bullet path against server-side enemy hitboxes in that room.
- The server tracks enemy alive/dead state per room.
- The server broadcasts `EnemyDied { enemy_id, room_x, room_y }` to all clients in the room.
- On room load/death, the client requests `EnemyState` from the server and spawns **only alive** enemies.
- Fixes "enemy respawns on room change" and "hit reverts".

## Decisions

These are the product/trade-off calls the spec resolves. Each is stated with its rationale so the planner does not have to re-derive them.

### D1. Protocol — New Packet Types

**Decision:** add four new `GamePacket` variants. `Action { kind: "shoot" }` is **removed** entirely (client→server and server→client). `BulletSpawn` replaces it as the authoritative shot confirmation.

```rust
GamePacket::BulletSpawn {
    bullet_id: u16,      // server-assigned unique ID
    x: f32, y: f32,      // spawn position (server-authoritative)
    dir: f32,             // direction (-1.0 or 1.0)
    owner_id: u8,         // shooter player ID
    seq: u16,             // input seq this shot corresponds to
}

GamePacket::EnemyDied {
    enemy_id: u8,         // stable enemy ID (room_x * 16 + spawn_index)
    room_x: u8, room_y: u8,
}

GamePacket::EnemyStateRequest {         // client → server on room load
    room_x: u8, room_y: u8,
}

GamePacket::EnemyStateResponse {        // server → client
    room_x: u8, room_y: u8,
    enemies: Vec<(enemy_id: u8, alive: bool, spawn_x: f32, spawn_y: f32)>,
}
```

**Rationale:** the existing `Action { kind: "shoot" }` carried no position or seq, so it could not be reconciled. `BulletSpawn` includes the authoritative spawn position and the input `seq` it corresponds to, enabling the shooter client to reconcile correctly. `EnemyDied` is a minimal death notification. `EnemyStateRequest/Response` enables room-load sync without the server pushing full state on every tick.

**Rejected alternative:** keep `Action` for remote clients and add `BulletSpawn` only for the shooter. This would create two code paths for the same event and the shooter would still have no authoritative timing to reconcile against.

### D2. Server Enemy State

**Decision:** `ServerWorld` maintains `enemies: HashMap<(u8, u8), Vec<EnemyState>>` keyed by room coordinates `(room_x, room_y)`.

```rust
struct EnemyState {
    id: u8,                 // stable: room_x * 16 + spawn_index
    alive: bool,
    spawn_pos: (f32, f32),  // center of the enemy's hitbox
}
```

- Enemies are **static** (no AI movement) — only `alive` toggles to `false` on validated hit.
- On `ServerWorld::new()`, enemies are spawned from map data for **all rooms** (not just the current room). The server iterates `Map::new().grids` and builds the enemy state for every room.
- On `BulletSpawn` (triggered by a confirmed `shoot` input), the server raycasts the bullet path against all **alive** enemy hitboxes in that room.
- Raycast logic: the bullet travels horizontally from `(x, y)` in direction `dir`. An enemy is hit if its `Rect` (spawn_pos ± w/2, h/2) intersects the ray segment up to a max range (e.g., 400 px, matching client bullet lifetime). Only the first hit counts per bullet.
- When hit, the server sets `alive = false` for that enemy and broadcasts `EnemyDied` to all clients in that room (via `coords` map).

**Rationale:** the server must have the same enemy layout the client has. Since `Map::new()` is deterministic and shared, building all rooms on `ServerWorld::new()` ensures parity. Enemies don't move in current maps, so no per-tick simulation is needed — only hit validation and state persistence.

**Rejected alternative:** spawn enemies on-demand when the first player enters a room. Simpler but creates a race: the shooter's `BulletSpawn` might arrive before the server has spawned enemies for that room, requiring deferred hit checks. Pre-building all rooms is trivial (4 rooms × <20 enemies) and eliminates the race.

### D3. Client Bullet Spawn Flow

**Decision:** the client **no longer spawns a bullet on `pressed_shoot`**. Instead:

1. Client presses Z → `pressed_shoot = true` (edge).
2. `update_network` sends `PlayerInput { shoot: true, ... }` with `seq`.
3. **Client does NOT spawn bullet locally.**
4. Server receives input, cooldown check passes → assigns `bullet_id` (incrementing `u16`), raycasts against server enemy state.
5. Server broadcasts `BulletSpawn { bullet_id, x, y, dir, owner_id, seq }` via **reliable-ordered** channel to **ALL** clients (including shooter).
6. Client receives `BulletSpawn` → spawns local bullet at server's `(x, y)` with server's `dir`.
7. Client bullet physics runs locally for prediction (same as before: `Bullet::update`, collision with local enemies).

**Rationale:** waiting for the server's `BulletSpawn` gives the client the authoritative spawn position and timing. The `seq` field lets the shooter client match the `BulletSpawn` to the input it sent, so reconciliation knows exactly which input produced the bullet. The client still simulates bullet motion locally for visual responsiveness; the server does **not** simulate bullet physics (scope boundary D5).

**Edge case — packet loss:** `BulletSpawn` is sent reliable-ordered. If the packet is lost, laminar retransmits. The client sees a slight delay (1 RTT) before the bullet appears — this is the cost of authority. The shooter's `shoot` input uses sticky edges (3 frames) so the server is likely to receive it even under loss.

**Edge case — duplicate `BulletSpawn`:** the server assigns a unique `bullet_id` per confirmed shot. The client tracks spawned `bullet_id`s and ignores duplicates (defense against server re-broadcast bugs).

**Edge case — shooter dies before `BulletSpawn` arrives:** if the shooter receives `Hit` (death) before `BulletSpawn`, the local player respawns at (100,100) and clears bullets. The late `BulletSpawn` would spawn a bullet for a dead player — the client should ignore `BulletSpawn` where `owner_id` does not match any live player (or where `world.id != owner_id` for the local player). Simplest: the client only processes `BulletSpawn` for `owner_id == world.id` (own bullets) and remote `owner_id` (enemy bullets). If the local player is dead (`world.id` is still set but position reset), the bullet still spawns — this is acceptable; the bullet is harmless.

### D4. Client Enemy Death Flow

**Decision:** the client **no longer does local hit detection against own bullets for enemy death**. Instead:

1. Client receives `EnemyDied { enemy_id, room_x, room_y }`.
2. If `room_x, room_y` match `world.current_coords`, find the enemy in `world.enemies` by `enemy_id` (must match server's ID scheme — see D6).
3. Call `enemy.die()` — no local collision check needed.
4. `World::update` retains `enemies.retain(|e| e.alive)` as before.

**Rationale:** the server is the sole authority on whether a bullet hit an enemy. The client's job is to render the death when told. This eliminates the "hit reverts" bug because the client never speculatively kills an enemy — it only kills when the server confirms.

**Edge case — `EnemyDied` for enemy not in local list:** if the client hasn't loaded that room yet, or the enemy was already removed, ignore silently. The server's `EnemyStateResponse` on room load will reflect the correct alive state.

**Edge case — multiple `EnemyDied` for same enemy:** the client calls `die()` (sets `alive = false`) idempotently. Duplicate packets are harmless.

### D5. Room Load Sync

**Decision:** `load_current_level` no longer calls `enemies_from_data` locally. Instead:

1. `check_level_transitions` triggers room change → calls `load_current_level`.
2. `load_current_level` clears local enemies and bullets, then sends `EnemyStateRequest { room_x: current_coords.0 as u8, room_y: current_coords.1 as u8 }` via reliable-ordered.
3. Server receives request, looks up its `enemies` map for that room, and responds with `EnemyStateResponse` containing `(enemy_id, alive, spawn_x, spawn_y)` for every enemy in that room.
4. Client receives `EnemyStateResponse`, spawns `Enemy::new(Vec2::new(spawn_x, spawn_y), kind)` for each `alive` entry. The `kind` is inferred from the spawn position matching the original map data (or the server could include `kind` in the response — see D6).

**Rationale:** this ensures the client's enemy population exactly matches the server's authoritative alive/dead state. Dead enemies stay dead across room transitions.

**Rejected alternative:** server pushes `EnemyStateResponse` on every room change proactively. The request/response model is simpler — the client controls when it's ready to receive the state (after clearing local lists).

**Edge case — request lost:** reliable-ordered guarantees delivery. If the response is delayed, the room loads with zero enemies until it arrives — the client will see enemies pop in. This is a brief visual glitch, not a correctness issue. A loading indicator could be added later.

**Edge case — client enters room, shoots, transitions back before `EnemyDied` arrives:** the server has already marked the enemy dead. The `EnemyStateResponse` on return will show `alive: false`. The late `EnemyDied` is ignored (enemy not in local list). Correct.

### D6. Enemy ID Scheme

**Decision:** `enemy_id = room_x * 16 + spawn_index`, where:
- `room_x`: 0..3 (column in the 4×4 grid).
- `spawn_index`: the 0-based index of the enemy spawn cell in row-major order within that room's map data (max 16 per room, fits in `u8`).

The server assigns IDs on `ServerWorld::new()` by iterating all rooms' map data in deterministic order (row-major y, then x). The same order is used by `enemies_from_data` in the client. This makes the ID stable across room loads — the same enemy in the same room always has the same ID.

**Rationale:** a stable ID lets the client match `EnemyDied` and `EnemyStateResponse` entries to local enemies without needing a full entity registry. The `room_x * 16 + index` scheme fits in `u8` (max 4*16=64) and is trivial to compute.

**Rejected alternative:** UUIDs or server-assigned incremental IDs. Overkill for static enemies in a fixed map.

**Note on `kind`:** the server's `EnemyState` currently does not store `kind` (Enemy/Potion). The client can infer `kind` by re-running `enemies_from_data` for that room and matching by `spawn_pos` (which is the tile center). Since spawn positions are unique per room, this works. Alternatively, add `kind: EnemyKind` to `EnemyState` and `EnemyStateResponse` — this is a one-line change if preferred.

### D7. Server Bullet Cooldown & Confirmation

**Decision:** the server's existing per-player `shoot_cooldown` (5 ticks at 20 Hz = 250 ms) remains. A `BulletSpawn` is only broadcast when the cooldown is 0 and a latched `shoot` input is present. The `seq` in `BulletSpawn` is the `seq` of the input that triggered the shot.

**Rationale:** the cooldown prevents multi-fire. The `seq` enables the shooter client to reconcile: when the server's `PlayerPos` arrives with `last_seq`, the client knows which inputs are acknowledged. The `BulletSpawn.seq` tells it exactly which input produced the bullet, so it can verify the bullet timing matches the acknowledged input.

**Edge case — input arrives, cooldown > 0:** the server's input accumulator OR-latches `shoot`. If cooldown > 0, the latched `shoot` is consumed but no `BulletSpawn` is emitted. The client's sticky edge (3 frames) means the `shoot` flag may arrive again on the next tick, at which point cooldown may be 0 and the shot fires. This is correct behavior — the shot fires on the first tick where cooldown permits.

### D8. Scope Boundaries (Explicit Non-Goals)

This phase does **not** do:
- **Server-authoritative enemy AI movement** (Phase 2f). Enemies remain static on the server; the client still simulates enemy AI locally for visual movement. The server only tracks alive/dead.
- **Server-side bullet physics**. The server raycasts on `BulletSpawn` (instant hit check) but does not simulate bullet travel over time. The client still runs `Bullet::update` locally for prediction.
- **Enemy position interpolation**. Enemies don't move in current maps, so no interpolation is needed. If Phase 2f adds server enemy AI, interpolation will be needed then.
- **Authoritative room transitions** (ADR-001 open question). Room coords in `PlayerInput` remain client-trusted.

## Goals / Non-Goals

### Goals

1. **New protocol packets:** `BulletSpawn`, `EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse` in `protocol/src/lib.rs`; `Action` variant removed.
2. **Server enemy state:** `ServerWorld` builds enemy state for all rooms on `new()`, tracks `alive` per enemy, validates hits on `BulletSpawn` via raycast.
3. **Server bullet confirmation:** on confirmed `shoot` input, server assigns `bullet_id`, raycasts, broadcasts `BulletSpawn` (reliable-ordered) to all clients.
4. **Client waits for `BulletSpawn`:** local bullet spawn moves from `World::update` (on `pressed_shoot`) to `handle_packet` (on `BulletSpawn`).
5. **Client enemy death from server:** `handle_packet` processes `EnemyDied`, calls `enemy.die()` by `enemy_id`.
6. **Room load sync:** `load_current_level` sends `EnemyStateRequest`, spawns only `alive` enemies from `EnemyStateResponse`.
7. **Reconciliation uses server bullet timing:** the shooter client matches `BulletSpawn.seq` to its input buffer, so reconciliation knows the authoritative bullet spawn frame.
8. **Clean quality gates:** `cargo build/test/clippy/fmt` pass.

### Non-Goals (Explicitly Out of Scope)

- Server-authoritative enemy AI movement.
- Server-side bullet physics (travel over time, collision with walls).
- Enemy position interpolation.
- Authoritative room transitions.
- Potion/collectible logic (enemies with `kind == Potion` are still treated as enemies for death tracking; collectible behavior is a separate feature).
- Anti-cheat beyond the authority model (no input plausibility checks).

## Behavior

### Server

- **On start:** `ServerWorld::new()` builds `enemies: HashMap<(u8,u8), Vec<EnemyState>>` by iterating `Map::new().grids` for all rooms. For each `EntityType::EnemySpawn` or `EntityType::PotionSpawn`, create `EnemyState { id: room_x * 16 + spawn_index, alive: true, spawn_pos: (x * TILE_SIZE + TILE_SIZE/2, y * TILE_SIZE + TILE_SIZE/2) }`. Store in `enemies[(room_x, room_y)]`.
- **On `PlayerInput` with `shoot: true`:** the existing input accumulator latches the edge. On the next tick, if `shoot_cooldown[id] == 0`, the server:
  1. Gets the player's authoritative `pos`, `dir`, and current room `(level_x, level_y)`.
  2. Assigns `bullet_id = next_bullet_id++` (u16, wrapping).
  3. Raycasts: from `(player.pos.x + player.w/2, player.pos.y + player.h/2)` in direction `dir` (horizontal). For each alive enemy in `enemies[(level_x, level_y)]`, check if the ray intersects the enemy's `Rect` (spawn_pos ± 15, 19). Take the first hit (closest).
  4. If hit: set `enemy.alive = false`; broadcast `EnemyDied { enemy_id, room_x: level_x, room_y: level_y }` to all clients in that room (via `coords` map).
  5. Broadcast `BulletSpawn { bullet_id, x: spawn_x, y: spawn_y, dir, owner_id: id, seq: player.max_seq }` to **all** connected clients (reliable-ordered).
  6. Set `shoot_cooldown[id] = 5`.
- **On `EnemyStateRequest { room_x, room_y }`:** look up `enemies[(room_x, room_y)]`, build `Vec<(enemy_id, alive, spawn_x, spawn_y)>`, send `EnemyStateResponse` to the requesting client (reliable-ordered).
- **Broadcasts:** `PlayerPos` unchanged (includes `last_seq`). `Action { kind: "shoot" }` is **no longer sent**.
- **Join/Leave/Timeout:** unchanged.

### Client: Sending Inputs

- `update_network` unchanged: sends `PlayerInput { shoot: sticky_shoot, ... }` with `seq` on unreliable channel. The sticky edge (3 frames) ensures the `shoot` edge reaches the server.
- **Critical:** `World::update` **no longer** spawns a bullet on `pressed_shoot`. The `pressed_shoot` flag is still sampled and sent via `shoot_sticky`, but the bullet spawn is deferred to `BulletSpawn` receipt.

### Client: Receiving `BulletSpawn`

In `handle_packet`:
```rust
GamePacket::BulletSpawn { bullet_id, x, y, dir, owner_id, seq } => {
    // Ignore duplicates
    if world.spawned_bullet_ids.contains(&bullet_id) { return; }
    world.spawned_bullet_ids.insert(bullet_id);

    let spawn_pos = glam::Vec2::new(x, y);
    if Some(owner_id) == world.id {
        // Own bullet: spawn at server-authoritative position
        world.bullets.push(Bullet::new(spawn_pos, dir, owner_id));
        // Reconciliation: the input buffer entry with this seq now has a confirmed bullet.
        // The prediction module can use this to verify timing.
    } else {
        // Remote player's bullet: spawn as enemy_bullet
        world.enemy_bullets.push(Bullet::new(spawn_pos, dir, owner_id));
    }
}
```
- The client maintains `spawned_bullet_ids: HashSet<u16>` to deduplicate.
- **Reconciliation integration:** when a `PlayerPos` arrives with `last_seq`, the reconciliation replay knows that any input with `seq <= last_seq` is acknowledged. The `BulletSpawn.seq` matches the input that produced the shot. The client can assert that a bullet was spawned for that `seq` (or will be spawned when the `BulletSpawn` arrives). If a `BulletSpawn` arrives for a `seq` that was NOT acknowledged (i.e., the server processed a newer input than the client knows about), it's a future bullet — the client spawns it immediately. This is correct.

### Client: Receiving `EnemyDied`

In `handle_packet`:
```rust
GamePacket::EnemyDied { enemy_id, room_x, room_y } => {
    if room_x as usize == world.current_coords.0 && room_y as usize == world.current_coords.1 {
        if let Some(enemy) = world.enemies.iter_mut().find(|e| e.id == enemy_id) {
            enemy.die();
        }
    }
}
```
- Requires `Enemy` to have an `id: u8` field (currently it does not — see Data Model).

### Client: Room Load (`load_current_level`)

```rust
fn load_current_level(world: &mut World) {
    world.enemies.clear();
    world.bullets.clear();
    world.enemy_bullets.clear();

    // Request authoritative enemy state from server
    let (rx, ry) = world.current_coords;
    let req = GamePacket::EnemyStateRequest {
        room_x: rx as u8,
        room_y: ry as u8,
    };
    world.network.send(req);

    // Response handled in handle_packet -> spawns enemies
}
```

In `handle_packet`:
```rust
GamePacket::EnemyStateResponse { room_x, room_y, enemies } => {
    if room_x as usize == world.current_coords.0 && room_y as usize == world.current_coords.1 {
        // Re-run enemies_from_data to get kinds, then match by spawn_pos
        let level = &world.levels_matrix[room_y][room_x];
        let template_enemies = enemies_from_data(&level.data);
        // template_enemies is in deterministic spawn order; enemies in response
        // are in the same order (server built them the same way).
        for (i, (enemy_id, alive, spawn_x, spawn_y)) in enemies.into_iter().enumerate() {
            if alive && i < template_enemies.len() {
                let mut enemy = template_enemies[i];
                enemy.id = enemy_id;  // set stable ID
                world.enemies.push(enemy);
            }
        }
    }
}
```

### Edge Cases

| Edge Case | Behavior |
|-----------|----------|
| `BulletSpawn` lost (reliable-ordered) | Laminar retransmits; client sees bullet after 1 RTT delay. Sticky edge ensures server receives `shoot`. |
| `BulletSpawn` for dead player | Client ignores if `owner_id` not in `other_players` and `owner_id != world.id`. (Or spawns anyway — harmless.) |
| `EnemyDied` before room load | Client ignores (room mismatch). Room load `EnemyStateResponse` shows `alive: false`. |
| `EnemyDied` duplicate | `enemy.die()` is idempotent (`alive = false`). |
| Room load response delayed | Room renders with zero enemies until response arrives; enemies pop in. |
| Shooter transitions room before `BulletSpawn` arrives | Server uses shooter's reported room at tick time. `BulletSpawn` carries that room's coords implicitly via `owner_id`'s coords. If client is in different room when `BulletSpawn` arrives, it spawns the bullet but it won't hit anything (wrong room). The bullet despawns on wall collision. |
| Server raycast hits multiple enemies | Only the first (closest) hit counts. `BulletSpawn` still broadcasts; `EnemyDied` for the hit enemy. |
| Bullet passes through enemy (client prediction) | Client bullet may visually pass through an enemy that the server didn't hit (different timing/position). Server's `EnemyDied` is authoritative — the enemy dies when the server says so. The client's local bullet collision is purely visual; `enemy.die()` is only called on `EnemyDied`. |

## Data Model

### `protocol/src/lib.rs` — `GamePacket`

```rust
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    JoinRequest,
    JoinResponse { assigned_id: u8 },
    PlayerPos {
        id: u8,
        x: f32,
        y: f32,
        dir: f32,
        level_x: u8,
        level_y: u8,
        last_seq: u16,
    },
    // Action REMOVED
    Leave { id: u8 },
    Hit { entity_id: u8 },
    PlayerInput {
        id: u8,
        level_x: u8,
        level_y: u8,
        move_left: bool,
        move_right: bool,
        jump: bool,
        shoot: bool,
        seq: u16,
    },
    // --- NEW ---
    BulletSpawn {
        bullet_id: u16,
        x: f32,
        y: f32,
        dir: f32,
        owner_id: u8,
        seq: u16,
    },
    EnemyDied {
        enemy_id: u8,
        room_x: u8,
        room_y: u8,
    },
    EnemyStateRequest {
        room_x: u8,
        room_y: u8,
    },
    EnemyStateResponse {
        room_x: u8,
        room_y: u8,
        enemies: Vec<(u8, bool, f32, f32)>,  // (enemy_id, alive, spawn_x, spawn_y)
    },
}
```

### `server/src/world.rs` — `ServerWorld`

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

    // --- NEW ---
    enemies: HashMap<(u8, u8), Vec<EnemyState>>,  // key: (room_x, room_y)
    next_bullet_id: u16,
}

pub struct EnemyState {
    pub id: u8,
    pub alive: bool,
    pub spawn_pos: (f32, f32),  // center of hitbox
}

impl ServerWorld {
    pub fn new() -> Self { /* build levels + enemies for all rooms */ }
    pub fn spawn(&mut self, id: u8) { /* ... */ }
    pub fn remove(&mut self, id: u8) { /* ... */ }
    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16) { /* ... */ }
    /// Returns (shots, enemy_deaths) where shots = Vec<(bullet_id, x, y, dir, owner_id, seq)>
    /// and enemy_deaths = Vec<(enemy_id, room_x, room_y)>.
    pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>) { /* ... */ }
    pub fn get_enemy_state(&self, room_x: u8, room_y: u8) -> Vec<(u8, bool, f32, f32)> { /* ... */ }

    // Accessors for main.rs
    pub fn players(&self) -> &HashMap<u8, Player> { &self.players }
    pub fn coords(&self) -> &HashMap<u8, (u8, u8)> { &self.coords }
    pub fn max_seq(&self) -> &HashMap<u8, u16> { &self.max_seq }
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
```

**Raycast logic in `update`:**
- For each player whose `shoot` was latched and cooldown == 0:
  - `spawn_x = player.pos.x + player.w / 2.0`
  - `spawn_y = player.pos.y + player.h / 2.0`
  - `dir = player.dir`
  - Get room `(rx, ry) = coords[&id]`
  - Iterate `enemies.get(&(rx, ry))` (alive only), check ray-rect intersection.
  - Ray: horizontal segment from `(spawn_x, spawn_y)` to `(spawn_x + dir * 400.0, spawn_y)`.
  - Rect: `Rect::new(enemy.spawn_pos.0 - 15.0, enemy.spawn_pos.1 - 19.0, 30.0, 38.0)`.
  - First hit wins. Set `enemy.alive = false`.
  - Emit `BulletSpawnInfo` and (if hit) `EnemyDiedInfo`.

### `game/src/entities/enemy.rs` — `Enemy`

Add stable `id` field:

```rust
#[derive(Debug)]
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
    // --- NEW ---
    pub id: u8,  // stable enemy ID (room_x * 16 + spawn_index)
}
```

`enemies_from_data` assigns `id` based on spawn order (same as server).

### `client/src/world/game.rs` — `World`

Add fields:

```rust
pub struct World {
    // ... existing fields ...
    pub spawned_bullet_ids: std::collections::HashSet<u16>,  // deduplicate BulletSpawn
    // Enemy now has `id` field
}
```

`World::new()` initializes `spawned_bullet_ids: HashSet::new()`.

`World::update`:
- **Removes** the `if pressed_shoot { spawn bullet }` block (lines 136-142 in current code).
- Keeps own bullet `update` and collision with local enemies — but collision only does visual effects (particles, etc.), NOT `enemy.die()`. The `enemy.die()` call moves to `handle_packet` on `EnemyDied`.
- Actually: the client's own bullets should still collide with local enemies for **visual feedback** (hit flash, knockback?), but the authoritative death is `EnemyDied`. To keep it simple: the client's bullet collision loop is removed entirely for enemies. The bullet still updates and hits walls. When `EnemyDied` arrives, the enemy dies. This eliminates the "hit reverts" discrepancy completely.

**Revised:** remove the own-bullet vs enemy collision loop in `World::update` (lines 150-158). The bullet still flies and hits walls. Enemy death is purely server-driven.

### `client/src/network/world.rs` — `handle_packet`

Add cases for `BulletSpawn`, `EnemyDied`, `EnemyStateResponse`.

### `client/src/world/lib.rs` — `load_current_level`

Replaced with `EnemyStateRequest` flow.

## Network

### Channel Usage

| Packet | Direction | Channel | Notes |
|--------|-----------|---------|-------|
| `PlayerInput` | client→server | `unreliable` | unchanged from 2c |
| `PlayerPos` | server→client | `unreliable` | unchanged |
| `BulletSpawn` | server→client | **`reliable_ordered`** | all clients including shooter |
| `EnemyDied` | server→client | **`reliable_ordered`** | clients in same room |
| `EnemyStateRequest` | client→server | **`reliable_ordered`** | on room load |
| `EnemyStateResponse` | server→client | **`reliable_ordered`** | reply to request |
| `JoinRequest`/`Response` | both | `reliable_ordered` | unchanged |
| `Leave` | server→client | `reliable_ordered` | unchanged |
| `Hit` | client→server | `reliable_ordered` | unchanged |

**Removed:** `Action { kind: "shoot" }` (was server→client reliable-ordered).

### Packet Sizes (postcard, approximate)

| Packet | Fields | Est. Size |
|--------|--------|-----------|
| `BulletSpawn` | u16 + 2×f32 + f32 + u8 + u16 | ~18 bytes |
| `EnemyDied` | u8 + 2×u8 | ~3 bytes |
| `EnemyStateRequest` | 2×u8 | ~2 bytes |
| `EnemyStateResponse` | 2×u8 + vec of (u8, bool, 2×f32) | ~2 + N×13 bytes |

Negligible for 4 rooms × <20 enemies.

## Acceptance Criteria

Each item is a checkable statement of "done"; not an implementation plan.

### Protocol

1. `protocol/src/lib.rs` defines the four new `GamePacket` variants (`BulletSpawn`, `EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse`) with exactly the fields specified in D1.
2. `GamePacket::Action` variant is **removed** from the enum.
3. A `protocol` round-trip test serializes/deserializes each new variant with representative values; all fields preserved.
4. `protocol/Cargo.toml` declares no dependency on `game` or `glam`.

### Server Enemy State & Hit Validation

5. `ServerWorld::new()` builds `enemies: HashMap<(u8,u8), Vec<EnemyState>>` for all rooms from `Map::new()`; each `EnemyState` has `id = room_x * 16 + spawn_index`, `alive = true`, `spawn_pos` at tile center.
6. A server unit test: given a room with an enemy at a known tile, a `BulletSpawn` raycast from the left traveling right hits that enemy and sets `alive = false`.
7. A server unit test: a `BulletSpawn` raycast that passes above/below an enemy does NOT hit it.
7. A server unit test: two enemies in line; only the closer one is hit (first hit wins).
8. `ServerWorld::update` returns `Vec<BulletSpawnInfo>` and `Vec<EnemyDiedInfo>`; `main.rs` broadcasts both.

### Server Bullet Confirmation

9. On a confirmed `shoot` input (cooldown 0), `ServerWorld::update` emits a `BulletSpawnInfo` with `bullet_id` (incrementing u16), spawn position from the authoritative player center, `dir`, `owner_id`, and the input `seq`.
10. `main.rs` broadcasts `BulletSpawn` (reliable-ordered) to **ALL** connected clients (including the shooter).
11. `Action { kind: "shoot" }` is no longer sent by the server (code path removed).
12. A server unit test: two `shoot` inputs within 5 ticks → only the first produces a `BulletSpawnInfo` (cooldown enforced).

### Client Bullet Spawn Flow

13. `client/src/world/game.rs` `World::update` **does not** spawn a bullet on `pressed_shoot` (the `if pressed_shoot { ... }` block is removed).
14. `client/src/network/world.rs` `handle_packet` on `BulletSpawn` spawns a local `Bullet` at the server's `(x, y)` with server's `dir` for both own bullets (`owner_id == world.id`) and remote bullets (`owner_id != world.id` → `enemy_bullets`).
15. The client maintains `spawned_bullet_ids: HashSet<u16>` and ignores duplicate `BulletSpawn` packets.
16. An e2e test: Client A presses Z → server broadcasts `BulletSpawn` → Client A and Client B both have a bullet at the same server-authoritative position.

### Client Enemy Death Flow

17. `game/src/entities/enemy.rs` `Enemy` gains `id: u8` field; `enemies_from_data` assigns sequential IDs per room (0, 1, 2... matching server's `room_x * 16 + index`).
18. `handle_packet` on `EnemyDied` finds the enemy by `enemy_id` in `world.enemies` (only if room matches) and calls `enemy.die()`.
19. The own-bullet vs enemy collision loop in `World::update` is **removed** (no local `enemy.die()` on bullet collision).
20. An e2e test: Client A shoots enemy → server validates hit → `EnemyDied` broadcast → enemy dies on Client A AND Client B.

### Room Load Sync

21. `load_current_level` clears local enemies/bullets and sends `EnemyStateRequest` (reliable-ordered) for the new room coords.
22. Server `main.rs` handles `EnemyStateRequest`, replies with `EnemyStateResponse` containing `(enemy_id, alive, spawn_x, spawn_y)` for all enemies in that room.
23. Client `handle_packet` on `EnemyStateResponse` re-runs `enemies_from_data` for the room, matches by spawn order, sets `enemy.id = enemy_id`, and spawns only `alive` enemies.
24. An e2e test: Client A kills enemy in room (0,0) → Client A transitions to (1,0) and back to (0,0) → the enemy stays dead (not respawned).

### Reconciliation Integration

25. The shooter client's reconciliation logic (in `handle_packet` on `PlayerPos`) uses `BulletSpawn.seq` to verify bullet timing: when a `PlayerPos` acknowledges `seq`, the client knows the bullet for that `seq` is confirmed. If a `BulletSpawn` arrives for an unacknowledged `seq`, it's a future bullet — spawn it immediately. No position snap on shoot.
26. An e2e test: Client A moves right, shoots (seq 10) → server confirms with `BulletSpawn(seq=10)` → Client A's reconciliation on `PlayerPos(last_seq=10)` shows no position correction related to the shot.

### Regression (Phase 2d Fix)

27. No duplicate `JoinRequest` (the `World::new` fix from Phase 2d remains).

### Quality Gates

28. `cargo build --workspace` succeeds (all four crates).
29. `cargo test --workspace` passes (new unit tests + e2e tests + all existing tests).
30. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
31. `cargo fmt --check --all` reports no diffs.
32. `grep -rn "unwrap()\|expect(" server/src/world.rs client/src/network/world.rs` finds zero uses in new logic (all `Option`/`Result` handling explicit).

---

## Files to Modify (Reference)

- `protocol/src/lib.rs` — new packet variants, remove `Action`
- `server/src/world.rs` — `EnemyState`, `enemies` map, raycast hit validation in `update`, `get_enemy_state`
- `server/src/main.rs` — handle `EnemyStateRequest`, broadcast `BulletSpawn` to all, broadcast `EnemyDied` to room clients
- `game/src/entities/enemy.rs` — add `id` field, assign in `enemies_from_data`
- `client/src/network/world.rs` — `handle_packet` cases for `BulletSpawn`, `EnemyDied`, `EnemyStateResponse`
- `client/src/world/game.rs` — remove local bullet spawn on `pressed_shoot`, remove own-bullet vs enemy collision loop, add `spawned_bullet_ids`
- `client/src/world/lib.rs` — `load_current_level` → `EnemyStateRequest` flow

---

## Follow-up (Phase 2f)

- Server-authoritative enemy AI (deterministic RNG, per-tick decisions).
- Server-side bullet physics (if needed for wall collision authority).
- Potion collectible logic (replace `EnemyKind::Potion` with a real `Collectible` entity).
- Authoritative room transitions (server validates `level_x`/`level_y`).