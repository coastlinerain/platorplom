# Authoritative player death — Implementation Plan

- Status: implemented (2026-11-26; 198 tests pass under `cargo test --workspace`,
  3 ignored; includes all 21 named tests)
- Spec: `docs/specs/authoritative-player-death.md`
- Date: 2026-11-26
- Branch: `feat/authoritative-player-death`
- Roadmap: spec 2 of `docs/specs/netcode-hardening-roadmap.md`
- Pipeline: standard (spec → plan → plan review → tester writes RED → implementer green → code review → qa)

## Overview

Spec 1 left "shoot" inert and death a client self-report (`GamePacket::Hit`). This
plan makes the **server the sole authority** on player death:

1. rename `Hit { entity_id }` → `PlayerDied { id }` (same enum slot, same payload);
2. add `Level::solid_between` (single-row wall query — `solid_in_rect` is **not**
   usable for a 1-px ray at `y = 119`);
3. rewrite the server's second pass: sort `shoot_requests` by `player_id`, cast
   `raycast_player`, spawn the bullet at the impact x on a hit, dedupe victims,
   respawn after the loop, return `(Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>)`;
4. broadcast one `PlayerDied` per hit to **all** clients from `server/src/main.rs`;
   ignore any inbound `PlayerDied`;
5. delete the client's self-report and client-side death collision; handle
   `PlayerDied` for self (`respawn_local`) and for remotes (drop the stale
   `Nemesis`);
6. ripple the `update` tuple through the e2e harness and add a PvP death e2e.

Six boxes. Boxes 1–3 contain all the genuinely new logic and are the ones the
tester can RED first; Boxes 4–6 are wiring.

**Compilation state is expected to be RED from the end of Box 1 through the end of
Box 5.** The protocol rename (Box 1) immediately breaks `server/src/main.rs`,
`client/src/network/{transport,logging,world}.rs` and `client/src/world/lib.rs`
(repaired in Boxes 4–5); on top of that, `e2e/src/harness.rs`
(`let shots = self.server.update(...)`) breaks at Box 3 and is only repaired in
Box 6. Each box therefore has a *narrow* verification command (per-crate /
`--lib`) that can pass on its own; the whole-workspace gate is Box 6's
acceptance.

## Non-goals (hard constraints — do NOT do these)

- **No room authority** (spec 3): `coords` stays client-claimed via
  `apply_input`; `PlayerInput.level_x/level_y` stay; no `RequestRoomChange`, no
  door checks, no snap-back.
- **No lag compensation**: no position rewind; hits use this tick's
  post-movement positions.
- **No server bullet simulation**: `BulletSpawn` stays one packet per accepted
  shot; hits are instantaneous; bullets are client-rendered.
- **No i-frames, kill feed, score, death animation, or respawn timer**. Death →
  immediate respawn.
- **No room filtering of the death broadcast** (spec 3's job).
- **No `alive`/health flag on `Nemesis`** — remote death is entry removal (the
  `Leave` path).
- No change to `Player`, `Nemesis`, `Bullet`, map data (`TileType`/`EntityType`),
  movement/prediction/interpolation.
- No protocol versioning / wire migration.

## Public API (targets for the tester)

The tester compiles against these **before** the implementation exists. Exact
shapes:

```rust
// protocol/src/lib.rs — replace the `Hit` variant IN PLACE (between Leave and
// PlayerInput). Same payload type, one field, new name.
GamePacket::PlayerDied {
    id: u8,
}

// server/src/world.rs — new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerHitInfo {
    pub shooter_id: u8,
    pub victim_id: u8,
}

// server/src/world.rs — changed signature
pub fn update(&mut self, dt: f32)
    -> (Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>);

// server/src/world.rs — new PRIVATE methods (unit-tested from inside the file)
fn raycast_player(&self, shooter_id: u8, spawn_x: f32, spawn_y: f32, dir: f32)
    -> Option<(u8, f32)>; // (victim_id, hit_x)
fn respawn(&mut self, id: u8);

// game/src/level.rs — new public helper
/// True if any `TileType::Wall` tile blocks the horizontal segment from
/// `x0` to `x1` (order-insensitive) at height `y`.
pub fn solid_between(&self, x0: f32, x1: f32, y: f32) -> bool;

// client/src/world/lib.rs — renamed, network send removed
pub fn respawn_local(world: &mut World);

// server/src/lib.rs — re-export (single line replaces the current two)
pub use world::{BulletSpawnInfo, PlayerHitInfo, ServerWorld};
```

Semantics the tester may rely on (from the spec):

- `raycast_player` only uses `dir`'s sign (`dir >= 0.0` right, `< 0.0` left);
  same-room filter via raw `coords` (fallback `(0,0)`); self excluded; hitbox
  from the **victim's** `w`/`h`; inclusive vertical overlap; `distance =
  d_near.max(0.0)`; wall occlusion via `solid_between(spawn_x, hit_x, spawn_y)`;
  smallest distance wins; ascending victim-id iteration with strict `<` so a tie
  goes to the lower id.
- `update` spawns the bullet at `hit_x` on a hit and at `spawn_x` on a miss;
  assigns bullet ids in ascending shooter-id order; dedupes to one hit per
  victim per tick; respawns victims **after** the request loop; increments
  `current_tick` as today.
- `respawn(id)`: `*players.get_mut(&id) = Player::new(id)`,
  `coords.insert(id, (0,0))`, `pending.insert(id, PlayerInput::default())`;
  `shoot_cooldown` and `max_seq` untouched.
- `respawn_local(world)`: print, `current_coords = (0,0)`,
  `let id = world.player.id; world.player = Player::new(id);`, snap
  `prev_player_pos/dir`, clear bullet-id sets + clock sync, call
  `load_current_level`; **no network send**.

Unchanged public surface: `BulletSpawnInfo` (shape), `ServerWorld::{new, spawn,
remove, apply_input, current_tick, elapsed_ms, wall_time_ms, players, players_mut,
coords, max_seq}`, `PacketIO`, `World` fields, `Nemesis`, `Bullet`.

Deleted: `GamePacket::Hit`, `client::world::lib::check_player_death`,
`client::world::lib::handle_death`, `round_trip_hit`.

## Test cases (tester writes these first)

21 named tests (1 renamed protocol test + 20 new). Write them all in one RED
batch, in this order (each group below is independently runnable once its crate
compiles).

### Protocol — `protocol/tests/round_trip.rs` (1)

| # | Test | Exact assertion |
| --- | --- | --- |
| P1 | `round_trip_player_died` (**renames** `round_trip_hit`) | `round_trip(GamePacket::PlayerDied { id: 0 })` and `round_trip(GamePacket::PlayerDied { id: 255 })`; the helper asserts `assert_eq!(decoded, pkt)`. `round_trip_hit` must not exist. |

### Game — `game/src/level.rs` `mod tests` (5)

Fixtures: reuse `wall_ring_level()` (4 rows × 3 cols, wall border, interior col 1
empty) and `single_wall_cell_level()` (3 × 3, only `grid[0][0]` is a wall). Add
one fixture `wall_below_row_level()`: 4 rows × 4 cols, every tile `Empty` except
`grid[3][1] = TileType::Wall` (`data` all `Empty`, same dimensions).

| # | Test | Exact assertion |
| --- | --- | --- |
| G1 | `solid_between_wall_between_true` | `wall_ring_level`: `assert!(level.solid_between(50.0, 110.0, 60.0))` (row 1, cols 1..2, col 2 is `Wall`); order-insensitive: `assert!(level.solid_between(110.0, 50.0, 60.0))`. |
| G2 | `solid_between_stops_before_wall_false` | `wall_ring_level`: `assert!(!level.solid_between(45.0, 75.0, 60.0))` (only col 1, which is `Empty`). |
| G3 | `solid_between_ignores_adjacent_row_wall` | `wall_below_row_level`: `assert!(!level.solid_between(0.0, 80.0, 119.0))` (`gy = 2`, row 2 empty) **and** the regression proof `assert!(level.solid_in_rect(Rect::new(0.0, 119.0, 80.0, 1.0)))` (the 1-px rect spans rows 2 and 3 and sees `grid[3][1]`). |
| G4 | `solid_between_out_of_grid_row_not_solid` | `wall_ring_level`: `assert!(!level.solid_between(50.0, 110.0, -10.0))` (`gy < 0`) and `assert!(!level.solid_between(50.0, 110.0, 1000.0))` (`gy >= height`). |
| G5 | `solid_between_out_of_grid_columns_not_solid` | `single_wall_cell_level`: `assert!(!level.solid_between(100.0, 400.0, 100.0))` (cols ≥ 3 ignored) and `assert!(!level.solid_between(-100.0, 10.0, 100.0))` (negative cols ignored; col 0 at row 2 is `Empty`). |

### Server — `server/src/world.rs`, new `#[cfg(test)] mod raycast_and_hit_tests` (10)

Helpers inside the module (private fields are reachable from a child module):

```rust
fn place(world: &mut ServerWorld, id: u8, x: f32, y: f32, dir: f32)
// spawn(id) first; then set pos, w = 30.0, h = 38.0, vel = Vec2::ZERO,
// dir, grounded = true; coords.insert(id, (0, 0)).
fn tick() -> f32 { 1.0 / 60.0 }
```

Geometry used by the direct-`raycast_player` tests: shooter `1` at `(100, 200)`
→ bullet origin `(115.0, 219.0)`; victim `2` at `(400, 200)` → body
`x ∈ [400, 430]`, `y ∈ [200, 238]` (so `hit_x == victim.pos.x` for a rightward
shot).

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| S1 | `raycast_hits_player_same_room` | shooter 1 `(100,200,1.0)`, victim 2 `(400,200,1.0)`, both `(0,0)`. `assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)))`. Overlap clamp (same test): an origin inside the victim's rect → `assert_eq!(world.raycast_player(1, 410.0, 219.0, 1.0), Some((2, 410.0)))` (`d_near < 0` clamps `distance` to 0, so `hit_x == spawn_x`). |
| S2 | `raycast_misses_player_in_other_room` | same positions, but `world.coords.insert(2, (0, 1))` → `assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), None)`. |
| S3 | `raycast_blocked_by_wall` | same as S1; `world.levels[0][0].grid[5][5] = TileType::Wall` (row for `y = 219` is 5; col 5 is `x ∈ [200,240]`, inside the segment) → `assert_eq!(…, None)`; then `grid[5][5] = TileType::Empty` → `assert_eq!(…, Some((2, 400.0)))`. |
| S4 | `raycast_first_hit_wins` | shooter 1 `(100,200)`, victim 2 `(300,200)`, victim 3 `(500,200)` → `assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 300.0)))`. |
| S5 | `raycast_never_hits_shooter` | only shooter 1 → `assert_eq!(…, None)`; add victim 2 at `(400,200)` → `Some((2, 400.0))`; extra edge: move victim 2 to `(0,200)` (behind the shooter, `d_far < 0`) → `assert_eq!(…, None)`; `assert_ne!(returned_id, 1)` wherever `Some`. |
| S6 | `raycast_tie_breaks_on_lower_id` | shooter 1 `(100,200)`; spawn victim 5 **first** `(400,200)`, then victim 2 at the same `(400,200)` → `assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)))`. |
| S7 | `update_returns_bullet_and_hit_info` | shooter 1 `(100,200,1.0)`, victim 2 `(400,200)`, both `(0,0)`; `apply_input(1, 0, 0, PlayerInput { shoot: true, ..Default::default() }, 3)`; `let (shots, hits) = world.update(1.0/60.0);` → `assert_eq!(hits, vec![PlayerHitInfo { shooter_id: 1, victim_id: 2 }])`; `assert_eq!(shots.len(), 1)`; `assert_eq!(shots[0].owner_id, 1)`; `assert_eq!(shots[0].seq, 3)`; `assert_eq!(shots[0].dir, 1.0)`; `assert!((shots[0].x - 400.0).abs() < 0.01)` (**impact x**, not origin 115, not room edge 800). |
| S8 | `update_miss_emits_bullet_at_origin` | only shooter 1 `(100,200,1.0)` + shoot → `assert!(hits.is_empty())`; `assert_eq!(shots.len(), 1)`; `assert!((shots[0].x - 115.0).abs() < 0.01)`; `assert!((shots[0].y - 219.0).abs() < 1.0)` (allow one tick of gravity drift). |
| S9 | `update_respawns_victim_at_origin` | same as S7, but first give the victim a held input `apply_input(2, 0, 0, PlayerInput { move_left: true, ..Default::default() }, 0)` (so the pending reset is observable) and capture the shooter's `(pos, dir)` before. After `update`: `assert_eq!(hits, vec![PlayerHitInfo { shooter_id: 1, victim_id: 2 }])`; `assert_eq!(world.players()[&2].pos, glam::Vec2::new(100.0, 100.0))`; `assert_eq!(world.players()[&2].vel, glam::Vec2::ZERO)`; `assert_eq!(world.players()[&2].dir, 1.0)`; `assert!(!world.players()[&2].grounded)`; `assert_eq!(world.coords()[&2], (0, 0))`; all four `world.pending[&2]` flags `false`; shooter x/dir unchanged and `(shooter.pos.y - before.y).abs() < 1.0`. (`PlayerInput` has no `PartialEq` — assert fields individually.) |
| S10 | `update_dedupes_one_victim_per_tick` | shooter 1 `(100,200,1.0)`, shooter 3 `(700,200,-1.0)`, victim 2 `(400,200)`; shoot for both shooters (seq 1 and 2) in the same tick → `assert_eq!(hits.len(), 1)`; `assert_eq!(hits[0], PlayerHitInfo { shooter_id: 1, victim_id: 2 })` (ascending shooter order credits the first); `assert_eq!(shots.len(), 2)`; `assert_eq!(shots[0].owner_id, 1)`; `assert_eq!(shots[1].owner_id, 3)` (deterministic sort order). |

Why S10's geometry: two shooters on the same side would block each other (the
nearer player is the hit), hence shooter 3 faces left from the far side; both
rays reach the victim and the victim is the nearest candidate for each.

### Client — new `#[cfg(test)] mod tests` in `client/src/network/world.rs` (3)

Test double (the existing `DummyPacketIO`s record nothing):

```rust
use std::rc::Rc;
use std::cell::RefCell;
#[derive(Clone, Default)]
struct Sends(Rc<RefCell<Vec<GamePacket>>>);
struct RecordingPacketIO { sent: Sends }
// try_recv -> None; send -> self.sent.0.borrow_mut().push(packet)
```

| # | Test | Exact assertion |
| --- | --- | --- |
| C1 | `player_died_for_self_respawns_local_player` | World with `id = Some(7)`, `player.id = 7`; dirty `player.pos = (500,300)`, `vel = (1,2)`, `grounded = true`, `dir = -1.0`, `current_coords = (0,1)`, `client_tick = 42`, `server_tick_offset = -3`, `clock_offset_ms = 99`, `last_server_tick = 7`, `last_server_time_ms = 8`, `last_acked_server_tick = 9`. `handle_packet(&mut world, GamePacket::PlayerDied { id: 7 });` → `player.pos == Vec2::new(100.0, 100.0)`, `player.vel == Vec2::ZERO`, `player.dir == 1.0`, `!player.grounded`, `player.id == 7`, `current_coords == (0,0)`, all six clock fields `== 0`, `prev_player_pos == player.pos`, `prev_player_dir == player.dir`, `bullets` / `spawned_bullet_ids` / `confirmed_bullets` empty, and `sent.borrow().is_empty()` (no packet at all — `handle_packet` never sends). |
| C2 | `player_died_for_remote_removes_nemesis` | `id = Some(1)`; insert `other_players[2] = Nemesis::new(2, Vec2::new(400.0, 200.0))` + `other_interp.entry(2).or_default()`, and a control `other_players[3]` + `other_interp.entry(3).or_default()`. `handle_packet(&mut world, GamePacket::PlayerDied { id: 2 });` → `!other_players.contains_key(&2)`, `!other_interp.contains_key(&2)`, `other_players.contains_key(&3)`, `other_interp.contains_key(&3)`, `world.id == Some(1)`, `player.pos == Vec2::new(100.0, 100.0)` (untouched). |
| C3 | `remote_bullet_contact_does_not_kill_or_send` | `id = Some(1)`; `other_players.insert(2, Nemesis::new(2, …))` (so the bullet survives the retain); `remote_bullets.push(Bullet::new(Vec2::new(110.0, 115.0), 1.0, 2))` (overlaps the local rect `100..130 × 100..138`); capture `before = player.pos`; `world.update(1.0 / TICK_HZ as f32)` → `(player.pos - before).length() < 2.0` (not teleported; prediction may move < 1 px), `current_coords == (0,0)`, and `sent.borrow().iter().all(|p| !matches!(p, GamePacket::PlayerDied { .. }))` — **zero death packets** (a `PlayerInput` heartbeat is expected because `id.is_some()`; do **not** assert zero total sends). |

### E2E — new `e2e/tests/authoritative_death.rs` (2)

```rust
use e2e::harness::{ClientHandle, TestHarness};
use metroidvania_server::PlayerHitInfo;
```

Private helper in the test file:

```rust
fn setup_pvp(h: &mut TestHarness) -> (ClientHandle, ClientHandle)
// connect shooter, connect victim
// set_server_player_coords(shooter, 0, 0) / (victim, 0, 0)
// set_server_player_pos(shooter, 100.0, 201.0) / (victim, 400.0, 201.0)
// set_server_player_grounded(.., true) for both
// set_client_player_pos(.., same) for both
// h.tick_n(3)
// return (shooter, victim)
```

Room `(0,0)`, row 5 (`y ≈ 201`) is fully open between the border walls;
`TILE_SIZE = 40`, so `gy = floor(201/40) = 5`. The shooter's bullet origin is
`(115, 220)`; the victim's body is `x ∈ [400,430]`, `y ≈ [201,239]` — a vertical
overlap, no wall, hit at `hit_x = 400`.

| # | Test | Exact assertion |
| --- | --- | --- |
| E1 | `e2e_shoot_kills_player_in_same_room` | Setup via `setup_pvp`; `let shooter_id = h.client(shooter).player_id; let victim_id = h.client(victim).player_id;` capture `let (sx, sy) = h.client_player_pos(shooter);` then `h.press_shoot(shooter); for _ in 0..5 { h.tick(); if !h.last_deaths.is_empty() { break; } }` → `assert_eq!(h.last_deaths.len(), 1)`; `assert_eq!(h.last_deaths[0], PlayerHitInfo { shooter_id, victim_id })`; victim client: `let (vx, vy) = h.client_player_pos(victim); assert!((vx - 100.0).abs() <= 2.0 && (vy - 100.0).abs() <= 2.0);` and `assert_eq!(h.client_player_coords(victim), (0, 0))`; shooter did not respawn: `let (x, y) = h.client_player_pos(shooter); assert!((x - sx).abs() <= 5.0 && (y - sy).abs() <= 5.0)`. |
| E2 | `e2e_victim_does_not_self_report` | Same setup + shoot + tick loop until `!h.last_deaths.is_empty()` (assert it became non-empty so the test is not vacuous), then `assert_eq!(h.client_death_reports, 0)` (no client ever sent `PlayerDied`). |

`last_deaths` is refreshed every tick, so E1/E2 must break out of the loop on the
same tick the death appears (never read it after a later tick).

Within a harness tick the `PlayerDied` fan-out happens **before** the `PlayerPos`
and `BulletSpawn` fan-outs (Step 4b), mirroring `server/src/main.rs`. The victim's
client therefore resets via `respawn_local` first and then receives the same
tick's `BulletSpawn` — which is also what keeps
`bullet_spawn.rs::test_e2e_bulletspawn_both_clients` passing (its two clients
overlap at spawn, so the shooter's shot kills the observer at `distance = 0`; if
`PlayerDied` arrived after `BulletSpawn`, `respawn_local` → `load_current_level`
would clear the just-pushed `remote_bullets` and that test's
`assert_eq!(client_b.world.remote_bullets.len(), 1)` would fail).

### Handoff order (what the tester can RED when)

1. **Immediately RED-able (no wiring needed):** P1, G1–G5, and the S1–S10 shapes.
   P1 fails at compile time until `GamePacket::PlayerDied` exists; G1–G5 fail at
   compile time until `Level::solid_between` exists; the `raycast_*` tests fail at
   compile time until `PlayerHitInfo` / `raycast_player` exist. Compile-error RED
   is expected and accepted for these.
2. **Needs Box 1 (variant) + Box 5 (handler):** C1–C3. C3 also needs `World::update`
   to stop calling `check_player_death`.
3. **Needs Box 6 (harness fields):** E1–E2 (`last_deaths`,
   `client_death_reports`, `set_server_player_coords`).

Run per crate to read the RED state without the unrelated compile cascade:
`cargo test -p protocol`, `cargo test -p game`, `cargo test -p metroidvania-server --lib`,
`cargo test -p metroidvania-rust`, `cargo test -p e2e`.

## Ripple surfaces the spec's Gotchas call out

| Surface | Sites | Why it breaks |
| --- | --- | --- |
| `client/src/network/transport.rs` | 2 (`RealPacketIO::send`, `HeadlessPacketIO::send`) | `GamePacket::Hit { .. }` in the reliability match → no such variant |
| `client/src/network/logging.rs` | 6 (3 code arms + 3 test assertions) | exhaustive `match` on `GamePacket` + stale assertions |
| `server/src/main.rs` | 5 (3 logging helpers, inbound arm, outgoing construction) | `packet_type_name` is exhaustive (no catch-all) |
| `server/src/world.rs` `world.update(...)` | **7** bindings: L297, L306, L331, L482, L526, L532, L547 | tuple return; the 5 discarding calls (L266, L316, L353, L378, L402) need **no** change |
| `e2e/src/harness.rs` | Step 3, Step 4, Step 4b | tuple return + new death fan-out (immediately after Step 4, before `PlayerPos`/`BulletSpawn`) + self-report counter |
| `client/src/world/lib.rs` | `use protocol::GamePacket;` | unused after the `Hit` send is deleted → `clippy -D warnings` |
| e2e geometry | `TILE_SIZE = 40`; row 5 `y ≈ 201`; spawn centre `y = 119` sits on the row 2/3 boundary | do not put the PvP scenario on the spawn row |
| determinism | `self.players` is a `HashMap` | sort `shoot_requests` by `player_id` before the second pass |
| stale comments | `client/src/world/game.rs` L151–152, L186–187; `server/src/world.rs` L158–160 | they describe the removed death-collision / "next spec" behavior |

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Box 1 — Protocol rename `Hit` → `PlayerDied`

- [x] done
- **Files**: `protocol/src/lib.rs`; `protocol/tests/round_trip.rs`.
- **Change**:
  - `protocol/src/lib.rs`: replace the `Hit { entity_id: u8 }` variant **in
    place** (same position, between `Leave` and `PlayerInput`) with
    `PlayerDied { id: u8 }`. No other variant/field changes, no reordering.
  - `protocol/tests/round_trip.rs`: rename `round_trip_hit` →
    `round_trip_player_died`; body becomes
    `round_trip(GamePacket::PlayerDied { id: 0 });` and
    `round_trip(GamePacket::PlayerDied { id: 255 });`. Nothing else in the file
    changes.
- **Acceptance**: `cargo test -p protocol` green (P1); `grep -rn 'Hit\|entity_id'
  protocol/src protocol/tests` → nothing. The rest of the workspace is
  **expected to fail to compile** through Box 5 (and `e2e/src/harness.rs`
  through Box 6) — do not "fix" it in this box.
- **Test**: P1.
- **Explain**
  - What: Replaced the `Hit { entity_id: u8 }` enum variant with
    `PlayerDied { id: u8 }` in the same enum slot; the tester's `round_trip_player_died`
    test already covers it.
  - Why: Spec 2 makes death server-authoritative; the packet is now a
    server→client notification, not a client self-report, so its name and field
    reflect the new meaning.
  - Rust concepts: enum variant rename (the payload stays one `u8`); deriving
    `Serialize`/`Deserialize` and `PartialEq` continues to make postcard
    round-trips comparable with `assert_eq!`.
  - Watch out: this rename intentionally breaks compilation in the server binary
    and client crate until Boxes 4–5; only `cargo test -p protocol` is expected
    to pass now.

### Box 2 — `Level::solid_between` + unit tests

- [x] done
- **Files**: `game/src/level.rs`.
- **Change**:
  - Add inside `impl Level`, directly after `solid_in_rect`:

    ```rust
    /// True if any `TileType::Wall` tile blocks the horizontal segment from
    /// `x0` to `x1` (order-insensitive) at height `y`.
    pub fn solid_between(&self, x0: f32, x1: f32, y: f32) -> bool
    ```

    Semantics (exactly the spec's): `gy = (y / TILE_SIZE).floor() as i32`; if
    `gy < 0 || gy >= self.height as i32` return `false`; `x_min/x_max` of
    `x0,x1`; `gx_start`/`gx_end` from `floor(x / TILE_SIZE)`; for `gx in
    gx_start..=gx_end`, return `true` if the column is in
    `0..self.width` and `self.grid[gy][gx] == TileType::Wall`; else `false`.
  - Add the `wall_below_row_level()` fixture and tests G1–G5 to the existing
    `#[cfg(test)] mod tests` (keep every existing test untouched).
  - Do **not** change `solid_point`, `colision`, `solid_in_rect`, `grounded`.
- **Acceptance**: `cargo test -p game` green, including G1–G5; G3 proves
  `solid_in_rect` would have been wrong for the ray (1-px rect at `y = 119`
  spans rows 2 and 3).
- **Test**: G1–G5.
- **Explain**
  - What: Added `Level::solid_between(x0, x1, y)` after `solid_in_rect`: it
    floors the ray height to one tile row and scans the inclusive column range
    between the two x endpoints (order-insensitive), ignoring out-of-grid
    columns.
  - Why: Server hit detection needs wall occlusion for a 1-px-high ray; the
    existing `solid_in_rect` walks every overlapping row, so a ray on a tile
    boundary would be blocked by the row below (G3 pins this regression).
  - Rust concepts: `f32::min`/`max` for order-insensitivity, `floor() as i32`,
    inclusive range iteration (`..=`), slice indexing with bounds checks.
  - Watch out: `TILE_SIZE` is 40, so `y = 119` is row 2, not row 2–3; out-of-grid
    rows/columns return `false` (not solid), unlike the private `solid_point`.

### Box 3 — `ServerWorld`: `PlayerHitInfo`, `raycast_player`, `respawn`, new `update`

- [x] done
- **Files**: `server/src/world.rs`.
- **Change**:
  - Add `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct PlayerHitInfo {
    pub shooter_id: u8, pub victim_id: u8 }` near `BulletSpawnInfo`.
  - Change `update` to `pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>,
    Vec<PlayerHitInfo>)`; update its doc comment (shots **and** kills; victims
    are respawned before returning).
  - First pass: unchanged (post-movement origin `(pos.x + w/2.0, pos.y + h/2.0)`,
    `dir = player.dir`, `shoot: false` into `player.update`, latch/cooldown logic).
  - Second pass (replace the current one):
    1. `shoot_requests.sort_by_key(|r| r.player_id);` **before** iterating (the
       `HashMap` iteration order is otherwise nondeterministic — bullet ids and
       multi-victim results depend on this).
    2. For each request: `let hit = self.raycast_player(req.player_id,
       req.spawn_x, req.spawn_y, req.dir);`
    3. Assign `bullet_id` exactly as today (`next_bullet_id`,
       `wrapping_add(1)`).
    4. Emit exactly one `BulletSpawnInfo` with
       `x = hit.map_or(req.spawn_x, |(_, hit_x)| hit_x)`, `y = req.spawn_y`,
       `dir = req.dir`, `owner_id = req.player_id`, `seq = req.player_seq`.
    5. If `Some((victim_id, _))` and no `PlayerHitInfo` for that `victim_id` is
       already collected this tick, push `PlayerHitInfo { shooter_id:
       req.player_id, victim_id }` (first shot in ascending shooter-id order
       wins; at most one death per victim per tick).
    6. After the loop: for each collected hit, `self.respawn(victim_id)` (iterate
       the local `Vec`; `respawn` takes `&mut self`, the vec is a local).
    7. `self.current_tick += 1; return (bullet_spawns, player_hits);`
    - Replace the stale comment block ("There is no hit-detection target after
      the PvE removal (spec 1); the next spec re-adds …") with the two
      consequences: hit tests see this tick's post-movement positions; respawns
      run after the loop so a victim cannot be teleported mid-pass or hit twice;
      a victim that also shot still fires.
  - Add `fn raycast_player(&self, shooter_id: u8, spawn_x: f32, spawn_y: f32,
    dir: f32) -> Option<(u8, f32)>` exactly per the spec (direction sign, room
    filter with `(0,0)` fallback and `levels` fallback, self exclusion, victim
    `w`/`h` hitbox, inclusive vertical overlap, `d_near`/`d_far`/`distance`/
    `hit_x`, `level.solid_between(spawn_x, hit_x, spawn_y)` occlusion, ascending
    victim ids, strict-`<` best — tie goes to the lower id).
  - Add `fn respawn(&mut self, id: u8)` — **intentional deviation from the
    spec's literal `*self.players.get_mut(&id) = Player::new(id)`** (that form
    requires `Option::expect`/`unwrap`): use `if let Some(p) =
    self.players.get_mut(&id) { *p = Player::new(id); }`, which is behaviorally
    identical for a spawned id and keeps AGENTS.md's "no `unwrap`/`expect` in
    non-test code" rule. Do not "restore" the spec's literal form in review.
    Then `self.coords.insert(id, (0, 0));`,
    `self.pending.insert(id, PlayerInput::default());`. Do **not** touch
    `shoot_cooldown` / `max_seq`.
  - Add the new `#[cfg(test)] mod raycast_and_hit_tests` with S1–S10 and the
    `place`/`tick` helpers.
  - Fix the existing `let shots = world.update(...)` bindings (7 sites: L297,
    L306, L331, L482, L526, L532, L547) to `let (shots, _hits) = …` (or `let
    (shots, _)`). Leave the 5 discarding calls (L266, L316, L353, L378, L402)
    alone.
- **Acceptance**: `cargo test -p metroidvania-server --lib` green (S1–S10 plus
  all pre-existing world tests). The `server` **binary** still fails to compile
  (it references `Hit`) — Box 4 fixes it.
- **Test**: S1–S10.
- **Explain**
  - What: Added `PlayerHitInfo`; rewrote `update`'s second pass to sort
    `shoot_requests` by `player_id`, raycast each shot, spawn the bullet at the
    impact x on a hit, dedupe victims to one death per tick, and respawn after
    the loop. Added private `raycast_player` and `respawn`; destructured the 7
    pre-existing `world.update` bindings.
  - Why: The server must be the sole authority on PvP death: it resolves the
    hit from authoritative post-movement positions, resets the victim, and
    returns the kills for `main.rs` to broadcast.
  - Rust concepts: tuple return + destructuring, `Option::map_or`,
    `sort_by_key`/`sort_unstable` over `HashMap`-derived iteration, `&self`
    borrow for the raycast followed by `&mut self` respawn after the loop,
    `if let Some` instead of `unwrap`, `derive(Debug, Clone, Copy, PartialEq,
    Eq)`.
  - Watch out: impact-x bullets (not the origin), inclusive vertical overlap,
    `distance = d_near.max(0.0)` for overlapping players, ascending victim-id
    iteration with strict `<` for ties, and `TILE_SIZE = 40` row semantics. The
    server binary still fails to compile until Box 4.

### Box 4 — `server/src/lib.rs` re-export + `server/src/main.rs`

- [x] done
- **Files**: `server/src/lib.rs`; `server/src/main.rs`.
- **Change**:
  - `server/src/lib.rs`: replace the two lines with
    `pub use world::{BulletSpawnInfo, PlayerHitInfo, ServerWorld};`.
  - `server/src/main.rs`:
    - `packet_type_name`: `GamePacket::Hit { .. } => "Hit"` →
      `GamePacket::PlayerDied { .. } => "PlayerDied"`.
    - `packet_id`: `GamePacket::Hit { entity_id } => Some(*entity_id)` →
      `GamePacket::PlayerDied { id } => Some(*id)`.
    - `packet_key_fields`: → `json!({ "id": id })`.
    - Replace the whole received-`Hit` block (the `println!("Player {} died")`
      plus the partial rebroadcast loop, L224–239) with an explicit no-op arm
      and a comment:
      `GamePacket::PlayerDied { .. } => {}` — *server→client only; a client must
      never report a death (defense in depth, no state change, no broadcast)*.
      (It must sit before the final `_ => {}`.)
    - `let shots = world.update(tick_dt);` →
      `let (shots, deaths) = world.update(tick_dt);`.
    - Insert the death broadcast **immediately after `update` and before the
      `PlayerPos` nested loop** (the spec's tick order is update → `PlayerDied` →
      `PlayerPos` → `BulletSpawn`): for each `hit in &deaths`, build
      `GamePacket::PlayerDied { id: hit.victim_id }`, `println!("Player {} was
      killed by {}", hit.victim_id, hit.shooter_id)`, `log_packet("sent",
      &packet)`, serialize once, then
      `packet_sender.send(Packet::reliable_ordered(dest_addr, bytes.clone(), None))`
      for **every** `addrs.keys()` (including the victim).
    - Leave the `PlayerPos` and `BulletSpawn` broadcast blocks otherwise
      untouched.
- **Acceptance**: `cargo check -p metroidvania-server` green (lib **and** bin);
  `cargo test -p metroidvania-server` green (S1–S10 unaffected). `grep -rn
  'Hit { entity_id\|"Hit"\|entity_id' server` → nothing.
- **Test**: S1–S10 still green. There is **no dedicated log-format coverage** for
  `PlayerDied`: `e2e/tests/packet_log_rate.rs` only exercises
  `PlayerInput` (`"type":"PlayerInput"`), so the renamed
  `packet_type_name`/`packet_id`/`packet_key_fields` arms in `server/src/main.rs`
  are compile-checked but not asserted by any test. Optional (not required by
  the spec): add one `PlayerDied` line to that suite's expected-log check; do not
  claim existing coverage.
- **Explain**
  - What: Re-exported `PlayerHitInfo` from the server crate; renamed the three
    logging arms, replaced the inbound `Hit` handler with an ignoring
    `PlayerDied` arm, destructured `world.update`, and broadcast one
    `PlayerDied { id: victim_id }` to every address before the `PlayerPos` and
    `BulletSpawn` fan-outs.
  - Why: `main.rs` is the only place that owns sockets, so it turns the server's
    authoritative kill list into the reliable-ordered death notification. The
    inbound arm is a no-op so a client can never forge a death.
  - Rust concepts: `pub use` re-exports; match-arm rename; iterating a borrowed
    `Vec<PlayerHitInfo>` while cloning serialized bytes for each destination;
    `postcard::to_allocvec` + `bytes.clone()` to fan out one serialization.
  - Watch out: tick order is update → `PlayerDied` → `PlayerPos` → `BulletSpawn`;
    the victim is included in the broadcast; the old partial-rebroadcast loop is
    gone.

### Box 5 — Client: drop self-report + client death collision, handle `PlayerDied`

- [x] done
- **Files**: `client/src/network/transport.rs`; `client/src/network/logging.rs`;
  `client/src/world/lib.rs`; `client/src/network/world.rs`;
  `client/src/world/game.rs`.
- **Change**:
  - `client/src/network/transport.rs`: in **both** `PacketIO::send` match blocks
    drop `| GamePacket::Hit { .. }` so the arm is just
    `GamePacket::JoinRequest => reliable_ordered`, `_ => unreliable`. (The client
    never sends `PlayerDied`.)
  - `client/src/network/logging.rs`: `packet_type_name` → `GamePacket::PlayerDied
    { .. } => "PlayerDied"`; `packet_id` → `GamePacket::PlayerDied { id } =>
    Some(*id)`; `packet_key_fields` → `json!({ "id": id })`; update the three
    test assertions (`packet_type_name_all_variants`,
    `packet_id_all_variants`, `packet_key_fields_all_variants`) to construct
    `PlayerDied { id: 1 }` / `{ id: 9 }`.
  - `client/src/world/lib.rs`:
    - **Delete** `check_player_death` entirely.
    - Rename `handle_death` → `pub fn respawn_local(world: &mut World)`; delete
      the `GamePacket::Hit` send; delete the now-redundant
      `world.player.pos = Vec2::new(100.0, 100.0)`; add
      `let id = world.player.id; world.player = Player::new(id);` **before** the
      `prev_player_*` snap; keep the rest of the reset set (print,
      `current_coords = (0,0)`, clear `spawned_bullet_ids` /
      `confirmed_bullets`, the six clock resets, `load_current_level`).
    - Drop the now-unused `use protocol::GamePacket;` (clippy) and add
      `use game::entities::player::Player;`.
  - `client/src/network/world.rs`: replace the `GamePacket::Hit` arm with the two
    `PlayerDied` arms (guard arm first):

    ```rust
    GamePacket::PlayerDied { id } if Some(id) == world.id => {
        crate::world::lib::respawn_local(world);
    }
    GamePacket::PlayerDied { id } => {
        // The server already respawned the victim; drop the stale body and
        // interpolation so nothing is drawn at the death position. The next
        // PlayerPos re-creates the Nemesis at the respawn point.
        world.other_players.remove(&id);
        world.other_interp.remove(&id);
    }
    ```

  - `client/src/world/game.rs`: remove `check_player_death` from the `use
    crate::world::lib::{...}` list and the `check_player_death(self);` call in
    `World::update`; rewrite the stale comments at L151–152 ("Run transition
    check AFTER remote-bullet collision so bullets can hit the local player …")
    and L186–187 — remote bullets are purely visual now, the transition check
    still runs after the bullet updates so `load_current_level` clears them.
  - Add the C1–C3 client tests with `RecordingPacketIO`.
- **Acceptance**: `cargo check -p metroidvania-rust --all-targets` green;
  `cargo test -p metroidvania-rust` green (C1–C3 + existing client tests);
  `grep -rn 'check_player_death\|handle_death\|GamePacket::Hit' client` →
  nothing.
- **Test**: C1–C3.
- **Explain**
  - What: Removed the `Hit` reliability arms and log arms, deleted the
    client-side death collision (`check_player_death`) and its `update` call,
    renamed `handle_death` → `respawn_local` (no packet, rebuilds the player via
    `Player::new`), and handled `PlayerDied` for self (local respawn) and remotes
    (drop `Nemesis` + interpolation).
  - Why: The client must never decide a death; it only mirrors the server's
    authoritative respawn, which also removes the self-report attack surface.
  - Rust concepts: exhaustive `match` arm replacement with a guard arm
    (`if Some(id) == world.id`); `HashMap::remove` for remote cleanup; moving a
    fresh `Player::new(id)` into `world.player`; module-path calls
    (`crate::world::lib::respawn_local`).
  - Watch out: `respawn_local` rebuilds the player **before** snapping
    `prev_player_*`; duplicate `PlayerDied` is harmless; remote bullets stay
    simulated but purely visual.

### Box 6 — E2E harness + PvP death e2e

- [x] done
- **Files**: `e2e/src/harness.rs`; create `e2e/tests/authoritative_death.rs`.
- **Change**:
  - `e2e/src/harness.rs`:
    - `use metroidvania_server::{PlayerHitInfo, ServerWorld};`.
    - `TestHarness` fields: `pub last_deaths: Vec<PlayerHitInfo>` and
      `pub client_death_reports: usize`; initialise both in `new()`.
    - `tick`, Step 3: decode each drained packet once and match — the
      `PlayerInput` arm keeps calling `server.apply_input(...)`; add
      `GamePacket::PlayerDied { .. } => { self.client_death_reports += 1; }`
      (should always stay 0) and `_ => {}`. (Keep the existing
      `for rx in &self.from_clients` shape — disjoint-field borrows are
      unchanged.)
    - `tick`, start: `self.last_deaths.clear();`.
    - `tick`, Step 4: `let (shots, deaths) = self.server.update(tick_dt);` and
      after the `total_shots`/`last_shots` loop,
      `self.last_deaths.extend(deaths.iter().copied());`.
    - `tick`, **Step 4b** — immediately after Step 4 (`world.update` +
      `last_deaths` fill) and **before** the Step 5 `PlayerPos` loop and Step 6
      `BulletSpawn` fan-out (this mirrors `server/src/main.rs`'s mandated order:
      update → `PlayerDied` → `PlayerPos` → `BulletSpawn`): for each
      `hit in &self.last_deaths`, serialize
      `GamePacket::PlayerDied { id: hit.victim_id }` once and send it to
      **every** `self.to_clients` channel (mirrors the server's all-clients
      broadcast; includes the victim's own channel).
      *Why this order and not after `BulletSpawn`: the victim's client must run
      `respawn_local` (which clears `remote_bullets` via `load_current_level`)
      before the same tick's `BulletSpawn` is applied. Placing the fan-out after
      Step 6 breaks `e2e/tests/bullet_spawn.rs::test_e2e_bulletspawn_both_clients`,
      where both clients spawn overlapping at `(100,100)` and the shooter's shot
      kills the observer (`distance = 0`); the observer's `remote_bullets` would
      be cleared after the spawn and `assert_eq!(…, 1)` at
      `bullet_spawn.rs:200` would fail.*
    - Add `pub fn set_server_player_coords(&mut self, handle: ClientHandle, x:
      u8, y: u8)` delegating to `self.server.apply_input(player_id, x, y,
      PlayerInput::default(), 0)`.
    - `total_shots`/`last_shots` unchanged.
  - Create `e2e/tests/authoritative_death.rs` with `setup_pvp` + E1 + E2.
- **Acceptance**: `cargo test -p e2e` green — E1–E2 plus the movement /
  multiplayer / reconciliation / rooms / shoot / edge-case suites. Note that
  `bullet_spawn.rs::test_e2e_bulletspawn_both_clients` is **not**
  ordering-neutral: it only keeps passing because the harness now delivers
  `PlayerDied` before `BulletSpawn` in the same tick (see Step 4b). Then the
  full gate: `cargo test --workspace`, `cargo fmt --check --all`,
  `cargo clippy --all-targets --all-features --workspace -- -D warnings`.
- **Test**: E1–E2 (plus `bullet_spawn`, which pins the Step 4b ordering).
- **Explain**
  - What: Rippled the `update` tuple through the harness, added `last_deaths`
    and `client_death_reports`, fanned `PlayerDied` out in a new Step 4b before
    `PlayerPos`/`BulletSpawn`, added `set_server_player_coords`, and kept the
    tester's `authoritative_death.rs` cases green.
  - Why: The in-process harness must mirror the real server's tick order and
    authority, including counting any (currently zero) client death reports.
  - Rust concepts: tuple destructuring; `Vec::extend(iter.copied())` for
    `Copy` elements; disjoint-field borrows while iterating `&self.from_clients`
    and mutating `self.server`/`self.client_death_reports`; sharing serialized
    bytes across channels with `clone()`.
  - Watch out: Step 4b must precede `PlayerPos`/`BulletSpawn` or
    `bullet_spawn.rs::test_e2e_bulletspawn_both_clients` breaks; `last_deaths`
    is cleared every tick, so readers must break on the same tick as the death.

## Definition of done

- [x] `cargo test --workspace` passes (21 named tests above present; ~20 net new
      test functions).
- [x] `cargo fmt --check --all` clean.
- [x] `cargo clippy --all-targets --all-features --workspace -- -D warnings` clean.
- [x] Grep-clean over code only (historical `docs/` hits are fine):

  ```bash
  grep -RIn 'GamePacket::Hit\|round_trip_hit\|entity_id\|check_player_death\|handle_death' \
    protocol/ server/ client/ game/ e2e/
  # → no output
  ```

- [x] No `Hit` variant remains; every other `GamePacket` variant is byte-identical.
- [x] `cargo test -p e2e` proves the pre-existing suites still pass. Only the
      `update` tuple ripple and the log renames touch shared code; additionally
      `bullet_spawn.rs::test_e2e_bulletspawn_both_clients` depends on the new
      PlayerDied-before-BulletSpawn harness ordering (Step 4b).
- [x] `PlayerInput.level_x/level_y` and client-claimed `coords` are still present
      (spec 3 owns them).
- [ ] Process: update `docs/progress.md` (status + next actions: spec 3 is next),
      add `docs/lessons.md` entries for `sort_by_key` as the determinism fix for
      HashMap-driven bullet-id/hit order and for the `&self` raycast read
      followed by the `&mut self` respawn write (borrow split), and record an ADR
      for the 8 ambiguities resolved in this plan.

## Explain (commit body template — fill per box)

```text
## What
<the box's changes, one sentence per crate>

## Why
<how this box serves "server is the sole authority on player death">

## Rust concepts
<e.g. enum-variant rename in place; tuple return + destructuring ripples;
 Option::map_or and Option<(u8,f32)>; HashMap iteration order vs sort_by_key;
 &self borrow for the raycast + &mut self for respawn after the loop;
 pub use re-exports; borrow-splitting when iterating self.last_deaths while
 sending through self.to_clients>

## Watch out
<the expected RED window (end of Box 1 → end of Box 6 for the workspace); the 7
 destructure sites; impact-x bullet; TILE_SIZE = 40 / row 5; harness Step 4b must
 precede BulletSpawn or bullet_spawn.rs breaks; no room authority / no i-frames>
```

## Open questions / ambiguities (for the plan reviewer)

1. **C3's "zero sends" is impossible literally.** `World::update` →
   `update_network` → `send_input` emits one `PlayerInput` every tick when
   `world.id.is_some()`. The plan therefore asserts **zero `PlayerDied` sends**
   (and keeps `id = Some(1)` so the old self-report path would have fired). The
   alternative — `id = None` and asserting zero total sends — would make the
   test vacuous.
2. **Harness death-broadcast position — RESOLVED: aligned with `main.rs`.** The
   fan-out goes in the harness **immediately after Step 4** (`world.update`),
   before the `PlayerPos` and `BulletSpawn` fan-outs, matching the spec's
   "Ordering in the tick" (`update → PlayerDied → PlayerPos → BulletSpawn`).
   Putting it after `BulletSpawn` is not just a cosmetic difference: the victim's
   `respawn_local` → `load_current_level` clears `remote_bullets`, which would
   fail `bullet_spawn.rs:200` in `test_e2e_bulletspawn_both_clients` (the two
   clients overlap at spawn and the shot kills the observer at `distance = 0`).
3. **`update` destructure count.** The spec says "≈6 sites"; there are **7**
   `let shots… = world.update(...)` bindings (L297, L306, L331, L482, L526,
   L532, L547) plus 5 discarding calls that need no change. Enumerated in
   Box 3.
4. **S9's `coords == (0,0)` assertion is tautological** in a same-room scenario
   (the victim must already be in `(0,0)` to be hittable). The plan strengthens
   the test with a held victim input so the `pending` reset is observable, and
   keeps the coords assertion as a sanity check rather than moving the scenario
   to another room (that would couple the unit test to map data).
5. **Client test location.** The spec names the three tests but not the module.
   The plan puts them in a new `#[cfg(test)] mod tests` in
   `client/src/network/world.rs` (`handle_packet` lives there) with an
   `Rc<RefCell<Vec<GamePacket>>>` recorder — no existing fake records sends
   (`DummyPacketIO` in `world/game.rs` and `network/mod.rs` discards them).
6. **`set_server_player_coords` is redundant for E1/E2.** Clients already claim
   `(0,0)` in every `PlayerInput`, so the helper's effect is overwritten each
   tick. Implemented and used as specified (it makes the fixture explicit and is
   the seam spec 3 will need).
7. **Vertical drift in the `update_*` server tests.** `GRAVEDAD = 20.0` at
   `dt = 1/60` drifts the players' `y` by < 1 px per tick; the assertions on
   `shots[0].y` and on the victim's centre are given ≤ 1 px tolerances, and the
   pure `raycast_*` tests call the method directly (no movement) so they stay
   exact.
8. **Leftward impact x.** `hit_x` on a leftward shot is the victim's right
   (near) edge; only the rightward case is pinned by S1/S7. Noted so the
   implementer does not special-case it.
9. **Intentional deviation from the spec's `respawn` body.** The spec writes
   `*self.players.get_mut(&id) = Player::new(id)`, which needs
   `Option::expect`/`unwrap`; the plan uses
   `if let Some(p) = self.players.get_mut(&id) { *p = Player::new(id); }`
   instead, per AGENTS.md's no-`unwrap`-in-non-test-code rule. Behaviorally
   identical for a spawned id — code review should not read this as drift.
