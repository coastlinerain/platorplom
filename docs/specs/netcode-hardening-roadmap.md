# Netcode hardening roadmap

- Status: proposed
- Date: 2026-11-26
- Reference: Gabriel Gambetta, "Fast-Paced Multiplayer" (Parts I–IV)

Modular, limited specs. Each is its own branch + `docs/specs/<name>.md` +
`docs/plans/<name>.md`, independently shippable, ordered so each leaves the game in a
working state.

## 1. `refactor/remove-pve-enemies` — *current*

Strip the PvE `Enemy` machinery end-to-end and rename `enemy_bullets` → `remote_bullets`.

- `protocol`: remove `EnemyDied`, `EnemyStateRequest`, `EnemyStateResponse` (+ round-trip tests).
- `server`: remove `EnemyState`, `EnemyDiedInfo`, `enemies` map, `raycast_hit`, `get_enemy_state`, and the `EnemyState*`/`EnemyDied` handling in `main.rs`.
- `game`: delete `game/src/entities/enemy.rs` (`Enemy`, `EnemyState` enum, `EnemyKind`, `enemies_from_data`); keep `EntityType`/`TileType` + map data (inert data model).
- `client`: remove `World.enemies`, `pending_enemy_deaths`, the RNG enemy loop + draw, the `EnemyDied`/`EnemyStateResponse` handlers, `EnemyStateRequest` in `load_current_level`, and the `Enemy` draw in `render.rs`.
- `e2e`: remove the `EnemyDied` broadcast + `world.enemies` refs in `harness.rs`; gut `room_load.rs` + `enemy_death.rs`.

*Result: PvP movement-only; shooting becomes inert (no targets) until spec 2.*

## 2. `feat/authoritative-player-death`

Server-authoritative PvP combat: `raycast_player` + wall occlusion, `PlayerHitInfo`, rename
`Hit` → `PlayerDied`, server-originated death, client respawns only on server death.

## 3. `feat/authoritative-room-membership`

`RequestRoomChange` + door-aware `can_enter`/`apply_room_change`, drop
`PlayerInput.level_x/level_y`, client snap-back on rejection.

## 4. `fix/reconciliation-replay-dt`

Replay dt = `TICK_HZ` (not `CLIENT_HZ`); fold in `CLIENT_HZ` → `RENDER_HZ` as a separate commit.

## 5. `refactor/single-interpolation-path`

Delete the legacy wall-clock `interpolate`/`interpolate_at`; keep only `interpolate_at_tick`.

## Deferred (separate specs, not this effort)

- `feat/lag-compensation` — Part IV (shoot a moving player rendered in the past).
- `feat/world-snapshot-batching` — one `WorldSnapshot` per tick instead of O(n²) `PlayerPos`.
- `fix/transport-lifecycle` — transport `Drop` socket/thread leak (M2), `.expect` cleanup, RTT clock sync.

## Owner decisions (captured)

- PvE enemies removed for now; the game is **PvP-only** until a future PvE phase.
- Rename `enemy_bullets` → `remote_bullets` (folded into spec 1).
- `Hit` → `PlayerDied { id }` (spec 2).
- `CLIENT_HZ` → `RENDER_HZ` as a **separate commit** (spec 4).
- Drop `PlayerInput.level_x/level_y` from the protocol (spec 3).
- Specs must stay **modular and limited** — one narrow concern per spec/branch.
