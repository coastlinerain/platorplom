# Remove PvE enemies — Implementation Plan

- Status: implemented (2026-11-26; 178 passed, 3 ignored)
- Spec: `docs/specs/remove-pve-enemies.md`
- Date: 2026-11-26
- Branch: `refactor/remove-pve-enemies`
- Tests: 194 passed (8 ignored) — `cargo test --workspace` green **before** this refactor

## Overview

Pure **removal + one rename** refactor (spec 1 of the netcode-hardening
roadmap). The PvE `Enemy` concept is deleted end-to-end across `protocol`,
`server`, `game`, `client`, and `e2e`; the PvP field `enemy_bullets` is renamed
`remote_bullets` (and `update_enemy_bullets_position` →
`update_remote_bullets_position`). No new types, no protocol additions, no wire
migration, no behavior change to PvP.

There is **no new logic to test** — the acceptance criteria are "existing tests
pass + grep clean". This is a **single-box plan**: one implementer box does the
removal, updates/deletes tests, and verifies the grep acceptance. There is no
separate tester step (nothing to write RED for — removal only).

**Baseline:** `cargo test --workspace` is green at 194 tests *before* any edit.
Any test failure *after* Box 1 is a regression introduced by this change, not a
pre-existing condition. The implementer must run the full quality gate at the
end and confirm the suite returns to green.

## Non-goals (hard constraints — do NOT do these)

- Do **not** re-add any hit detection. The server now has no hit-detection
  target (see "Known consequence" below); this is accepted and temporary, not a
  bug to fix here.
- Do **not** touch `Nemesis` / `World.other_players`, `remote_bullets`
  (formerly `enemy_bullets`, beyond the rename itself), `check_player_death`'s
  collision loop against them, or the `BulletSpawn` flow.
- Do **not** remove `EntityType` / `TileType` enum variants
  (`EnemySpawn`, `PotionSpawn`, `BossSpawn`) or any map data in
  `game/src/maps.rs` / `game/src/types.rs`. They are inert data that stays.
- Do **not** rename `Hit` → `PlayerDied` (separate spec:
  `feat/authoritative-player-death`).
- Do **not** touch `protocol/` wire format beyond deleting the three PvE
  variants; no renumbering, no `PlayerInput.level_x/level_y` removal (that is
  spec 3).

## Known consequence (accepted — do NOT "fix")

After removal, a client "shoot" still latches on the server and emits a
`BulletSpawn` (broadcast to all clients; spawn point = shooter center), but the
server no longer tests that ray against anything, so nothing is hit and no
`EnemyDied` is ever produced. This is a **consequence**, not a bug. The next
spec (`feat/authoritative-player-death`) re-adds server-authoritative PvP hit
detection.

## Public API changes (targets)

```rust
// server/src/world.rs — ServerWorld::update return type changes
// BEFORE: pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)
pub fn update(&mut self, dt: f32) -> Vec<BulletSpawnInfo>
```

```rust
// server/src/world.rs — the internal ShootRequest struct drops one field
struct ShootRequest {
    player_id: u8,
    spawn_x: f32,
    spawn_y: f32,
    dir: f32,
    // room_coords: (u8, u8),   ← DELETED (only raycast_hit used it)
    player_seq: u16,
}
```

```rust
// server/src/lib.rs — re-export change (edit line 3 only)
// line 2 `pub use world::ServerWorld;` is ALREADY present and stays untouched.
// BEFORE (line 3): pub use world::{BulletSpawnInfo, EnemyDiedInfo, EnemyState};
pub use world::BulletSpawnInfo;   // drop EnemyDiedInfo / EnemyState; do NOT re-add ServerWorld
```

```rust
// client/src/world/game.rs + client/src/world/lib.rs — the rename
// World.enemy_bullets: Vec<Bullet>        → World.remote_bullets: Vec<Bullet>
// pub fn update_enemy_bullets_position(...) → pub fn update_remote_bullets_position(...)
```

Deleted public/type items (must not remain anywhere):

| Item | Location | Fate |
| --- | --- | --- |
| `GamePacket::{EnemyDied, EnemyStateRequest, EnemyStateResponse}` | `protocol/src/lib.rs` | removed |
| `server::EnemyState` (**struct**) | `server/src/world.rs` | removed |
| `server::EnemyDiedInfo` | `server/src/world.rs` | removed |
| `ServerWorld.enemies` | `server/src/world.rs` | removed |
| `ServerWorld::{build_enemies_for_room, raycast_hit, get_enemy_state}` | `server/src/world.rs` | removed |
| `game::entities::enemy::{Enemy, EnemyState (enum), EnemyKind, enemies_from_data}` | `game/src/entities/enemy.rs` | removed (file deleted) |
| `World.enemies: Vec<Enemy>` | `client/src/world/game.rs` | removed |
| `World.pending_enemy_deaths: HashSet<u8>` | `client/src/world/game.rs` | removed |
| `impl Drawable for Enemy` (+ `EnemyKind -> Color` mapping) | `client/src/render.rs` | removed |

## Implementation checklist

### Box 1 — Remove the PvE enemy machinery + rename `enemy_bullets` → `remote_bullets`

- [x] done
- **Files**: edit `protocol/src/lib.rs`; edit `protocol/tests/round_trip.rs`;
  edit `server/src/world.rs`; edit `server/src/lib.rs`; edit `server/src/main.rs`;
  delete `game/src/entities/enemy.rs`; edit `game/src/entities/mod.rs`;
  edit `client/src/world/game.rs`; edit `client/src/world/lib.rs`;
  edit `client/src/network/world.rs`; edit `client/src/render.rs`;
  edit `client/src/network/logging.rs`; edit `client/src/network/mod.rs`;
  edit `e2e/src/harness.rs`; delete `e2e/tests/room_load.rs`;
  delete `e2e/tests/enemy_death.rs`; edit `e2e/tests/edge_cases_2e.rs`;
  edit `e2e/tests/bullet_spawn.rs`.

- **Change** (file by file — work from this list alone):

  - `protocol/src/lib.rs`
    - Remove the three `GamePacket` variants: `EnemyDied { enemy_id, room_x,
      room_y }`, `EnemyStateRequest { room_x, room_y }`, and
      `EnemyStateResponse { room_x, room_y, enemies }`.
    - Leave every other variant unchanged (`JoinRequest`, `JoinResponse`,
      `PlayerPos`, `Leave`, `Hit`, `PlayerInput`, `BulletSpawn`).

  - `protocol/tests/round_trip.rs`
    - Delete the three tests `round_trip_enemydied`,
      `round_trip_enemystaterequest`, `round_trip_enemystateresponse`.
    - Keep `round_trip_bulletspawn` and all other round-trip tests.

  - `server/src/world.rs`
    - Delete `pub struct EnemyState` (lines 8–13) and `pub struct EnemyDiedInfo`
      (lines 25–30).
    - Delete the `enemies: HashMap<(u8, u8), Vec<EnemyState>>` field on
      `ServerWorld` (line 40) and its construction in `ServerWorld::new` (the
      `let mut enemies = ...` + `for room_y { for room_x { ... } }` pre-build
      loop, lines 51–59) and the `enemies,` initializer (line 81).
    - Delete `fn build_enemies_for_room` (lines 90–113), `fn raycast_hit`
      (lines 275–352), and `fn get_enemy_state` (lines 356–368).
    - Rewrite `ServerWorld::update`:
      - Return type `(Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)` →
        `Vec<BulletSpawnInfo>`.
      - Drop the `let mut enemy_deaths = Vec::new();` local.
      - Drop the `room_coords: (u8, u8)` field from the local `ShootRequest`
        struct, and the `let room_coords = self.coords.get(&id)...` binding
        (line 198) plus the `room_coords,` initializer (line 206).
      - Second pass: emit `BulletSpawnInfo { bullet_id, x: req.spawn_x,
        y: req.spawn_y, dir: req.dir, owner_id: req.player_id,
        seq: req.player_seq }` with **no** raycast and **no** hit/death branch.
        Bullet-ID assignment (`self.next_bullet_id` + `wrapping_add`) and
        cooldown logic are unchanged.
      - Final `return` is `bullet_spawns` only.
    - In the test region **after** `mod tests` (lines 625–945): these are
      **top-level `#[test]` fns**, not a `#[cfg(test)]` block or a `mod`, so
      edit them individually — do not delete the region wholesale.
      - Delete the stale comment header `// Phase 2e — Server Enemy State & Hit
        Validation (AC 5, 6, 7, 8)` (it no longer applies once the enemy tests
        are gone; it contains no grep token, but it is stale).
      - Delete the **5 enemy tests** (each references `EnemyState`,
        `world.enemies`, or `EnemyDiedInfo`): `serverworld_builds_enemies_all_rooms`,
        `raycast_hits_enemy`, `raycast_miss_above_below`,
        `raycast_first_hit_wins`, `update_returns_shots_and_deaths`.
      - **Keep** `shoot_emits_bulletspawninfo` and `shoot_cooldown_enforced`
        (pure PvP `BulletSpawn`; they reference neither `world.enemies` nor
        `EnemyState` and are the only tests covering bullet-ID increment,
        spawn-position-from-center, `seq` propagation, and cooldown
        enforcement). Fix only their tuple destructuring:
        - `shoot_emits_bulletspawninfo` — `let (shots, _deaths) =
          world.update(1.0 / 60.0)` → `let shots = world.update(1.0 / 60.0)`.
        - `shoot_cooldown_enforced` — `let (shots1, _) = world.update(1.0 /
          60.0)` → `let shots1 = world.update(1.0 / 60.0)`; `let (shots, _) =
          world.update(1.0 / 60.0)` → `let shots = ...`; `let (shots6, _) =
          world.update(1.0 / 60.0)` → `let shots6 = ...`.
        - Their assertions (bullet_id, spawn pos, dir, owner_id, seq, cooldown)
          are unchanged.
    - Keep the first `mod tests` block (movement/input/max_seq) intact, but fix
      the two tests that destructure the two-tuple return:
      - `input_edge_flags_latch_and_clear` — `let (shots, _deaths) =
        world.update(tick_dt())` → `let shots = world.update(tick_dt())`, and
        `let (shots2, _deaths2) = world.update(tick_dt())` → `let shots2 =
        world.update(tick_dt())` (two call sites).
      - `input_edge_flag_not_lost_between_ticks` — `let (shots, _deaths) =
        world.update(tick_dt())` → `let shots = world.update(tick_dt())`.
      - Their assertions (shot count, owner, dir) are unchanged.

  - `server/src/lib.rs`
    - Line 3 `pub use world::{BulletSpawnInfo, EnemyDiedInfo, EnemyState};` →
      `pub use world::BulletSpawnInfo;` — drop only `EnemyDiedInfo` /
      `EnemyState`. Line 2 (`pub use world::ServerWorld;`) is already present
      and stays **untouched** — do **not** re-add `ServerWorld` to line 3 (that
      would duplicate the line-2 re-export and trip E0252).

  - `server/src/main.rs`
    - Remove the `GamePacket::EnemyDied`, `GamePacket::EnemyStateRequest`,
      `GamePacket::EnemyStateResponse` arms from `packet_type_name` and
      `packet_key_fields` (the catch-alls already cover the removed variants).
    - In `packet_id`, remove **only** the `GamePacket::EnemyDied` arm (lines
      57–61) — `EnemyStateRequest`/`EnemyStateResponse` have no arm there and
      fall through `_ => None`.
    - Remove the `GamePacket::EnemyStateRequest { room_x, room_y }` handling arm
      in the event loop (lines 265–277, incl. its `get_enemy_state` call and
      `EnemyStateResponse` reply).
    - `let (shots, deaths) = world.update(tick_dt);` → `let shots =
      world.update(tick_dt);` (line 308).
    - Delete the "Broadcast EnemyDied to clients in the same room" block
      (lines 355–374: the `for death in &deaths { ... }` loop).

  - `game/src/entities/enemy.rs`
    - Delete the **whole file**: `Enemy`, the `EnemyState` enum (client/game
      side), `EnemyKind`, `enemies_from_data`, the `impl Colisionable for
      Enemy`, and the file's `#[cfg(test)]` module (4 `#[test]` functions).
      `Colisionable` itself stays (still implemented by `Bullet` and `Player`).

  - `game/src/entities/mod.rs`
    - Remove `pub mod enemy;` (line 2). Keep `bullet`, `nemesis`, `player`.

  - `client/src/world/game.rs`
    - Remove `use game::entities::enemy::Enemy;` (line 13).
    - Remove the `pub enemies: Vec<Enemy>` field (line 34) and the
      `pub pending_enemy_deaths: HashSet<u8>` field (line 59).
    - In `World::new`: remove the `let enemies = game::entities::enemy::
      enemies_from_data(&levels[0][0].data, 0);` call (line 110) and the
      `enemies,` / `pending_enemy_deaths: HashSet::new(),` initializers.
    - In `World::update`: remove the `for enemy in &mut self.enemies { ... }`
      RNG loop (lines 196–200; this also removes the last use of
      `macroquad::rand::gen_range`) and the `self.enemies.retain(|e| e.alive);`
      cleanup (line 211).
    - In `World::draw`: remove the `for enemy in &self.enemies { enemy.draw(); }`
      loop (lines 291–293).
    - Remove the `enemies` field from `impl fmt::Debug for World` (line 82).
    - **Rename** (see "The rename" sub-list below).
    - **Also** delete/rewrite the stale comment `// Enemy death is now
      server-authoritative via EnemyDied packets.` (line 194) — it contains the
      `EnemyDied` grep token and must go for grep-clean. Clean up the adjacent
      stale comments (`// NOTE: Own-bullet vs enemy collision loop REMOVED`,
      `// Enemies still simulated client-side`, and `// Run transition check
      AFTER bullet-vs-enemy collision...`) since they reference removed behavior.

  - `client/src/world/lib.rs`
    - Rename `pub fn update_enemy_bullets_position` →
      `update_remote_bullets_position`, and its internal `world.enemy_bullets`
      reference (line 8).
    - In `check_player_death`: rename the `world.enemy_bullets` loop variable
      target (line 20) — PvP collision stays.
    - In `load_current_level`: remove `world.enemies.clear();` (line 61), remove
      `world.pending_enemy_deaths.clear();` (line 66), and remove the
      `EnemyStateRequest` send block (lines 78–89: `let (rx, ry)` + `req`
      construction + `world.network.send(req)` + the "Level load requested"
      `println!`). Keep the other clears (bullets, clock reset, sticky reset).
    - In `handle_death`: remove `world.pending_enemy_deaths.clear();` (line 49),
      keep the other clears.
    - Rename `world.enemy_bullets.clear()` → `world.remote_bullets.clear()`
      (in `load_current_level`, line 63 — the only such site; `handle_death`
      has no `enemy_bullets.clear()` of its own, it calls `load_current_level`).
      See the rename sub-list.

  - `client/src/network/world.rs`
    - In the `BulletSpawn` handler, rename `world.enemy_bullets.push(...)` →
      `world.remote_bullets.push(...)` (line 264–266) — PvP bullet routing
      stays.
    - Remove the `GamePacket::EnemyDied { .. }` handler arm entirely
      (lines 269–288, incl. its comment).
    - Remove the `GamePacket::EnemyStateResponse { .. }` handler arm entirely
      (lines 289–320, incl. its comment).
    - In the `JoinResponse` handler, remove `world.pending_enemy_deaths.clear();`
      (line 98).

  - `client/src/render.rs`
    - Remove `use game::entities::enemy::{Enemy, EnemyKind};` (line 12).
    - Remove `impl Drawable for Enemy` (lines 48–56, incl. the
      `EnemyKind -> Color` mapping).
    - Update the module doc comment (lines 1–8): drop the stale sentence "The
      `EnemyKind -> Color` mapping lives here too, keeping `Color` (a macroquad
      type) out of `game`." (it contains the `EnemyKind` grep token). `Drawable`
      stays (implemented for `Level`, `Player`, `Nemesis`, `Bullet`).

  - `client/src/network/logging.rs`
    - Remove the `GamePacket::EnemyDied`, `EnemyStateRequest`,
      `EnemyStateResponse` arms from `packet_type_name` and
      `packet_key_fields`.
    - In `packet_id`, remove **only** the `GamePacket::EnemyDied` arm (line 53) —
      `EnemyStateRequest`/`EnemyStateResponse` have no arm there and fall
      through `_ => None`.
    - Remove the `#[cfg(test)]` assertions in `packet_type_name_all_variants`,
      `packet_id_all_variants`, and `packet_key_fields_all_variants` that
      construct those three variants; leave the rest of each test intact.

  - `client/src/network/mod.rs`
    - In `handshake_tests`, swap the two `GamePacket::EnemyStateRequest {
      room_x, room_y }` sample packets (in
      `poll_join_response_discards_packets_before_join_response` line 58 and
      `poll_join_response_ignores_non_join_packets` line 104) for a surviving
      variant, e.g. `GamePacket::BulletSpawn { bullet_id: 0, x: 0.0, y: 0.0,
      dir: 1.0, owner_id: 1, seq: 0 }`. Keep the tests' intent ("non-join
      packets are discarded/ignored").

  - `e2e/src/harness.rs`
    - `let (shots, deaths) = self.server.update(tick_dt);` (line 235) →
      `let shots = self.server.update(tick_dt);`.
    - Remove Step 6b (the `EnemyDied` broadcast loop, lines 293–313).
    - In Step 7b (client local prediction): remove the `for enemy in &mut
      client.world.enemies { enemy.update(tick_dt, level, 0); }` loop
      (lines 352–354), remove `client.world.enemies.retain(|e| e.alive);`
      (line 365), and rename `client.world.enemy_bullets` →
      `client.world.remote_bullets` (lines 349 and 364).
    - Update the stale doc comment on `inject_server_packet` (line 498) that
      mentions `EnemyDied`.

  - `e2e/tests/room_load.rs` — **delete the file** (all four `#[ignore]` tests
    exist only to cover `EnemyStateRequest`/`EnemyStateResponse` sync).

  - `e2e/tests/enemy_death.rs` — **delete the file** (its tests construct
    `Enemy`/`EnemyKind`, mutate `world.enemies`, and inject
    `GamePacket::EnemyDied`).

  - `e2e/tests/edge_cases_2e.rs`
    - Delete the single PvE test `test_e2e_enemy_died_before_room_load`
      (lines 48–~78, constructs `GamePacket::EnemyDied`). Keep
      `test_e2e_bullet_spawn_during_room_transition` and
      `test_e2e_shoot_cooldown_edge` (PvP, no enemy types).

  - `e2e/tests/bullet_spawn.rs`
    - Rename `world.enemy_bullets` → `world.remote_bullets` (see rename
      sub-list; 4 code refs + 2 comment refs).

- **The rename** (`enemy_bullets` → `remote_bullets`,
  `update_enemy_bullets_position` → `update_remote_bullets_position`) —
  explicit site list:

  | File | Ref |
  | --- | --- |
  | `client/src/world/game.rs` | `use crate::world::lib::{... update_enemy_bullets_position ...}` import (line 5) → `update_remote_bullets_position` |
  | `client/src/world/game.rs` | struct field `pub enemy_bullets: Vec<Bullet>` (line 40) → `pub remote_bullets` |
  | `client/src/world/game.rs` | `enemy_bullets: Vec::new(),` (line 129) → `remote_bullets: Vec::new(),` |
  | `client/src/world/game.rs` | `update_enemy_bullets_position(self, dt);` call (line 163) → `update_remote_bullets_position(self, dt);` |
  | `client/src/world/game.rs` | `for bullet in &self.enemy_bullets` draw loop (line 286) → `&self.remote_bullets` |
  | `client/src/world/lib.rs` | `pub fn update_enemy_bullets_position` (line 4) → `update_remote_bullets_position` |
  | `client/src/world/lib.rs` | `world.enemy_bullets.retain_mut(...)` (line 8) → `world.remote_bullets` |
  | `client/src/world/lib.rs` | `for bullet in &mut world.enemy_bullets` (line 20) → `&mut world.remote_bullets` |
  | `client/src/world/lib.rs` | `world.enemy_bullets.clear()` in `load_current_level` (line 63) → `world.remote_bullets.clear()` |
  | `client/src/network/world.rs` | `world.enemy_bullets.push(...)` (line 264–266) → `world.remote_bullets.push(...)` |
  | `e2e/src/harness.rs` | `for bullet in &mut client.world.enemy_bullets` (line 349) → `remote_bullets` |
  | `e2e/src/harness.rs` | `client.world.enemy_bullets.retain(...)` (line 364) → `remote_bullets` |
  | `e2e/tests/bullet_spawn.rs` | 4 code refs (lines 116, 117, 200, 203) → `remote_bullets` |
  | `e2e/tests/bullet_spawn.rs` | 2 comment refs (line 84 doc `/// Bullet in world.enemy_bullets.`, line 113 `// ... in enemy_bullets (not bullets)`) → `remote_bullets` |

- **Test changes / deletions** (summary — each maps to a file above):
  - Delete `e2e/tests/room_load.rs` and `e2e/tests/enemy_death.rs`.
  - Delete `test_e2e_enemy_died_before_room_load` in `e2e/tests/edge_cases_2e.rs`.
  - Delete `game/src/entities/enemy.rs` including its `#[cfg(test)]` module.
  - In `server/src/world.rs`: delete the **5 enemy tests**
    (`serverworld_builds_enemies_all_rooms`, `raycast_hits_enemy`,
    `raycast_miss_above_below`, `raycast_first_hit_wins`,
    `update_returns_shots_and_deaths`) and the stale
    `// Phase 2e — Server Enemy State & Hit Validation` header comment. **Keep**
    `shoot_emits_bulletspawninfo` and `shoot_cooldown_enforced` (pure PvP
    `BulletSpawn`; they reference neither `world.enemies` nor `EnemyState`) and
    fix only their tuple destructuring (see the `server/src/world.rs` entry).
  - Delete the 3 protocol round-trip tests (`round_trip_enemydied`,
    `round_trip_enemystaterequest`, `round_trip_enemystateresponse`).
  - Update the shoot-latch server tests' destructuring
    (`input_edge_flags_latch_and_clear`, `input_edge_flag_not_lost_between_ticks`).
  - Swap the 2 `EnemyStateRequest` handshake-test samples in
    `client/src/network/mod.rs` to `BulletSpawn`.
  - Rename the `enemy_bullets` refs in `e2e/tests/bullet_spawn.rs`.

- **Acceptance / verification** (run in this order; all must pass):

  1. `cargo test --workspace` — must be **green (zero failures)**. The count
     will be **lower** than the 194 baseline because this box deletes PvE tests
     (`room_load.rs`, `enemy_death.rs`, the 5 enemy server tests in
     `server/src/world.rs`, the 3 protocol round-trips,
     `game/entities/enemy.rs`'s tests, `test_e2e_enemy_died_before_room_load`).
     Any test **failure** is a regression from this change, not a pre-existing
     condition.
  2. `cargo fmt --check --all` — clean.
  3. `cargo clippy --all-targets --all-features --workspace -- -D warnings` —
     clean (watch for now-unused imports: `Enemy` and
     `macroquad::rand::gen_range` in `client/src/world/game.rs`;
     `game::config::TILE_SIZE` and `game::types::EntityType` in
     `server/src/world.rs`, both used only by the deleted
     `build_enemies_for_room`). Do **not** expect `HashMap`, `Map`, or `HashSet`
     to be unused — `HashMap`/`Map` stay used in `server/src/world.rs`, and
     `HashSet` stays used by `spawned_bullet_ids` in `client/src/world/game.rs`.
  4. Grep-clean (case-sensitive), over `server/ client/ protocol/ game/src/entities/`:

     ```bash
     grep -RIn 'EnemyState\|EnemyDied\|EnemyStateRequest\|EnemyStateResponse\|enemies_from_data\|EnemyKind' server client protocol game/src/entities
     # → no output (EnemyDiedInfo is caught as a substring of EnemyDied)
     ```

     ```bash
     grep -RIn 'raycast_hit\|build_enemies_for_room\|get_enemy_state' server client protocol game/src/entities
     # → no output
     ```

     ```bash
     grep -RIn 'enemy_bullets\|update_enemy_bullets_position' server client protocol game/src/entities
     # → no output
     ```

     ```bash
     grep -RIn 'entities::enemy\|entities::enemy::' server client protocol game/src/entities
     # → no output (no game::entities::enemy import remains; enemy.rs deleted;
     #   pub mod enemy; removed from entities/mod.rs)
     ```

  5. Additional e2e greps (the spec's primary scope excludes `e2e/`, but these
     files are edited and must be clean too):

     ```bash
     grep -RIn 'enemy_bullets\|EnemyDied\|EnemyState\|EnemyStateRequest\|EnemyStateResponse\|enemies_from_data\|EnemyKind\|\.enemies\b' e2e
     # → no output
     ```

     ```bash
     grep -RIn 'remote_bullets' e2e/tests/bullet_spawn.rs e2e/src/harness.rs
     # → present (the renamed PvP refs)
     ```

  6. Confirm PvP suites still pass: `movement`, `multiplayer`,
     `reconciliation`, `rooms`, `bullet_spawn` (the latter updated only for the
     rename) — covered by `cargo test --workspace`.

- **Test**: no new `#[test]` functions (removal only). Verification is the
  existing suite passing at baseline plus the grep checks in "Acceptance"
  above.

- **Explain**
  - What: Removed the PvE enemy machinery end-to-end and renamed the PvP field.
    `protocol` lost `GamePacket::{EnemyDied, EnemyStateRequest,
    EnemyStateResponse}` (+3 round-trip tests); `server` lost `EnemyState`,
    `EnemyDiedInfo`, `ServerWorld.enemies`, `build_enemies_for_room`,
    `raycast_hit`, `get_enemy_state`, the `EnemyStateRequest` handler and the
    `EnemyDied` broadcast, and `ServerWorld::update` now returns
    `Vec<BulletSpawnInfo>` (5 enemy tests deleted, 2 shoot tests kept);
    `game` unregister the `enemy` module; `client` lost the `enemies` /
    `pending_enemy_deaths` fields, the client-side RNG simulation and drawing,
    the two enemy packet handlers and `impl Drawable for Enemy`; `e2e` lost its
    PvE tests. `enemy_bullets` → `remote_bullets` and
    `update_enemy_bullets_position` → `update_remote_bullets_position`
    everywhere. The three files slated for deletion were emptied and their
    modules unregistered so the build stays green until the orchestrator
    removes them.
  - Why: The game is PvP-only; the leftover PvE monster concept duplicated a
    second authoritative state machine, drove a `raycast_hit` path with no
    remaining gameplay purpose, and gave other players' bullets a misleading
    "enemy" name. Removing it shrinks the surface before spec 2 re-introduces
    authoritative PvP hit detection.
  - Rust concepts: enum-variant deletion forcing exhaustive matches to shrink
    (the compiler proves no stale arm remains); function signature change
    (tuple → `Vec`) surfacing every call site at compile time; ownership and
    borrowing (dropping `&mut self.enemies` field borrows and the
    `HashMap<(u8, u8), Vec<EnemyState>>` map); orphan-rule trait impl removal
    (`impl Drawable for Enemy`); module registration (`pub mod enemy;`) as the
    boundary that decides whether a file is compiled at all; `Result`/`Option`
    unaffected — no `unwrap` added.
  - Watch out: `update` no longer hits anything (accepted: shooting emits a
    `BulletSpawn` that hits nothing until spec 2); the three emptied files are
    still on disk and must be `rm`-ed by the orchestrator (this shell cannot
    delete); `e2e/tests/edge_cases_2e.rs` also needed its now-unused
    `use protocol::GamePacket;` removed for clippy; test count drops from 194
    to 178 passed (16 PvE tests intentionally deleted), 3 ignored.

## Open questions / ambiguities (for the plan reviewer)

1. **`game.rs` line 194 comment contains the `EnemyDied` grep token but is not
   in the spec's game.rs list.** The spec lists removing the RNG loop and
   `retain`, but the comment `// Enemy death is now server-authoritative via
   EnemyDied packets.` (and the adjacent stale `// NOTE: ...` / `// Enemies
   still simulated ...` comments) also reference the removed machinery. This
   plan adds their removal/rewrite; verify the reviewer agrees (the `EnemyDied`
   token must go for grep-clean).

2. **`bullet_spawn.rs` "four references" undercounts.** There are 4 *code*
   references (lines 116, 117, 200, 203) **plus** 2 *comment* references
   (line 84 doc comment `/// Bullet in world.enemy_bullets.` and line 113
   inline comment `// ... in enemy_bullets (not bullets)`). All 6 must be
   renamed for the `enemy_bullets` grep to be clean. The plan lists all 6.

3. **Grep scope vs. `game/src/`.** The spec scopes the `EnemyState`/`EnemyKind`
   grep to `game/src/entities/`, which is correct — `EntityType::EnemySpawn`,
   `EntityType::PotionSpawn`, `EntityType::BossSpawn`, and `TileType::EnemySpawn`
   / `TileType::BossSpawn` in `game/src/types.rs` do **not** match any grep
   token (`EnemyState` / `EnemyKind` etc.), so the inert data model survives the
   grep untouched. Confirmed.

4. **No `Enemy`/`enemies` references elsewhere.** Grep confirms the only
   remaining `world.enemies` / `game::entities::enemy` / `Enemy` references are
   exactly those enumerated in this plan's file list — nothing outside the
   spec's surface.
