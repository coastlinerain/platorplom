# Remove PvE enemies

- Status: draft
- Date: 2026-11-26
- Branch: `refactor/remove-pve-enemies`

## Summary

The game is **PvP-only**: the adversaries that matter are the other human
players (`game::entities::nemesis::Nemesis` / `World.other_players`) and the
bullets they fire (`World.enemy_bullets`). There is also a leftover **PvE
monster** concept — `game::entities::enemy::Enemy` (with
`EnemyKind::{Enemy,Potion}`), spawned from `EnemySpawn`/`PotionSpawn` map cells,
moved by client-side RNG (`macroquad::rand`), with server-side `EnemyState`
tracking and a `raycast_hit` enemy hit-test.

The owner has decided to **remove the PvE machinery for now**; PvE may return in
a future phase. This spec is a **pure removal plus one rename**: no new
behavior, no new types, no protocol additions.

The rename: `enemy_bullets` → `remote_bullets` (and
`update_enemy_bullets_position` → `update_remote_bullets_position`). These are
bullets fired **by other players** (PvP) and stay; only the misleading "enemy"
name goes away.

## Goals / non-goals

**Goals**

- Delete the PvE `Enemy` concept end-to-end across `protocol`, `server`,
  `game`, `client`, and `e2e`.
- Remove the three PvE protocol variants and their round-trip tests.
- Remove the server's enemy state map, enemy hit-test, and the `EnemyDied`
  broadcast path.
- Delete `game/src/entities/enemy.rs` and its `pub mod enemy;` registration.
- Remove all client-side enemy simulation, drawing, and packet handling.
- Gut/remove the e2e tests that exist only to cover PvE sync.
- Rename `enemy_bullets` → `remote_bullets` everywhere (including e2e).

**Non-goals**

- No change to PvP behavior: `Nemesis` / `World.other_players`,
  `remote_bullets` (formerly `enemy_bullets`), `check_player_death`'s collision
  loop against them, and the `BulletSpawn` flow all stay exactly as-is.
- No removal of the inert data model: `EntityType` / `TileType` enum variants
  (`EnemySpawn`, `PotionSpawn`, `BossSpawn`) and the map data stay. A future
  PvE phase re-wires them.
- No new hit-detection target. Shooting still emits a `BulletSpawn`; it just
  hits nothing (see "Known consequence" below).
- No protocol renumbering, no wire-format migration, no `Hit` → `PlayerDied`
  rename (that is a separate spec: `feat/authoritative-player-death`).

## Behavior

### Known consequence (accepted, temporary)

After removal the server has **no hit-detection target**. A client "shoot" still
latches on the server and emits a `BulletSpawn` (broadcast to all clients, spawn
point = the shooter's center), but the server no longer tests that ray against
anything, so nothing is hit and no `EnemyDied` is ever produced. This is
**accepted and temporary** — the next spec (`feat/authoritative-player-death`)
re-adds server-authoritative PvP hit detection. This is a *consequence*, not a
bug to fix in this spec.

### Per-file removal surface

#### `protocol/src/lib.rs`

Remove the `GamePacket` variants:

- `EnemyDied { enemy_id, room_x, room_y }`
- `EnemyStateRequest { room_x, room_y }`
- `EnemyStateResponse { room_x, room_y, enemies }`

No other variant changes. `BulletSpawn`, `Hit`, `PlayerPos`, `PlayerInput`,
`JoinRequest`/`JoinResponse`, `Leave` stay unchanged.

#### `protocol/tests/round_trip.rs`

Delete the three round-trip tests `round_trip_enemydied`,
`round_trip_enemystaterequest`, `round_trip_enemystateresponse`. Leave the
`BulletSpawn` round-trip test and everything else intact.

#### `server/src/world.rs`

Remove:

- `pub struct EnemyState` (the **server-side struct** — see the name-collision
  gotcha below).
- `pub struct EnemyDiedInfo`.
- the `enemies: HashMap<(u8, u8), Vec<EnemyState>>` field on `ServerWorld`, and
  its construction in `ServerWorld::new` (the `for room_y { for room_x { ... } }`
  pre-build loop).
- `fn build_enemies_for_room`.
- `fn raycast_hit`.
- `fn get_enemy_state`.
- the enemy-death path inside `ServerWorld::update`: the second pass no longer
  calls `raycast_hit` and no longer builds/returns the `EnemyDiedInfo` vec.

Behavior of `update` after removal:

- Return type changes from `(Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)` to
  `Vec<BulletSpawnInfo>`.
- The `ShootRequest` struct keeps `player_id`, `spawn_x`, `spawn_y`, `dir`,
  `player_seq`; its `room_coords` field (only used by `raycast_hit`) is dropped,
  along with the `let room_coords = self.coords.get(&id)...` binding that fed it.
- The second pass emits `BulletSpawnInfo { bullet_id, x: req.spawn_x,
  y: req.spawn_y, dir: req.dir, owner_id: req.player_id, seq: req.player_seq }`
  with no hit test. Bullet ID assignment and cooldown logic are unchanged.

`BulletSpawnInfo` stays.

**Server unit tests** (bottom of `server/src/world.rs`, the `#[cfg(test)]` block
with a second `// Phase 2e — Server Enemy State & Hit Validation` block outside
the `mod tests`):

- Delete `serverworld_builds_enemies_all_rooms`, `raycast_hits_enemy`,
  `raycast_miss_above_below`, `raycast_first_hit_wins`,
  `update_returns_shots_and_deaths` (all reference `EnemyState`, `EnemyDiedInfo`,
  or `world.enemies`).
- Update the shoot-latch / cooldown tests that still exercise PvP bullet spawn
  but currently destructure the two-tuple return: `input_edge_flags_latch_and_clear`,
  `input_edge_flag_not_lost_between_ticks`, `shoot_emits_bulletspawninfo`,
  `shoot_cooldown_enforced` change `let (shots, _deaths) = world.update(...)` to
  `let shots = world.update(...)`. Their assertions (shot count, owner, dir,
  bullet_id increment, cooldown) are unchanged.

#### `server/src/lib.rs`

Change the re-export from
`pub use world::{BulletSpawnInfo, EnemyDiedInfo, EnemyState};` to
`pub use world::{BulletSpawnInfo, ServerWorld};` (i.e. drop `EnemyDiedInfo` /
`EnemyState`). Keep `ServerWorld` re-exported.

#### `server/src/main.rs`

- In the packet-logging helpers, remove the `GamePacket::EnemyDied`,
  `GamePacket::EnemyStateRequest`, `GamePacket::EnemyStateResponse` arms from
  `packet_type_name`, `packet_id`, and `packet_key_fields` (the `_ => None` /
  existing catch-alls already cover the removed variants after the match arms
  are deleted).
- Remove the `GamePacket::EnemyStateRequest { room_x, room_y }` handling arm in
  the event loop (and its `EnemyStateResponse` reply).
- Change `let (shots, deaths) = world.update(tick_dt);` to
  `let shots = world.update(tick_dt);`.
- Delete the "Broadcast EnemyDied to clients in the same room" block (the
  `for death in &deaths { ... }` loop and its `GamePacket::EnemyDied` build).

#### `game/src/entities/enemy.rs`

Delete the whole file: `Enemy` struct, the `EnemyState` **enum** (client/game
side), `EnemyKind`, `enemies_from_data`, the `impl Colisionable for Enemy`, and
the file's `#[cfg(test)]` module. (`Colisionable` itself stays — it is still
implemented by `Bullet` and `Player` and used by `Level::map_colision`.)

#### `game/src/entities/mod.rs`

Remove the line `pub mod enemy;`. Keep `bullet`, `nemesis`, `player`.

#### `client/src/world/game.rs`

- Remove `use game::entities::enemy::Enemy;`.
- Remove the `pub enemies: Vec<Enemy>` field and the
  `pub pending_enemy_deaths: HashSet<u8>` field.
- In `World::new`: remove the `let enemies = game::entities::enemy::enemies_from_data(...)`
  call and the `enemies,` / `pending_enemy_deaths: HashSet::new(),` initializers.
- In `World::update`: remove the `for enemy in &mut self.enemies { ... }` RNG
  loop (this also removes the last use of `macroquad::rand::gen_range` — see
  gotchas) and the `self.enemies.retain(|e| e.alive);` cleanup.
- In `World::draw`: remove the `for enemy in &self.enemies { enemy.draw(); }`
  loop.
- Remove the `enemies` field from the `impl fmt::Debug for World` body.
- Rename the field `enemy_bullets` → `remote_bullets` (struct field,
  `Vec::new()` initializer, the `for bullet in &self.enemy_bullets` draw loop).
  The `use crate::world::lib::{... update_enemy_bullets_position ...}` import
  becomes `update_remote_bullets_position`, and the call site in `update` is
  renamed accordingly.

#### `client/src/world/lib.rs`

- Rename `pub fn update_enemy_bullets_position` → `update_remote_bullets_position`,
  and the `world.enemy_bullets` references inside it.
- In `check_player_death`: rename the `world.enemy_bullets` loop variable target
  (PvP collision stays).
- In `load_current_level`: remove `world.enemies.clear();` and the
  `EnemyStateRequest` send block (the `req` construction + `world.network.send(req)`
  + the "Level load requested" println). Keep the rest of `load_current_level`
  (bullet clears, clock reset, sticky reset).
- In `handle_death` (respawn path): remove `world.pending_enemy_deaths.clear();`
  (keep the other clears). Rename `world.enemy_bullets.clear()` →
  `world.remote_bullets.clear()`.
- In `load_current_level`: rename `world.enemy_bullets.clear()` →
  `world.remote_bullets.clear()`.

#### `client/src/network/world.rs`

- In the `BulletSpawn` handler, rename `world.enemy_bullets.push(...)` →
  `world.remote_bullets.push(...)` (PvP bullet routing stays).
- Remove the `GamePacket::EnemyDied { .. }` handler arm entirely.
- Remove the `GamePacket::EnemyStateResponse { .. }` handler arm entirely.
- In the `JoinResponse` handler, remove `world.pending_enemy_deaths.clear();`.

#### `client/src/render.rs`

- Remove `use game::entities::enemy::{Enemy, EnemyKind};`.
- Remove `impl Drawable for Enemy` (including the `EnemyKind -> Color` mapping).
- Update the module doc comment that mentions the `EnemyKind -> Color` mapping
  (drop the stale sentence). `Drawable` stays (implemented for `Level`, `Player`,
  `Nemesis`, `Bullet`).

#### `client/src/network/logging.rs` *(missed by the initial scope — see below)*

- Remove the `GamePacket::EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse`
  arms from `packet_type_name`, `packet_id`, and `packet_key_fields`.
- Remove the `#[cfg(test)]` assertions in `packet_type_name_all_variants`,
  `packet_id_all_variants`, and `packet_key_fields_all_variants` that construct
  those three variants (leave the rest of each test intact).

#### `client/src/network/mod.rs` *(missed by the initial scope)*

The `handshake_tests` module uses `GamePacket::EnemyStateRequest { room_x, room_y }`
as a sample "discard before `JoinResponse`" packet (two sites:
`poll_join_response_discards_packets_before_join_response` and
`poll_join_response_ignores_non_join_packets`). Replace those samples with a
surviving variant (e.g. `GamePacket::BulletSpawn { bullet_id: 0, x: 0.0,
y: 0.0, dir: 1.0, owner_id: 1, seq: 0 }`) so the handshake tests keep testing
"non-join packets are discarded/ignored" without referencing a deleted variant.

#### `e2e/src/harness.rs`

- Change `let (shots, deaths) = self.server.update(tick_dt);` to
  `let shots = self.server.update(tick_dt);`.
- Remove Step 6b (the `EnemyDied` broadcast loop).
- In Step 7b (client local prediction): remove the
  `for enemy in &mut client.world.enemies { enemy.update(tick_dt, level, 0); }`
  loop, and rename `client.world.enemy_bullets` → `client.world.remote_bullets`
  (both the update loop and the `retain` cleanup). Remove
  `client.world.enemies.retain(|e| e.alive);`.
- Update the stale doc comment on `inject_server_packet` that mentions
  `EnemyDied`.

#### `e2e/tests/room_load.rs`

Delete the file (all four `#[ignore]` tests exist only to cover
`EnemyStateRequest`/`EnemyStateResponse` sync).

#### `e2e/tests/enemy_death.rs`

Delete the file (its tests construct `Enemy`/`EnemyKind`, mutate
`world.enemies`, and inject `GamePacket::EnemyDied`).

#### `e2e/tests/edge_cases_2e.rs` *(missed by the initial scope)*

Delete the single PvE test `test_e2e_enemy_died_before_room_load` (constructs
`GamePacket::EnemyDied`). Keep the other tests in the file (bullet spawn during
room transition, shoot cooldown edge) — they are PvP and do not reference enemy
types.

#### `e2e/tests/bullet_spawn.rs` *(missed by the initial scope)*

Rename `world.enemy_bullets` → `world.remote_bullets` (four references in
`test_*_bullet_spawn`-style tests). These are PvP bullet tests and stay.

### Edge cases

- A `BulletSpawn` from another player still lands in `remote_bullets` and is
  still drawn and still collides with the local player via `check_player_death`
  — the only observable change for PvP is the field/helper name.
- Shooting now always emits a `BulletSpawn` at the shooter's center with no hit
  test; this must not panic and must not change the cooldown cadence.
- The map data in `game/src/maps.rs` currently contains **zero**
  `EnemySpawn`/`PotionSpawn`/`BossSpawn` cells (all four `DATA_*` grids are all
  `0` → `EntityType::Empty`). So no runtime behavior ever exercised enemy spawns
  today; the server unit test `serverworld_builds_enemies_all_rooms` (asserting
  all four rooms are empty) is correct, and nothing user-visible is lost.
- `EntityType::EnemySpawn`, `EntityType::PotionSpawn`, `EntityType::BossSpawn`,
  `TileType::EnemySpawn`, `TileType::BossSpawn` remain defined and the
  `From<u8>` impls keep mapping them — they are inert data, not removed.

## Data model

| Item | Today | After |
| --- | --- | --- |
| `protocol::GamePacket::{EnemyDied, EnemyStateRequest, EnemyStateResponse}` | present | removed |
| `server::EnemyState` (struct) | `server/src/world.rs` | removed |
| `server::EnemyDiedInfo` | `server/src/world.rs` | removed |
| `ServerWorld.enemies` | `HashMap<(u8,u8), Vec<EnemyState>>` | removed |
| `ServerWorld::{build_enemies_for_room, raycast_hit, get_enemy_state}` | present | removed |
| `ServerWorld::update` return | `(Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)` | `Vec<BulletSpawnInfo>` |
| `game::entities::enemy::{Enemy, EnemyState (enum), EnemyKind, enemies_from_data}` | present | removed (file deleted) |
| `game::types::{TileType, EntityType}` variants + `From<u8>` | present | unchanged (inert) |
| `World.enemies: Vec<Enemy>` | present | removed |
| `World.pending_enemy_deaths: HashSet<u8>` | present | removed |
| `World.enemy_bullets: Vec<Bullet>` | present | renamed `remote_bullets` |
| `update_enemy_bullets_position` | `client/src/world/lib.rs` | renamed `update_remote_bullets_position` |
| `World.other_players: HashMap<u8, Nemesis>` | present | unchanged (PvP) |
| `Drawable for Enemy` + `EnemyKind -> Color` | `client/src/render.rs` | removed |

## Network

Removed packets (no additions, no renumbering, no wire-migration concern since
postcard is self-describing over the enum):

- `EnemyDied { enemy_id: u8, room_x: u8, room_y: u8 }`
- `EnemyStateRequest { room_x: u8, room_y: u8 }`
- `EnemyStateResponse { room_x: u8, room_y: u8, enemies: Vec<(u8,bool,f32,f32)> }`

`BulletSpawn` remains the only "shoot" output: server → all clients,
reliable-ordered, one per accepted shot (cooldown-gated). `Hit` (client → server
self-report of death) is unchanged and is **not** part of this spec.

## Acceptance criteria

- `cargo test --workspace` passes.
- `cargo fmt --check --all` clean; `cargo clippy --all-targets --all-features
  --workspace -- -D warnings` clean.
- `grep`-clean (case-sensitive) across `server/`, `client/`, `protocol/`,
  `game/src/entities/` for each of: `EnemyState`, `EnemyDied`, `EnemyStateRequest`,
  `EnemyStateResponse`, `enemies_from_data`, `EnemyKind`. (This also catches
  `EnemyDiedInfo` as a substring.)
- `grep`-clean across `server/`, `client/`, `protocol/`, `game/src/entities/`
  for `raycast_hit`, `build_enemies_for_room`, `get_enemy_state` (the server
  helpers).
- No `game::entities::enemy` (or `entities::enemy`) import remains anywhere; the
  file `game/src/entities/enemy.rs` is gone and `pub mod enemy;` is gone from
  `game/src/entities/mod.rs`.
- `enemy_bullets` and `update_enemy_bullets_position` appear nowhere; the
  identifiers `remote_bullets` / `update_remote_bullets_position` are used
  instead (including `e2e/tests/bullet_spawn.rs` and `e2e/src/harness.rs`).
- `World` no longer has `enemies` or `pending_enemy_deaths` fields; `Enemy`
  is no longer drawn or simulated client-side.
- `e2e/tests/room_load.rs` and `e2e/tests/enemy_death.rs` are deleted;
  `test_e2e_enemy_died_before_room_load` is deleted from
  `e2e/tests/edge_cases_2e.rs`.
- PvP movement/prediction/interpolation is unchanged: e2e suites `movement`,
  `multiplayer`, `reconciliation`, `rooms`, and `bullet_spawn` still pass (with
  `bullet_spawn` updated only for the rename).

## Follow-up

- `feat/authoritative-player-death` (roadmap spec 2) re-adds server-authoritative
  PvP hit detection; until then shooting is inert by design.
- The `macroquad::rand::gen_range` call disappears with the enemy RNG loop; if
  `macroquad::rand` is otherwise unused, that is expected and needs no
  dependency change (it is part of the macroquad prelude, not a separate crate).

## Gotchas (for the implementer / planner)

- **`EnemyState` name collision**: it exists as a *struct* in
  `server/src/world.rs` AND as an *enum* in `game/src/entities/enemy.rs`. Both
  are removed; do not leave one behind.
- **Server unit tests live in two blocks**: the file has a `mod tests` block
  (movement/input) and a *second* top-level `#[cfg(test)]` block below it
  ("Phase 2e — Server Enemy State & Hit Validation"). The second block is the
  enemy block to delete; the first block's shoot-latch tests only need their
  `update(...)` tuple destructuring fixed.
- **Map data is all-empty**: `game/src/maps.rs` ships zero spawn cells (verified),
  so `serverworld_builds_enemies_all_rooms` is the only test that touches the
  now-removed `enemies` map, and no runtime behavior depends on spawn cells.
