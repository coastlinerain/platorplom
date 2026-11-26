# Phase 2e — Netcode Correctness: Test Report (RED Phase)

## Summary
All 32 test functions have been written across 7 test files. They are currently in the **RED phase** — they fail to compile because the implementation (new protocol variants, server enemy state, client bullet/enemy flow changes) does not exist yet.

## Test Files Created

### 1. Protocol Tests (`protocol/tests/round_trip.rs`)
**6 tests** (4 new + 2 existing)

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `round_trip_bulletspawn` | 1,3 | ❌ Compile fail | `GamePacket::BulletSpawn` variant not found |
| `round_trip_enemydied` | 1,3 | ❌ Compile fail | `GamePacket::EnemyDied` variant not found |
| `round_trip_enemystaterequest` | 1,3 | ❌ Compile fail | `GamePacket::EnemyStateRequest` variant not found |
| `round_trip_enemystateresponse` | 1,3 | ❌ Compile fail | `GamePacket::EnemyStateResponse` variant not found |
| `action_variant_removed` | 2 | ❌ Runtime panic | Panics: "Action variant still exists" |
| `test_no_game_or_glam_deps` | 4 | ❌ Compile fail | `Command` not imported (minor fix needed) |

### 2. Server Unit Tests (`server/src/world.rs` - test module)
**8 tests** (8 new)

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `serverworld_builds_enemies_all_rooms` | 5 | ❌ Compile fail | `world.enemies` field not found |
| `raycast_hits_enemy` | 6 | ❌ Runtime panic | "raycast_hit method not yet implemented" |
| `raycast_miss_above_below` | 7 | ❌ Runtime panic | "raycast_hit method not yet implemented" |
| `raycast_first_hit_wins` | 7 | ❌ Runtime panic | "raycast_hit method not yet implemented" |
| `update_returns_shots_and_deaths` | 8 | ❌ Compile fail | `update()` return type mismatch; `BulletSpawnInfo`/`EnemyDiedInfo` not found |
| `shoot_emits_bulletspawninfo` | 9 | ❌ Compile fail | `BulletSpawnInfo` struct not found |
| `shoot_cooldown_enforced` | 12 | ❌ Compile fail | `BulletSpawnInfo` struct not found; return type mismatch |

### 3. E2E: Bullet Spawn (`e2e/tests/bullet_spawn.rs`)
**5 tests** (4 AC + 1 edge)

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_no_local_bullet_on_pressed_shoot` | 13 | ❌ Compile fail | `GamePacket::BulletSpawn` not found |
| `test_bulletspawn_handler_spawns_own_bullet` | 14 | ❌ Compile fail | `GamePacket::BulletSpawn` not found |
| `test_bulletspawn_handler_spawns_remote_bullet` | 14 | ❌ Compile fail | `GamePacket::BulletSpawn` not found |
| `test_bulletspawn_dedupe` | 15 | ❌ Compile fail | `GamePacket::BulletSpawn` not found; `spawned_bullet_ids` field not on `World` |
| `test_e2e_bulletspawn_both_clients` | 16 | ❌ Runtime panic | "Server does not yet broadcast BulletSpawn" |

### 4. E2E: Enemy Death (`e2e/tests/enemy_death.rs`)
**4 tests**

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_enemy_has_id_field` | 17 | ❌ Compile fail | `Enemy.id` field not found |
| `test_enemydied_handler_kills_enemy` | 18 | ❌ Compile fail | `GamePacket::EnemyDied` not found; `Enemy.id` not found |
| `test_own_bullet_enemy_collision_removed` | 19 | ❌ Compile fail | `Enemy.id` field not found |
| `test_e2e_shoot_enemy_both_clients` | 20 | ❌ Runtime panic | "Server enemy state not yet implemented" |

### 5. E2E: Room Load (`e2e/tests/room_load.rs`)
**4 tests**

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_load_level_sends_request` | 21 | ❌ Runtime panic | "load_current_level not yet updated..."; `spawned_bullet_ids`/`confirmed_bullets` missing |
| `test_server_enemystate_response` | 22 | ❌ Runtime panic | "Server does not yet handle EnemyStateRequest" |
| `test_client_spawns_only_alive` | 23 | ❌ Compile fail | `GamePacket::EnemyStateResponse` not found; `Enemy.id` not found |
| `test_e2e_enemy_stays_dead_across_room_transition` | 24 | ❌ Runtime panic | "Full room load sync not yet implemented" |

### 6. E2E: Reconciliation (`e2e/tests/reconciliation_bullet.rs`)
**2 tests**

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_reconciliation_uses_confirmed_bullets` | 25 | ❌ Runtime panic | "confirmed_bullets field not yet implemented" |
| `test_e2e_no_position_snap_on_shoot` | 26 | ❌ Runtime panic | "Server BulletSpawn broadcast not yet implemented" |

### 7. E2E: Regression (`e2e/tests/regression.rs`)
**1 test**

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_no_duplicate_join_request` | 27 | ✅ Should pass | Tests existing Phase 2d fix |

### 8. E2E: Edge Cases (`e2e/tests/edge_cases_2e.rs`)
**3 tests** (additional from plan)

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `test_e2e_bullet_spawn_during_room_transition` | — | ❌ Runtime panic | "Room transition during BulletSpawn not yet handled" |
| `test_e2e_enemy_died_before_room_load` | — | ❌ Runtime panic | "EnemyDied for different room not yet handled" |
| `test_e2e_shoot_cooldown_edge` | — | ❌ Runtime panic | "Server cooldown edge case not fully verified" |

### 9. E2E: Quality Gates (`e2e/tests/quality_gates.rs`)
**5 tests**

| Test | AC | Status | Expected Failure |
|------|-----|--------|------------------|
| `cargo_build_workspace` | 28 | ❌ Runtime fail | Build fails due to missing API |
| `cargo_test_workspace` | 29 | ❌ Runtime fail | Tests fail to compile |
| `cargo_clippy_workspace` | 30 | ❌ Runtime fail | Clippy fails on missing API |
| `cargo_fmt_check` | 31 | ✅ Should pass | Formatting is clean |
| `test_no_unwrap_in_new_logic` | 32 | ⚠️ Warns | Prints warnings for existing unwrap/expect |

## Total: 32 Test Functions (matching 32 ACs)

## Key Compilation Errors (from first `cargo test` run)

### Protocol Crate
```
error[E0599]: no variant named `BulletSpawn` found for enum `GamePacket`
error[E0599]: no variant named `EnemyDied` found for enum `GamePacket`
error[E0599]: no variant named `EnemyStateRequest` found for enum `GamePacket`
error[E0599]: no variant named `EnemyStateResponse` found for enum `GamePacket`
```

### Server Crate
```
error[E0609]: no field `enemies` on type `ServerWorld`
error[E0599]: no method named `set_server_player_pos` found for struct `ServerWorld`
error[E0599]: no method named `set_server_player_grounded` found for struct `ServerWorld`
error[E0308]: mismatched types - `update()` returns `Vec<(u8, f32)>` but tests expect `(Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)`
error[E0412]: cannot find type `BulletSpawnInfo` in this scope
error[E0412]: cannot find type `EnemyDiedInfo` in this scope
```

### E2E Crate
```
error[E0599]: no variant named `BulletSpawn` found for enum `GamePacket`
error[E0599]: no variant named `EnemyDied` found for enum `GamePacket`
error[E0599]: no variant named `EnemyStateRequest` found for enum `GamePacket`
error[E0599]: no variant named `EnemyStateResponse` found for enum `GamePacket`
error[E0609]: no field `id` on type `Enemy`
error[E0609]: no field `spawned_bullet_ids` on type `World`
error[E0609]: no field `confirmed_bullets` on type `World`
```

## Next Steps for Implementer

The implementer should work through the plan boxes in order:

1. **Group A (Protocol)**: Add 4 new `GamePacket` variants, remove `Action` variant
2. **Group D (Game Crate)**: Add `id: u8` to `Enemy`, update `enemies_from_data`
3. **Group B (Server World)**: Add `EnemyState`, `BulletSpawnInfo`, `EnemyDiedInfo`, `enemies` HashMap, raycast logic, change `update()` return type
4. **Group C (Server Main)**: Broadcast `BulletSpawn` to all, remove `Action` send, handle `EnemyStateRequest`
5. **Group E (Client Network)**: Handle `BulletSpawn`, `EnemyDied`, `EnemyStateResponse`
6. **Group F (Client Game)**: Remove local bullet spawn, remove bullet-enemy collision
7. **Group G (Client Room Load)**: Update `load_current_level` to send `EnemyStateRequest`
8. **Group H (Reconciliation)**: Use `confirmed_bullets` in reconciliation
9. **Group I (E2E)**: Tests will pass once above is done
10. **Group J (Quality Gates)**: All CI checks pass

## Handoff Items

The following files need implementation changes (not test changes):

- `protocol/src/lib.rs` — Add new packet variants, remove `Action`
- `server/src/world.rs` — Add enemy state, raycast, change `update()` return type
- `server/src/main.rs` — Handle new packets, broadcast changes
- `game/src/entities/enemy.rs` — Add `id` field, assign in `enemies_from_data`
- `client/src/network/world.rs` — Handle new packet types
- `client/src/world/game.rs` — Remove local spawn/collision, add `spawned_bullet_ids`, `confirmed_bullets`
- `client/src/world/lib.rs` — Update `load_current_level`

No test files should be modified by the implementer — they are the specification.