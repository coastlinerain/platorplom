# Authoritative room membership — Implementation Plan

- Status: draft
- Spec: `docs/specs/authoritative-room-membership.md`
- Date: 2026-11-26
- Branch: `feat/authoritative-room-membership` (already branched from `dev`; do not create a branch)
- Roadmap: spec 3 of `docs/specs/netcode-hardening-roadmap.md`
- Pipeline: standard (spec → plan → plan review → tester writes RED → implementer green → code review → qa)

## Overview

Room membership becomes **server-authoritative for shooter and victim**:

1. `game` gains a pure-logic `RoomGraph` (`Direction`, `Door` incl.
   `Door::server_trigger`, `RoomCoords`, eight directed doors) plus
   `ROOM_CHANGE_COOLDOWN_TICKS` / `ROOM_CHANGE_RETRY_TICKS` /
   `DOOR_LEAD_TOLERANCE_PX`;
2. `protocol` drops `PlayerInput.level_x/level_y` and appends
   `RequestRoomChange { id, direction }` / `RoomChangeResponse { accepted, level_x,
   level_y, x, y }` after `BulletSpawn` (wire-index stability), taking `game` as a
   dependency for the shared `Direction`;
3. the server validates `RequestRoomChange` against its authoritative rect, the
   door graph (via the **lead-expanded** `Door::server_trigger`: the geometric
   band plus `DOOR_LEAD_TOLERANCE_PX` on the approach edge) and a per-player tick
   cooldown, teleports to the door's canonical entry on acceptance, and answers
   the **requesting address only**;
4. the client stops mutating `current_coords` on movement; it sends one request
   when its **predicted** body reaches a door's raw geometric band (the client
   never uses `server_trigger`), applies the ack (or snaps back), and adopts the
   authoritative room from its own `PlayerPos` when that arrives first;
5. `check_level_transitions`, the `coords_match` reconciliation skip, and every
   client-claimed room packet field are gone.

Seven boxes. The tester writes the whole RED batch first (Boxes 1, 2, 3, 5, 6, 7
test cases below), so **the workspace is RED from the tester's RED batch through
the end of Box 6**, and e2e stays RED until Box 7. Each box below names a narrow
per-crate command that passes on its own (Box 5's caveat: C8–C11 stay RED until
Box 6). This mirrors the RED window in
`docs/plans/authoritative-player-death.md`.

## Non-goals (hard constraints — do NOT do these)

- **No lag compensation**; hits resolve against this tick's authoritative
  positions.
- **No PvE**, no health, no i-frames, no kill feed.
- **No checkpoint/room-local respawn.** Respawn stays `(0,0)` at `(100,100)`.
- **No broadcast scoping**: `PlayerPos`/`PlayerDied` still go to everyone.
- **No new rooms/tiles/doors**; the 2×2 map is the complete graph.
- **No diagonal doors, sub-room doors, or two doors in one direction.** A future
  map needs a door id (out of scope).
- **No door animation/fade/sound/delay.**
- **No protocol versioning / wire migration** (single workspace, both sides
  rebuilt together).
- **No change to movement, prediction replay dt, interpolation, or raycast
  geometry.**
- **Do not drop `PlayerPos.level_x/level_y`** — only `PlayerInput` loses room
  fields.
- **No client-side lead expansion.** `door_at` stays on the raw geometric
  `trigger`; `DOOR_LEAD_TOLERANCE_PX` / `server_trigger` are server validation
  only and grant no arbitrary teleport.
- **No new normal `glam` dependency in `server`**: `server` keeps `glam` as a
  dev-dependency; `RoomChangeInfo::pos` is a `(f32, f32)` pair and player fields
  are assigned component-wise.
- **No new `unwrap`/`expect` in the library code the quality gate checks**
  (`server/src/world.rs`, `client/src/network/world.rs`,
  `client/src/world/game.rs`, `client/src/world/lib.rs`). `server/src/main.rs`
  keeps its existing serialization `.unwrap()` style; the new
  `RequestRoomChange` arm uses `if let Ok(bytes) = postcard::to_allocvec(&response)`
  before sending (see Box 4).

## Public API (targets for the tester)

The tester compiles against these **before** the implementation exists. Exact
shapes:

```rust
// ---- game ----
// game/Cargo.toml (new normal dependency):
// serde = { version = "1.0.228", features = ["derive"] }
// game/src/lib.rs: add `pub mod room_graph;`

// game/src/room_graph.rs (new)
pub type RoomCoords = (u8, u8); // (level_x, level_y) = (column, row)

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    North, // variant index 0 — WIRE FORMAT, do not reorder
    South, // 1
    East,  // 2
    West,  // 3
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Door {
    pub from: RoomCoords,
    pub direction: Direction,
    pub target: RoomCoords,
    /// Geometric band used by the client's `door_at` detection.
    pub trigger: crate::rect::Rect,
    /// Canonical player top-left inside `target` after the teleport.
    pub entry: (f32, f32),
}

impl Door {
    /// Server-side validation rect: `trigger` grown by
    /// `DOOR_LEAD_TOLERANCE_PX` on the approach edge (East: left, West: right,
    /// South: top, North: bottom). The client never uses this.
    pub fn server_trigger(&self) -> crate::rect::Rect;
}

#[derive(Debug, Clone)]
pub struct RoomGraph {
    doors: Vec<Door>, // private
}

impl RoomGraph {
    pub fn new() -> Self;
    pub fn doors(&self) -> &[Door];
    pub fn door(&self, from: RoomCoords, direction: Direction) -> Option<Door>;
    pub fn door_at(&self, from: RoomCoords, player_rect: crate::rect::Rect) -> Option<Door>;
    pub fn can_enter(
        &self,
        from: RoomCoords,
        direction: Direction,
        player_rect: crate::rect::Rect,
    ) -> bool;
}

impl Default for RoomGraph {
    fn default() -> Self { Self::new() }
}

// game/src/config.rs (new constants, exact values)
pub const ROOM_CHANGE_COOLDOWN_TICKS: u8 = 12;
pub const ROOM_CHANGE_RETRY_TICKS: u8 = 6;
pub const DOOR_LEAD_TOLERANCE_PX: f32 = 8.0;

// ---- protocol ----
// protocol/Cargo.toml: game = { path = "../game" } (no cycle).
// `PlayerInput` loses level_x/level_y. Append the two variants at the END,
// after BulletSpawn:
GamePacket::RequestRoomChange {
    id: u8,
    direction: game::room_graph::Direction,
}
GamePacket::RoomChangeResponse {
    accepted: bool,
    level_x: u8,
    level_y: u8,
    x: f32,
    y: f32,
}
// PlayerPos is UNCHANGED (keeps level_x/level_y).

// ---- server ----
// server/src/world.rs — new type
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomChangeInfo {
    pub accepted: bool,
    pub room: RoomCoords,
    pub pos: (f32, f32), // authoritative player top-left
}

impl ServerWorld {
    pub fn apply_room_change(
        &mut self,
        id: u8,
        direction: Direction,
    ) -> Option<RoomChangeInfo>;
    pub fn force_teleport(&mut self, id: u8, room: RoomCoords, pos: (f32, f32));
    pub fn apply_input(&mut self, id: u8, input: PlayerInput, seq: u16); // was (id, x, y, input, seq)
}

// server/src/lib.rs
pub use world::{BulletSpawnInfo, PlayerHitInfo, RoomChangeInfo, ServerWorld};

// ---- client ----
// client/src/world/game.rs — new public World fields + helper
pub struct World {
    // ...existing fields...
    pub room_graph: game::room_graph::RoomGraph,
    pub pending_room_change: Option<game::room_graph::Direction>,
    pub room_change_armed: bool,
    pub room_change_retry: u8,
}
impl World {
    pub fn current_room(&self) -> game::room_graph::RoomCoords; // casts (usize, usize) → (u8, u8)
}

// client/src/world/lib.rs
pub fn request_room_change(world: &mut World);
pub fn apply_room_change_local(world: &mut World, room: RoomCoords, pos: glam::Vec2);

// client/src/lib.rs — re-export replacement
pub use world::lib::request_room_change; // was check_level_transitions

// ---- e2e harness (targets for the tester) ----
impl TestHarness {
    /// Room-only server teleport (`force_teleport`) that preserves the player's
    /// current position; replaces the old `apply_input` abuse.
    pub fn set_server_player_coords(&mut self, handle: ClientHandle, x: u8, y: u8);
    /// Cumulative `RoomChangeResponse` packets delivered to this client (proves
    /// per-requester routing, not a broadcast).
    pub fn room_change_responses(&self, handle: ClientHandle) -> usize;
}
pub struct TestHarness {
    // ...existing fields...
    /// Cumulative client→server `RequestRoomChange` packets observed in Step 3.
    pub client_room_change_requests: usize,
    /// Cumulative `accepted: false` `RoomChangeResponse`s sent. Must stay 0 for
    /// the deterministic walk-in test.
    pub client_room_change_rejects: usize,
}
```

Semantics the tester may rely on (all from the spec):

- `RoomGraph::new()` builds exactly the eight doors of the spec's table.
  `door_at` = `doors.iter().find(|d| d.from == from && d.trigger.overlaps(&rect)).copied()`
  (raw geometric `trigger`; table order breaks ties). `can_enter` =
  `door(from, direction).map(|d| d.server_trigger().overlaps(&rect)).unwrap_or(false)`
  (lead-expanded server rect).
- `Door::server_trigger()` grows only the approach edge by
  `DOOR_LEAD_TOLERANCE_PX` (8.0): East `x -= 8`, West `w += 8`, South `y -= 8`,
  North `h += 8`. Concretely for the first four doors: `(752, 320, 48, 40)`,
  `(0, 320, 48, 40)`, `(280, 432, 80, 48)`, `(280, 0, 80, 48)`.
- `Rect::overlaps` is strict (`<`); entry positions live inside the reverse
  door's band (anti-bounce is handled by `room_change_armed` + cooldown).
- `apply_room_change` returns `None` only for an unknown player. Rejections
  return `Some(RoomChangeInfo { accepted: false, room: from, pos: (pos.x, pos.y) })`
  and mutate nothing. Acceptance sets `coords`, `pos = door.entry`, zero velocity,
  recomputes `grounded` from the **target** level, and preserves `dir`, `bullets`,
  `pending`, `shoot_cooldown`, `max_seq`; it arms the cooldown to 12.
- `update` decrements every cooldown entry with `saturating_sub(1)` once per tick
  (put it at the top of `update`). Cooldown is only set on acceptance.
- `force_teleport` never checks doors/cooldown and sends nothing; it is
  `pub` only for join/respawn/test setup.
- `request_room_change` order: no id → return; pending → return; retry > 0 →
  decrement and return; build a rect from predicted `player.pos` and test it
  against the door's **raw geometric** `trigger` via `door_at`; `None` →
  `armed = true`; `Some && !armed` → return; `Some && armed` → send one
  reliable-ordered `RequestRoomChange` and set `pending`. Because the client
  predicts ~one tick ahead, the normal walk-in request arrives while the server's
  authoritative rect is still a few px short of the geometric band; the server's
  `server_trigger` lead tolerance absorbs that and accepts the first request.
  Lead beyond the tolerance can still reject → snap back → retry after
  `ROOM_CHANGE_RETRY_TICKS` (acceptable; lag compensation is deferred).
- `apply_room_change_local` sets room + top-left pos + zero velocity, preserves
  `dir`, snaps `prev_player_*`, clears `input_buffer`, clears
  `pending_room_change`, sets `room_change_armed = false`, resets
  `room_change_retry = 0`, then calls `load_current_level`. It is idempotent.
- Rejection handling in `handle_packet`: if the reported room differs, call
  `apply_room_change_local` **first**, then set `pending = None`,
  `retry = ROOM_CHANGE_RETRY_TICKS`, snap pos/vel/prev, clear the input buffer.
- Own `PlayerPos`: only adopt the room when
  `game::is_newer_seq(last_seq, world.last_acked_seq)`; then reconcile without
  any `coords_match` gate (keep the bounds guard and `not_in_wall`).

## Test cases (tester writes these first)

Write them in one RED batch, crate by crate in dependency order:
`game` → `protocol` → `server` → `client` → `e2e`. Compile-error RED is expected
and accepted. Each group below turns GREEN in the listed box.

### Box 1 — `game/src/room_graph.rs` new `#[cfg(test)] mod tests` (12)

Test helper inside the module:

```rust
use crate::level::Level;
use crate::maps::Map;
use crate::types::TileType;

fn level_for(map: &Map, room: (u8, u8)) -> Level {
    Level::new(
        map.grids[room.1 as usize][room.0 as usize].clone(),
        map.data[room.1 as usize][room.0 as usize].clone(),
    )
}
```

`(0,0)=LVL_0_0`, `(1,0)=LVL_0_1`, `(0,1)=LVL_1_0`, `(1,1)=LVL_1_1` (the
`maps.rs` comments are wrong; the matrix is authoritative).

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| G1 | `room_graph_has_eight_unique_doors` | `assert_eq!(graph.doors().len(), 8)`; for every `i < j`, `(doors[i].from, doors[i].direction) != (doors[j].from, doors[j].direction)` (no `Hash` derive — compare pairs). |
| G2 | `room_graph_door_matches_table` | For each of the eight rows assert `graph.door(from, direction) == Some(Door { from, direction, target, trigger: Rect::new(...), entry })` with the exact table values. Spot-check `(0,0) East`: target `(1,0)`, trigger `Rect::new(760.0, 320.0, 40.0, 40.0)`, entry `(10.0, 321.0)`. |
| G3 | `room_graph_door_none_for_missing_direction` | `door((0,0), North) == None`, `door((0,0), West) == None`, `door((1,0), North) == None`, `door((0,1), West) == None`, `door((1,1), East) == None`, `door((1,1), South) == None`. |
| G4 | `room_graph_every_door_has_reverse` | For every `d`, `let back = graph.door(d.target, opposite(d.direction)).expect("reverse door")`; `assert_eq!(back.target, d.from)`; `assert_eq!(back.direction, opposite(d.direction))`. `opposite` = North↔South, East↔West. |
| G5 | `room_graph_can_enter_true_at_each_trigger` | For every `d`, `Rect::new(d.trigger.x + 4.0, d.trigger.y + 1.0, 30.0, 38.0)` → `assert!(graph.can_enter(d.from, d.direction, rect))`. Plus the spec's literal: `can_enter((0,0), East, Rect::new(766.0, 320.0, 30.0, 38.0))`. |
| G6 | `room_graph_can_enter_false_away_from_trigger` | For every `d`: `assert!(!graph.can_enter(d.from, d.direction, Rect::new(100.0, 200.0, 30.0, 38.0)))`. |
| G7 | `room_graph_door_at_returns_direction` | For every `d`: `assert_eq!(graph.door_at(d.from, inset_rect).map(|x| x.direction), Some(d.direction))`. For each room `(0,0)/(1,0)/(0,1)/(1,1)`: `assert_eq!(graph.door_at(room, Rect::new(400.0, 240.0, 30.0, 38.0)), None)`. |
| G8 | `room_graph_trigger_center_tile_is_empty` | For every `d`: `cx = d.trigger.x + d.trigger.w / 2.0`, `cy = d.trigger.y + d.trigger.h / 2.0`; `assert_eq!(level_for(&Map::new(), d.from).grid[(cy / 40.0) as usize][(cx / 40.0) as usize], TileType::Empty)`. Do **not** use `solid_in_rect(trigger)` (the trigger's inner edge shares the floor row). |
| G9 | `room_graph_entries_are_in_bounds_and_not_solid` | For every `d`, on `level_for(&Map::new(), d.target)`: `entry.0 >= 0.0`, `entry.1 >= 0.0`, `entry.0 + 30.0 <= level.pixel_width()`, `entry.1 + 38.0 <= level.pixel_height()`, and `!level.solid_in_rect(Rect::new(entry.0, entry.1, 30.0, 38.0))`. |
| G10 | `door_server_trigger_expands_approach_edge` | Assert the expanded rect for **all eight** doors: `(0,0) East → Rect::new(752.0, 320.0, 48.0, 40.0)`, `(1,0) West → (0.0, 320.0, 48.0, 40.0)`, `(0,0) South → (280.0, 432.0, 80.0, 48.0)`, `(0,1) North → (280.0, 0.0, 80.0, 48.0)`, `(1,0) South → (480.0, 432.0, 80.0, 48.0)`, `(1,1) North → (480.0, 0.0, 80.0, 48.0)`, `(0,1) East → (752.0, 400.0, 48.0, 40.0)`, `(1,1) West → (0.0, 400.0, 48.0, 40.0)`; also assert every non-approach edge is unchanged (height/width on the other axis, same `y`/`x` origin). |
| G11 | `can_enter_accepts_within_lead_tolerance` | East `(0,0)`: `Rect::new(730.0, 320.0, 30.0, 38.0)` → `can_enter` true (right edge 760, expanded left 752); `Rect::new(700.0, 320.0, 30.0, 38.0)` → false. One boundary per direction: West `(1,0)` `(42.0, 320.0, 30.0, 38.0)` true / `(50.0, 320.0, 30.0, 38.0)` false; South `(0,0)` `(305.0, 400.0, 30.0, 38.0)` true / `(305.0, 390.0, 30.0, 38.0)` false; North `(0,1)` `(305.0, 44.0, 30.0, 38.0)` true / `(305.0, 50.0, 30.0, 38.0)` false. |
| G12 | `door_at_stays_geometric_at_lead_boundary` | `door_at((0,0), Rect::new(730.0, 320.0, 30.0, 38.0)) == None` (raw trigger starts at 760); likewise West `(1,0)` at `(42.0, 320.0, 30.0, 38.0)`, South `(0,0)` at `(305.0, 400.0, 30.0, 38.0)`, North `(0,1)` at `(305.0, 44.0, 30.0, 38.0)` — all `None`. The client must never use `server_trigger` for detection. |

### Box 2 — `protocol/tests/round_trip.rs` (5)

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| P1 | `round_trip_player_input` (updated) | Every existing `GamePacket::PlayerInput` literal drops `level_x`/`level_y`; keep ids `7`/`0`/`255`, mixed flags, `seq: 0`. |
| P2 | `round_trip_player_input_seq` + `round_trip_player_input_seq_wraparound` (updated) | Drop the two fields; keep `seq: 42`, `seq: u16::MAX`, and the destructuring assertion. |
| P3 | `round_trip_request_room_change` (**new**) | For `dir in [North, South, East, West]` and `id in [0, 255]`: `round_trip(GamePacket::RequestRoomChange { id, direction: dir })`; the helper asserts `decoded == pkt`. |
| P4 | `round_trip_room_change_response` (**new**) | `{ accepted: true, level_x: 1, level_y: 0, x: 10.0, y: 321.0 }`; `{ accepted: false, level_x: 0, level_y: 0, x: 760.0, y: 321.0 }`; `{ accepted: true, level_x: 255, level_y: 255, x: -1.5, y: 432.25 }`. |
| P5 | `protocol_depends_on_game_not_glam` (**rewrites** `test_no_game_or_glam_deps`) | Run `cargo tree -p protocol --depth 1 --edges normal` (same `Command` pattern as today). Assert success, `stdout.contains("game v0.1.0")`, and `!stdout.lines().any(|line| line.contains("glam v"))` — direct deps only (`--depth 1` is load-bearing because `glam` is transitive through `game`). `test_no_game_or_glam_deps` must not exist. |

### Box 3 — `server/src/world.rs`, new `#[cfg(test)] mod room_change_tests` (13)

Helpers inside the module (private fields/items are reachable from a child module):

```rust
use super::ServerWorld;
use game::config::ROOM_CHANGE_COOLDOWN_TICKS;
use game::entities::bullet::Bullet;
use game::entities::player::PlayerInput;
use game::room_graph::Direction;
use glam::Vec2;

fn place(world: &mut ServerWorld, id: u8, x: f32, y: f32, dir: f32)
// spawn(id); pos = (x, y); w = 30.0; h = 38.0; vel = Vec2::ZERO; dir; grounded = true;
// coords.insert(id, (0, 0))
fn tick() -> f32 { 1.0 / 60.0 }
```

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| S1 | `apply_room_change_accepts_at_door` | `place(1, 766.0, 320.0, 1.0)`; `let info = world.apply_room_change(1, Direction::East).unwrap()` (tests may `unwrap`); `info.accepted`, `info.room == (1, 0)`, `info.pos == (10.0, 321.0)`; `world.coords()[&1] == (1, 0)`; `world.players()[&1].pos == Vec2::new(10.0, 321.0)`. |
| S2 | `apply_room_change_accepts_within_lead_tolerance` | `place(1, 730.0, 320.0, 1.0)` (one prediction tick short of the geometric East band, inside `server_trigger`); `East` → accepted, `room == (1,0)`, `pos == (10.0, 321.0)`, `coords == (1,0)`. |
| S3 | `apply_room_change_rejects_beyond_lead_tolerance` | `place(1, 700.0, 320.0, 1.0)`; `East` → `!accepted`, `room == (0,0)`, `pos == (700.0, 320.0)`, coords unchanged. |
| S4 | `apply_room_change_rejects_no_door` | `place(1, 100.0, 100.0, 1.0)`; `North` then `West` → `!info.accepted`, `info.room == (0, 0)`, `info.pos == (100.0, 100.0)`; coords and player pos unchanged after both. |
| S5 | `apply_room_change_rejects_off_trigger` | `place(1, 100.0, 200.0, 1.0)`; `East` → rejected; coords `(0,0)`, pos `(100,200)` unchanged. |
| S6 | `apply_room_change_rejects_during_cooldown` | `place(1, 766.0, 320.0, 1.0)`; first `East` accepted; immediate `West` → `!accepted`, room `(1,0)`, pos `(10.0, 321.0)`; run `world.update(tick())` `ROOM_CHANGE_COOLDOWN_TICKS` times; the same `West` is now accepted, `room == (0,0)`, `pos == (760.0, 321.0)`, coords `(0,0)`. (Only the cooldown blocks the reverse; the entry overlaps the West band.) |
| S7 | `apply_room_change_teleports_to_entry` | `place(1, 766.0, 320.0, -1.0)`; push one bullet: `world.players.get_mut(&1).unwrap().bullets.push(Bullet::new(Vec2::new(1.0, 1.0), 1.0, 1))`; `world.apply_input(1, PlayerInput { move_right: true, jump: true, ..Default::default() }, 9)`; `world.shoot_cooldown.insert(1, 3)`; accept `East`; pos exactly `(10.0, 321.0)`, `vel == Vec2::ZERO`, `dir == -1.0`, `grounded == true` (recomputed from the target level at `(10,321)`, where row 9 under the entry is wall), `bullets.len() == 1` (preserved), `pending[&1].move_right && pending[&1].jump`, `shoot_cooldown[&1] == 3`, `max_seq()[&1] == 9`. |
| S8 | `apply_input_never_changes_coords` | `place(1, 100.0, 100.0, 1.0)`; `force_teleport(1, (1,0), (100.0, 100.0))`; `apply_input(1, PlayerInput { move_right: true, ..Default::default() }, 1)` → coords still `(1,0)`. `world.respawn(1)` → coords `(0,0)`; `apply_input(1, …, 2)` → coords still `(0,0)` (the spec-2 respawn-overwrite bug). |
| S9 | `respawn_ignores_door_validation` | `place(1, 100.0, 100.0, 1.0)`; `force_teleport(1, (1,1), (10.0, 401.0))`; `world.respawn(1)` succeeds → coords `(0,0)`, pos `(100.0, 100.0)`. No request, no `can_enter`. |
| S10 | `force_teleport_bypasses_doors` | `place(1, 100.0, 100.0, 1.0)`; set a **non-zero** velocity first (`world.players.get_mut(&1).unwrap().vel = Vec2::new(50.0, -20.0)`); `force_teleport(1, (1,1), (10.0, 401.0))` → coords `(1,1)`, pos `(10.0, 401.0)`, `vel == Vec2::ZERO` (zeroing is actually exercised), `grounded == true` (recomputed in `(1,1)` at `(10,401)`: row 11 under the rect is wall), no packet involved. |
| S11 | `raycast_victim_cannot_hide_by_room_claim` | `place(1, 100.0, 200.0, 1.0)`, `place(2, 400.0, 200.0, 1.0)` (both `(0,0)`); `apply_room_change(2, East)` and `apply_room_change(2, South)` → both rejected; `world.coords()[&2] == (0,0)`; `assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)))`. |
| S12 | `apply_room_change_returns_none_for_missing_player` | Fresh world: `assert_eq!(world.apply_room_change(9, Direction::East), None)`; `world.coords().get(&9) == None`. (*Derived from the spec's step 1; not in the AC list.*) |
| S13 | `force_teleport_out_of_range_room_falls_back_to_origin` (**replaces** `apply_input_out_of_range_coords_fall_back_to_origin`) | `spawn(0)`; `force_teleport(0, (255,255), (100.0, 100.0))`; `apply_input(0, PlayerInput { move_right: true, ..Default::default() }, 0)`; `world.update(tick())` does not panic; `players()[&0].pos.x > 100.0`; `coords()[&0] == (255, 255)`. Keeps the `levels.get(...).unwrap_or(&levels[0][0])` fallback covered. |

### Box 4 — `server/src/main.rs`

No new tests; the logging helpers are compile-checked. `cargo check -p metroidvania-server`
(lib **and** bin) must be green, plus `cargo test -p metroidvania-server`.

### Box 5 — client tests, part 1 (`client/src/world/lib.rs` new `mod tests`; `client/src/network/world.rs`)

Add a local recording double to `world/lib.rs` (same shape as the one already in
`network/world.rs`):

```rust
#[derive(Clone, Default)]
struct Sends(Rc<RefCell<Vec<GamePacket>>>);
struct RecordingPacketIO { sent: Sends }
// try_recv -> None; send -> push into the shared Vec
```

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| C1 | `room_change_request_sent_when_predicted_pos_at_door` | `id = Some(1)`, `current_coords = (0,0)`, `player.pos = (766,320)`, `pending_room_change = None`, `room_change_armed = true`; `request_room_change(&mut world)` → exactly one send: `GamePacket::RequestRoomChange { id: 1, direction: East }`; `pending_room_change == Some(East)`. |
| C2 | `room_change_not_sent_away_from_door` | Same setup, `player.pos = (300,200)`: `sent` empty, pending `None`. |
| C3 | `room_change_not_sent_when_disarmed` | Door setup with `room_change_armed = false`: `sent` empty, pending `None`. |
| C4 | `room_change_rearms_after_leaving_band` | `current_coords = (1,0)`, pos `(10,321)`, armed `false`; call → no send, still disarmed. pos `(100,320)`, call → no send, `armed == true`. pos `(5,320)`, call → one `West` send, pending `Some(West)`. |
| C5 | `apply_room_change_local_sets_room_entry_and_resets_state` | Dirty world: pos `(500,300)`, `dir = -1.0`, `pending = Some(East)`, `retry = 4`, one buffered `InputEntry`, one `Bullet`; `apply_room_change_local(&mut world, (1,0), Vec2::new(10.0, 321.0))` → `current_coords == (1,0)`, pos `(10,321)`, vel `ZERO`, `dir == -1.0`, `prev_player_pos == pos`, `prev_player_dir == -1.0`, `input_buffer.is_empty()`, `bullets.is_empty()`, pending `None`, `room_change_retry == 0`, `room_change_armed == false`. |
| C6 | `respawn_local_resets_room_request_state` | `current_coords = (1,0)`, pos `(500,300)`, pending `Some(East)`, retry `5`, armed `false`; `respawn_local(&mut world)` → coords `(0,0)`, pos `(100,100)`, pending `None`, retry `0`, armed `true`. |
| C7 | `send_input_omits_room_fields` (in `client/src/network/world.rs`) | `id = Some(3)`, `move_right = true`, `pressed_jump = true`; `send_input(&mut world)` → one packet that destructures as `GamePacket::PlayerInput { id: 3, move_left: false, move_right: true, jump: true, shoot: false, seq }` and `seq == world.input_seq` after the call; `input_buffer` holds one entry. (The absence of room fields is a compile-time assertion.) |

#### Box 5 — logging all-variants tests (tester, same crate as C1–C7)

The four existing all-variants tests are part of the RED batch too (they exercise
`client/src/network/logging.rs`, which Box 5 changes). Exact edits:

| # | Test | Exact edit |
| --- | --- | --- |
| L1 | `packet_type_name_all_variants` | Add `packet_type_name(&GamePacket::RequestRoomChange { id: 1, direction: Direction::East }) == "RequestRoomChange"` and `packet_type_name(&GamePacket::RoomChangeResponse { accepted: false, level_x: 0, level_y: 0, x: 0.0, y: 0.0 }) == "RoomChangeResponse"`; drop `level_x`/`level_y` from the `PlayerInput` fixture. |
| L2 | `packet_id_all_variants` | Add `packet_id(&GamePacket::RequestRoomChange { id: 6, direction: Direction::South }) == Some(6)` and `packet_id(&GamePacket::RoomChangeResponse { accepted: false, level_x: 0, level_y: 0, x: 0.0, y: 0.0 }) == None`; drop the two `PlayerInput` fields from its fixture. |
| L3 | `packet_seq_all_variants` | Add `packet_seq(&GamePacket::RequestRoomChange { .. }) == None` and `packet_seq(&GamePacket::RoomChangeResponse { .. }) == None`; drop the two `PlayerInput` fields from its fixture. |
| L4 | `packet_key_fields_all_variants` | Call `packet_key_fields` for both new variants (must not panic); drop the two `PlayerInput` fields from its fixture. Only `packet_type_name` and `packet_key_fields` are exhaustive matches; `packet_id`/`packet_seq` have `_`, but `packet_id` still needs the explicit request arm for L2. |

### Box 6 — client tests, part 2 (`client/src/network/world.rs`, existing `mod tests`)

Reuse the existing `RecordingPacketIO` / `make_world()` helpers.

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| C8 | `room_change_response_accepted_applies_room_and_entry` | Pos `(766,320)`, dir `-1.0`, `pending = Some(East)`, armed `true`, one buffered input, one own bullet; `handle_packet(&mut world, GamePacket::RoomChangeResponse { accepted: true, level_x: 1, level_y: 0, x: 10.0, y: 321.0 })` → coords `(1,0)`, pos `(10,321)`, vel `ZERO`, dir `-1.0`, `prev_player_pos == pos`, pending `None`, armed `false`, `input_buffer.is_empty()`, `bullets.is_empty()`. Apply the same packet a second time → same state (idempotent). |
| C9 | `room_change_response_rejected_snaps_back` | Pos `(500,300)` in `(0,0)`, pending `Some(East)`, armed `true`, one buffered input; handle `{ accepted: false, level_x: 0, level_y: 0, x: 760.0, y: 321.0 }` → pos `(760,321)`, vel `ZERO`, `prev_player_pos == pos`, pending `None`, `room_change_retry == ROOM_CHANGE_RETRY_TICKS`, coords `(0,0)`, `input_buffer.is_empty()`. |
| C10 | `own_player_pos_with_new_room_adopts_it` | `id = Some(1)`, `player.id = 1`, `current_coords = (0,0)`, `last_acked_seq = 10`; handle `GamePacket::PlayerPos { id: 1, x: 10.0, y: 321.0, dir: 1.0, level_x: 1, level_y: 0, last_seq: 11, server_tick: 5, server_time_ms: 1000 }` → coords `(1,0)`, pos `(10,321)`, `prev_player_pos == pos`, `input_buffer.is_empty()`, `last_acked_seq == 11`, no panic in the reconcile path. |
| C11 | `stale_own_player_pos_does_not_undo_room_change` | First apply an accepted response to `(1,0)` at `(10,321)`; set `last_acked_seq = 20`; handle own `PlayerPos` with the old room `(0,0)`, pos `(766,320)`, `last_seq: 5` → coords stay `(1,0)`, pos stays `(10,321)`. |

### Box 7 — `e2e`

New file `e2e/tests/authoritative_room_membership.rs`:

```rust
use e2e::harness::{ClientHandle, TestHarness};
use game::room_graph::Direction;
use metroidvania_server::PlayerHitInfo;
use protocol::GamePacket;

fn place(h: &mut TestHarness, p: ClientHandle, room: (u8, u8), pos: (f32, f32)) {
    h.set_server_player_coords(p, room.0, room.1);
    h.set_server_player_pos(p, pos.0, pos.1);
    h.set_client_player_pos(p, pos.0, pos.1);
    h.set_server_player_grounded(p, true);
}
// copy setup_pvp from e2e/tests/authoritative_death.rs (both in (0,0), row 5,
// shooter (100,201), victim (400,201), settle 3 ticks)
```

| # | Test | Setup and exact assertion |
| --- | --- | --- |
| E1 | `e2e_valid_door_transition_changes_room_once` | One client, `tick_n(1)`; `place(h, p0, (0,0), (766,320))`; `hold_right(p0, 1)` (**one frame**, not ten — see deviation 12); poll ≤10 ticks for `client_player_coords(p0) == (1,0)` (assert it happened); `release_all`; `tick_n(2)` → client coords still `(1,0)`, `server_player_coords(p0) == (1,0)`, client pos `(x, y)` with `|x - 10.0| <= 2.0` and `|y - 321.0| <= 2.0`, and coords still `(1,0)` after the extra ticks (no bounce). |
| E2 | `e2e_non_door_teleport_is_rejected` | One client; `place(h, p0, (0,0), (300,200))` stages **both** client and server at `(300,200)`. For each of `North`, `West`, `East`: `h.client_mut(p0).world.network.send(GamePacket::RequestRoomChange { id, direction })`; `h.tick()`; `assert_eq!(h.server_player_coords(p0), (0,0))`, `assert_eq!(h.client_player_coords(p0), (0,0))`, client pos within 2 px of the server pos. `assert_eq!(h.client_room_change_requests, 3)` and `assert_eq!(h.room_change_responses(p0), 3)`. |
| E3 | `e2e_spoofed_room_cannot_dodge_death` | `setup_pvp` (both staged in `(0,0)` row 5); victim sends `RequestRoomChange { North }` and then `{ East }` (not at a door); `h.tick()`; both coords stay `(0,0)`; per-requester routing: `assert_eq!(h.room_change_responses(victim), 2)` and `assert_eq!(h.room_change_responses(shooter), 0)` (a broadcast would answer both); `assert_eq!(h.client_room_change_rejects, 2)`. Then `press_shoot(shooter)`; tick ≤5 until `!h.last_deaths.is_empty()`, breaking on that tick; `assert_eq!(h.last_deaths, vec![PlayerHitInfo { shooter_id, victim_id }])`; victim client pos within 2 px of `(100,100)` and `client_player_coords(victim) == (0,0)`. The victim is never unkillable. |
| E4 | `e2e_respawn_room_survives_later_inputs` | `setup_pvp`; kill once (same loop as E3); `hold_right(victim, 10)`; `tick_n(10)` → `server_player_coords(victim) == (0,0)`, `client_player_coords(victim) == (0,0)`. |
| E5 | `e2e_rejection_snaps_client_back` | One client; `place(h, p0, (0,0), (500,300))` stages both; then move **only the server** to `(760,321)` with `h.set_server_player_pos(p0, 760.0, 321.0)` (and leave the client at `(500,300)`); `inject_server_packet(p0, GamePacket::RoomChangeResponse { accepted: false, level_x: 0, level_y: 0, x: 760.0, y: 321.0 })`; `h.tick()` → client pos within 2 px of `(760,321)`, coords `(0,0)`. |
| E6 | `e2e_walk_in_first_request_accepted` | One client; `place(h, p0, (0,0), (700,320))` stages **both** sides at `(700,320)` on row 8; `hold_right(p0, 25)`; poll ≤25 ticks for `client_player_coords(p0) == (1,0)` (assert it happened); `release_all`; `assert_eq!(h.client_room_change_rejects, 0)` (the first request was accepted — the deterministic ~1-tick lead the `server_trigger` band must absorb); then `tick_n(3)` asserting after every tick that the client coords are never `(0,0)` (no reject/snap-back loop), and finally `client_player_coords(p0) == (1,0)` and `server_player_coords(p0) == (1,0)`. |

#### Box 7 — existing-suite test rewrites (tester, RED batch)

`rooms.rs`/`f1` are **test** edits, so the tester writes them in the RED batch.
They turn GREEN with Box 7's harness changes (the fixtures cannot pass before
the harness handles `RequestRoomChange`).

`e2e/tests/rooms.rs` — exact edits:

| # | Test | Exact edit |
| --- | --- | --- |
| B1 | `b1_transition_right_changes_coords` | Comment only: replace the `check_level_transitions` / 795 px explanation with the request/ack flow (client predicted at the raw band → `RequestRoomChange` → server entry `(10,321)`). Fixtures and asserts unchanged. |
| B2 | `b2_transition_left_changes_coords` | Add `h.set_client_player_pos(p0, 790.0, 320.0);` next to the server stage. After the right transition (and the `(1,0)` assert): stage **both** sides at `(100.0, 320.0)` (`set_server_player_pos` + `set_client_player_pos`) and `h.tick_n(12)`; then stage **both** at `(5.0, 320.0)`, `h.hold_left(p0, 10)`, `h.tick_n(10)`; assert `(0,0)`. |
| B3 | `b3_transition_down_changes_coords` | Add `h.set_client_player_pos(p0, 300.0, 470.0);` next to the server stage. |
| B4 | `b4_transition_up_changes_coords` | Add `h.set_client_player_pos(p0, 300.0, 470.0);` at the down stage. After the `row == 1` assert: stage both at `(300.0, 100.0)` and `h.tick_n(12)`; then `h.set_server_player_pos(p0, 300.0, 10.0); h.set_client_player_pos(p0, 300.0, 10.0);`, `set_server_player_grounded(p0, true)`, `press_jump(p0)`, `tick_n(5)`; assert row 0. |
| B6 | `b6_not_stuck_after_left_transition` | After the right transition: stage both at `(100.0, 320.0)` and `h.tick_n(12)`; then both at `(5.0, 320.0)`; `hold_left`/`tick_n(10)`; assert `(0,0)`. |
| B8 | `b8_not_stuck_after_up_transition` | After the down transition: stage both at `(300.0, 100.0)` and `h.tick_n(12)` before staging both at `(300.0, 10.0)`, grounded, jump, `tick_n(5)`. |
| B5/B7/B9 | unchanged outcomes | b5/b7/b9 keep their asserts. b7 already stages both sides; b5 and b9 need no fixture change. |

Lead tolerance vs rooms.rs: every b-test transition is staged with **both** sides
inside the raw geometric band, so `can_enter` passes on the raw band and the 8 px
expansion changes nothing there. The expansion only matters for the E6 walk-in
(client leads the server by ~6.7 px at request time). Do not weaken the b-tests
to rely on the tolerance.

`e2e/tests/edge_cases.rs`:

| # | Test | Change |
| --- | --- | --- |
| F1 | `f1_bogus_client_room_is_ignored` (**renames** `f1_bogus_level_coords_fall_back_to_origin`) | Rewrite body (the old name describes deleted behavior); see below. |

```rust
// f1_bogus_client_room_is_ignored
h.client_mut(p0).world.current_coords = (99, 99);
h.hold_right(p0, 5);
h.tick_n(5);
assert!(h.server_player_pos(p0).0 > 100.0);       // movement still processed
assert_eq!(h.server_player_coords(p0), (0, 0));   // the client cannot claim a room
assert_eq!(h.client_player_coords(p0), (0, 0));   // own PlayerPos re-adopts the authority
```

### RED → GREEN handoff

| Box | Crate | Tests that turn GREEN | Narrow command |
| --- | --- | --- | --- |
| 1 | `game` | G1–G12 | `cargo test -p game` |
| 2 | `protocol` | P1–P5 | `cargo test -p protocol` |
| 3 | `server` (lib) | S1–S13 | `cargo test -p metroidvania-server --lib` |
| 4 | `server` (bin) | — | `cargo check -p metroidvania-server` |
| 5 | `client` | C1–C7 + L1–L4 (C8–C11 stay RED) | `cargo test -p metroidvania-rust` |
| 6 | `client` | C8–C11 | `cargo test -p metroidvania-rust` |
| 7 | `e2e` | E1–E6 + F1 + B1–B9 + all existing suites | `cargo test -p e2e` |

**Expected RED window:** the workspace does not compile from the **tester's RED
batch** through the end of Box 6 (the RED tests reference the new game API,
protocol variants, `World` fields, and harness fields). Protocol's field removal
also breaks `server/src/main.rs`, `client/src/{network/{transport,logging,world},world/lib}.rs`
and `e2e/src/harness.rs`; Box 3's `apply_input` signature change keeps the server
binary red until Box 4; e2e stays red until Box 7. Only the per-crate command in
the table is expected to pass at each step. Box 5's caveat: the crate compiles,
C1–C7 + L1–L4 pass, and C8–C11 fail (their `handle_packet` arm does not exist
until Box 6).

## Ripple surfaces (compile-error-driven)

| Surface | Sites | Why it breaks |
| --- | --- | --- |
| `server/src/world.rs` `apply_input` | **21** call sites across the file's test code: `mod tests` (`input_held_flags_latest_wins` ×2, `input_edge_flags_latch_and_clear` ×2, `input_edge_flag_not_lost_between_ticks`, `apply_input_update_drives_authoritative_position`, `apply_input_update_jump_moves_up_when_grounded`, `apply_input_out_of_range_coords_fall_back_to_origin`, the four `max_seq_*`/`playerpos_*` tests), the two crate-level `#[test]` fns (`shoot_emits_bulletspawninfo`, `shoot_cooldown_enforced` ×2), and `mod raycast_and_hit_tests` (6 calls in the four `update_*` tests) | two parameters removed |
| `server/src/world.rs` test layout | **one** `#[cfg(test)] mod tests` + **two crate-level** `#[test]` fns + `mod raycast_and_hit_tests` (the spec's "two separate `#[cfg(test)]` blocks" wording is inaccurate — sweep all three regions) | wrong mental model risks missing call sites |
| `server/src/world.rs` | `apply_input_out_of_range_coords_fall_back_to_origin` | its premise (client-claimed `(255,255)`) disappears → replace with S13 |
| `server/src/main.rs` | `packet_type_name` (exhaustive, +2 arms), `packet_id` (+1), `packet_key_fields` (exhaustive, +2 arms, drop `"level"` from `PlayerInput`), inbound `PlayerInput` arm, `apply_input` call, new `RequestRoomChange`/`RoomChangeResponse` arms | variant + field changes |
| `client/src/network/world.rs` | `send_input` literal; `JoinResponse` reset of the three new fields; own-`PlayerPos` branch (`coords_match`, `level_x/level_y` still used by `PlayerPos` — keep) | `PlayerInput` fields + new room-request state |
| `client/src/network/logging.rs` | `packet_type_name` and `packet_key_fields` are **exhaustive** (+2 arms each); `packet_id` needs the explicit `RequestRoomChange` arm (`RoomChangeResponse` falls to `_`); `packet_seq` falls to `_`; 4 `PlayerInput` test fixtures | variant/field changes |
| `client/src/network/transport.rs` | both `PacketIO::send` match blocks | `RequestRoomChange` must be `reliable_ordered` (same arm as `JoinRequest`) |
| `client/src/world/game.rs` | `use ...::{check_level_transitions, ...}`, the `check_level_transitions(self)` call, `World::new`, the stale transition comment | function removed / new fields |
| `client/src/lib.rs` | re-export | function removed |
| `client/src/network/prediction.rs` | AC-38 doc comment (lines ~125, ~608) and `replay_uses_client_room_not_server_coords` doc | no longer "server lags across a transition"; the client adopts the room first |
| `game/src/room_graph.rs` (new) | `Door::server_trigger` + `DOOR_LEAD_TOLERANCE_PX`; `can_enter` uses it, `door_at` must not | new surface (no break — new module) |
| `e2e/src/harness.rs` | `use ...::check_level_transitions`, Step 2 `PlayerInput` literal, Step 3 `PlayerInput` arm + `apply_input` call (+ `RequestRoomChange`/response routing), Step 7c, `set_server_player_coords`, new `client_room_change_requests` / `client_room_change_rejects` / `room_change_responses` | protocol + API changes |
| `e2e/tests/rooms.rs` | B1 comment; B2/B3/B4/B6/B8 fixture edits (both-sides staging, re-arm + cooldown) | predicted-position gating + anti-bounce |
| `e2e/tests/edge_cases.rs` | `f1_bogus_level_coords_fall_back_to_origin` → **renamed** `f1_bogus_client_room_is_ignored` + rewritten | asserts the removed behavior |
| `e2e/tests/authoritative_room_membership.rs` (new) | E1–E6 (E6 is the `server_trigger` walk-in) | new suite |
| `protocol/tests/round_trip.rs` | `test_no_game_or_glam_deps` → `protocol_depends_on_game_not_glam` | protocol now depends on `game` |
| `e2e/tests/reconciliation.rs`, `e2e/tests/multiplayer.rs` | transition fixtures | should still pass (both sides staged); only stale comments; no required edit |
| `e2e/tests/edge_cases.rs::f5_disconnect_during_room_transition` | comment only | no transition is actually asserted; it still passes |

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Box 1 — `game`: `RoomGraph`, `Direction`, `Door`, `RoomCoords`, constants

- [x] done
- **Files**: `game/Cargo.toml`; `game/src/lib.rs`; `game/src/room_graph.rs` (new);
  `game/src/config.rs`.
- **Change**:
  - `game/Cargo.toml`: add `serde = { version = "1.0.228", features = ["derive"] }`
    under `[dependencies]`.
  - `game/src/lib.rs`: `pub mod room_graph;` (alphabetical position).
  - `game/src/room_graph.rs`: implement the public API verbatim from the spec.
    Build the eight doors from the table (in table order, so `door_at` ties break
    deterministically). `door` scans for the unique `(from, direction)` match;
    `door_at` uses `from` + the raw `trigger.overlaps(&player_rect)`;
    `Door::server_trigger` grows the approach edge by `DOOR_LEAD_TOLERANCE_PX`;
    `can_enter` is
    `door(from, direction).map(|d| d.server_trigger().overlaps(&player_rect)).unwrap_or(false)`.
    The client never uses `server_trigger`.
  - `game/src/config.rs`: append the three constants (`ROOM_CHANGE_COOLDOWN_TICKS`,
    `ROOM_CHANGE_RETRY_TICKS`, `DOOR_LEAD_TOLERANCE_PX = 8.0`) with the spec's
    doc comments.
  - Add the G1–G12 tests.
  - Do **not** touch `maps.rs` (the comments are known-wrong; the table is data).
- **Acceptance**: **only `cargo test -p game` is the gate** (G1–G12 + all existing
  game tests). Do **not** try to fix the other crates in this box; the workspace
  is expected to be RED from the tester's batch.
- **Test**: G1–G12.
- **Explain**
  - What: added `game/src/room_graph.rs` with `RoomCoords = (u8, u8)`,
    `Direction` (serde-derived), `Door` (+`server_trigger`), and `RoomGraph`
    (the eight-door table, `doors`, `door`, `door_at`, `can_enter`, `Default`);
    added `ROOM_CHANGE_COOLDOWN_TICKS`, `ROOM_CHANGE_RETRY_TICKS`, and
    `DOOR_LEAD_TOLERANCE_PX` to `game/src/config.rs`; added the `serde`
    dependency to `game/Cargo.toml`.
  - Why: both sides must derive "is the player at this door" from one shared
    source of truth instead of magic pixel heuristics. `door_at` gives the
    client its geometric detection band; `can_enter` gives the server the
    lead-expanded validation band, so a predicted-first request is accepted
    while arbitrary teleports still fail.
  - Rust concepts: a tuple alias (`RoomCoords`) documents `(u8, u8)` without a
    new type; `Copy` on the plain-data `Door` lets `door`/`door_at` return
    `Option<Door>` by value and callers treat it as a snapshot; the
    `serde::Serialize/Deserialize` derive on a `game`-local enum is what makes
    the protocol crate able to reuse `Direction` over the wire; `match` on
    `Direction` in `server_trigger` returns a new `Rect` (no mutation of the
    `&self` value); `Option::map(...).unwrap_or(false)` collapses
    "door exists AND rect overlaps" into one expression without an `unwrap`;
    `Iterator::find` + `.copied()` turns `Option<&Door>` into `Option<Door>`
    while keeping `doors` borrowed for only the scan; `Default` delegates to
    `new()` to satisfy `clippy::new_without_default`.
  - Watch out: the `maps.rs` comments are wrong — the matrix is
    `[row_y][col_x]`, so `(0,0)=LVL_0_0`, `(1,0)=LVL_0_1`,
    `(0,1)=LVL_1_0`, `(1,1)=LVL_1_1`. `door_at` must use the full player rect
    and the raw `trigger`; only `can_enter` uses `server_trigger`. The trigger's
    inner edge shares a tile row with the floor, so the tests validate the
    trigger **center** tile rather than `solid_in_rect(trigger)`. Table order is
    the deterministic `door_at` tie-breaker; do not sort the vector.

### Box 2 — `protocol`: packet surface

- [x] done
- **Files**: `protocol/Cargo.toml`; `protocol/src/lib.rs`;
  `protocol/tests/round_trip.rs`.
- **Change**:
  - `protocol/Cargo.toml`: add `game = { path = "../game" }`.
  - `protocol/src/lib.rs`: `use game::room_graph::Direction;`; remove
    `level_x`/`level_y` from `PlayerInput`; append `RequestRoomChange` then
    `RoomChangeResponse` **after** `BulletSpawn` (variant indices of the existing
    variants must not change). `PlayerPos` is untouched.
  - `protocol/tests/round_trip.rs`: P1–P5. Rewrite
    `test_no_game_or_glam_deps` → `protocol_depends_on_game_not_glam` using
    `cargo tree -p protocol --depth 1 --edges normal`; assert
    `stdout.contains("game v0.1.0")` and that **no line** contains `glam v`.
    `--depth 1` is load-bearing: `glam` is now transitive through `game`, so a
    full-tree check would always see it.
- **Acceptance**: `cargo test -p protocol` green (P1–P5 + existing round-trips);
  `grep -n 'level_x\|level_y' protocol/src/lib.rs` → only inside `PlayerPos` and
  `RoomChangeResponse`.
- **Test**: P1–P5.
- **Explain**
  - What: `protocol` now depends on `game`; `PlayerInput` lost `level_x`/
    `level_y`; `RequestRoomChange { id, direction }` and `RoomChangeResponse {
    accepted, level_x, level_y, x, y }` were appended after `BulletSpawn`;
    `PlayerPos` was left untouched.
  - Why: a client may ask to use a *direction* only. There is no packet field
    through which it can name a room, which closes the spoofed-room path in the
    type system rather than in server validation alone. The response carries
    the server's authoritative room/position for both accept and reject so the
    client has a precise apply or snap-back target.
  - Rust concepts: enum variants in `postcard` are encoded by index, so
    appending is the only index-stable change (inserting would shift
    `BulletSpawn` and silently break peers); a shared type (`Direction`) lives
    in `game` and is imported into `protocol`, giving one enum, not a
    protocol-local duplicate; the dependency edge is acyclic because `game`
    knows nothing about `protocol`; P5 uses `std::process::Command` +
    `cargo tree --depth 1 --edges normal` to assert the direct edge without
    failing on the now-transitive `glam`.
  - Watch out: append at the end; `Direction` variant order
    (`North, South, East, West`) is wire format; `PlayerPos` must keep
    `level_x`/`level_y` (nemesis placement); removing fields from a variant
    changes its payload, which is acceptable only because the whole workspace
    is rebuilt together (no wire migration).

### Box 3 — `server` world: validation, teleport, cooldown

- [x] done
- **Files**: `server/src/world.rs`; `server/src/lib.rs`.
- **Change**:
  - Imports: `use game::rect::Rect; use game::room_graph::{Direction, RoomCoords, RoomGraph};`
    and `use game::config::ROOM_CHANGE_COOLDOWN_TICKS;`.
  - `ServerWorld`: add private `room_graph: RoomGraph` and
    `room_change_cooldown: HashMap<u8, u8>`; initialise both in `new()`.
  - Add `pub struct RoomChangeInfo` (derive per the API block).
  - `apply_input` → `(id, input, seq)`; **delete** the `coords.insert` line and
    the stale comment above it.
  - `spawn`: `room_change_cooldown.insert(id, 0)`; `remove`: drop the entry.
  - `update`: at the top, `for cd in self.room_change_cooldown.values_mut() { *cd = cd.saturating_sub(1); }`.
  - Private `fn level(&self, room: RoomCoords) -> &Level` with the same `.get()`
    fallback already used in `update`/`raycast_player`.
  - `apply_room_change` exactly per the spec's six steps. Guard the guaranteed
    `door` with `let Some(door) = ... else { return Some(rejection) }` (no
    `unwrap`). Assign `player.pos.x/.y` and `player.vel.x/.y` component-wise
    (no `glam` in normal deps). Recompute `grounded` from the target level at
    `Rect::new(entry.0, entry.1, w, h)`. Preserve `dir`, `bullets`, `pending`,
    `shoot_cooldown`, `max_seq`.
  - `force_teleport`: no-op when `id` is not spawned (do not insert a phantom
    coord); otherwise set `coords`, pos, zero vel, recompute `grounded`. No door
    check, no cooldown, no packet.
  - `respawn`: unchanged except it must not call `can_enter` (it already only
    rebuilds the player + `coords (0,0)` + `pending`).
  - Update all 21 test call sites; replace
    `apply_input_out_of_range_coords_fall_back_to_origin` with S13; add the new
    `mod room_change_tests` (S1–S13, including the two lead-tolerance tests S2/S3
    and the grounded/bullet-preservation assertions in S7/S10). `can_enter`
    already carries the lead tolerance (Box 1); the server only calls it.
  - `server/src/lib.rs`: `pub use world::{BulletSpawnInfo, PlayerHitInfo, RoomChangeInfo, ServerWorld};`.
- **Acceptance**: `cargo test -p metroidvania-server --lib` green (S1–S13 + all
  pre-existing world tests). `cargo check -p metroidvania-server --lib` green.
  The **binary** still fails (it calls the old `apply_input`) — Box 4 fixes it.
- **Test**: S1–S13.
- **Explain**
  - What: `ServerWorld` now owns `room_graph` and a per-player
    `room_change_cooldown`; `apply_input` lost `x, y` and no longer writes
    `coords`; added `apply_room_change`, `force_teleport`, a private
    `level(room)` fallback helper, and `RoomChangeInfo` (exported from
    `server/src/lib.rs`); cooldowns are initialised in `spawn`, dropped in
    `remove`, and decremented at the top of `update`.
  - Why: this is the authority boundary. `coords` changes only through
    server-controlled paths, so `raycast_player` can trust both the shooter's
    and the victim's room. A request is accepted only when a door exists for
    `(from, direction)` and the authoritative body overlaps its lead-expanded
    trigger, and only outside the cooldown; acceptance teleports to the
    canonical entry, rejection returns the unchanged state for a client snap
    back. `force_teleport` exists so join/respawn/test setup never has to pass
    door validation.
  - Rust concepts: `HashMap::values_mut()` iterates all cooldown entries for the
    per-tick decrement, and `u8::saturating_sub` makes "already zero" free of
    underflow; `Option<RoomChangeInfo>` is deliberately a two-outcome reply
    (unknown player vs. accept/reject) instead of a `Result`, because a
    rejection is a normal protocol event, not an error; `let ... else` handles
    the invariant door without `unwrap`; the `&mut self` teleport is split from
    the `&self` `level`/`can_enter` reads by copying `(rect, w, h)` out first,
    then recomputing `grounded` before the mutable player borrow; `player.pos.x
    = …` / `player.vel.y = 0.0` field-wise assignment keeps `glam` a
    dev-dependency only.
  - Watch out: a rejection must mutate nothing (coords, pos, velocity);
    acceptance arms the cooldown (only acceptance); `grounded` is recomputed in
    the **target** room at the entry, or a stale airborne jump flag leaks into
    the new room; `pending`, `bullets`, `dir`, `shoot_cooldown`, and `max_seq`
    are preserved — unlike `respawn`, which resets the input; `apply_input`'s
    `max_seq` accumulation stays wrapping-aware; `force_teleport` no-ops for an
    unspawned id rather than inserting a phantom coord.

### Box 4 — `server/src/main.rs`: request handling + logging

- [x] done
- **Files**: `server/src/main.rs`.
- **Change**:
  - `packet_type_name` (exhaustive): add
    `GamePacket::RequestRoomChange { .. } => "RequestRoomChange"` and
    `GamePacket::RoomChangeResponse { .. } => "RoomChangeResponse"`.
  - `packet_id`: add `GamePacket::RequestRoomChange { id, .. } => Some(*id)`;
    leave `RoomChangeResponse` on `_ => None`.
  - `packet_key_fields` (exhaustive): add
    `RequestRoomChange { id, direction } => json!({ "id": id, "direction": direction })`
    (`Direction: Serialize`) and
    `RoomChangeResponse { accepted, level_x, level_y, x, y } => json!({ "accepted": accepted, "level": [level_x, level_y], "pos": [x, y] })`.
    Drop `"level"` from the `PlayerInput` arm.
  - Inbound `PlayerInput`: drop `level_x`/`level_y` from the pattern and call
    `world.apply_input(id, PlayerInput { .. }, seq)`.
  - New inbound `RequestRoomChange { id: packet_id, direction }` arm:
    1. `let Some(&resolved) = addrs.get(&addr) else { continue; };`
    2. `if resolved != packet_id { continue; }` (do not trust the packet id)
    3. `if let Some(info) = world.apply_room_change(resolved, direction) { ... }`
       build `GamePacket::RoomChangeResponse` from `info`, `log_packet("sent", &response)`,
       then `if let Ok(bytes) = postcard::to_allocvec(&response) { packet_sender.send(Packet::reliable_ordered(addr, bytes, None)); }`
       — **to `addr` only**, never broadcast. (`main.rs` keeps its existing
       serialization `.unwrap()` style elsewhere; the new arm uses `if let Ok` so
       the plan's "no new unwrap/expect" rule is not violated.)
  - New inbound `RoomChangeResponse { .. } => {}` arm with a comment (server→client
    only; defense in depth). Must sit before the final `_ => {}`.
- **Acceptance**: `cargo check -p metroidvania-server` green (lib + bin);
  `cargo test -p metroidvania-server` green. There is no dedicated log-format
  test for the two variants (the e2e log suite only inspects `PlayerInput`);
  do not claim coverage.
- **Test**: none new (compile + logging arms).
- **Explain**
  - What: added `RequestRoomChange`/`RoomChangeResponse` to
    `packet_type_name`, the request to `packet_id`, and both to
    `packet_key_fields` (dropping `"level"` from `PlayerInput`); the inbound
    `PlayerInput` arm and `apply_input` call lost the room fields; a new inbound
    `RequestRoomChange` arm resolves the id from `addrs`, calls
    `apply_room_change`, and sends one `RoomChangeResponse` back to the
    requesting `addr` via `reliable_ordered`; inbound `RoomChangeResponse` is
    explicitly ignored.
  - Why: the server, not the client, decides membership. The request carries
    only a direction, the transport map decides *who* is asking, and the ack is
    per-requester so no client ever learns/adopts another player's room from a
    broadcast.
  - Rust concepts: `packet_type_name`/`packet_key_fields` are exhaustive
    `match`es, so a new enum variant is a compile error until handled
    (`packet_id`/`packet_seq` already have `_` arms); `json!({ "direction":
    direction })` works because `Direction` derives `Serialize` (a unit variant
    becomes a string); `addrs.get(&addr)` yields `Option<&u8>` and
    `let Some(&resolved) = … else { continue; }` both dereferences and exits the
    enclosing `while let` drain loop; `postcard::to_allocvec` returns
    `Result<Vec<u8>, _>`, handled with `if let Ok(bytes)` rather than the
    file's existing `.unwrap()` style for the new path.
  - Watch out: the response goes to `addr` only (never `addrs.keys()`); the
    packet id must match the transport-resolved id (otherwise a client could
    request a change for someone else); `main.rs` keeps its `.unwrap()` style
    for the older sends, but the new arm avoids a new panic site; an unknown
    player (`apply_room_change` → `None`) sends nothing.

### Box 5 — client: world state, request/ack primitives, remove the local transition

- [x] done
- **Files**: `client/src/world/game.rs`; `client/src/world/lib.rs`;
  `client/src/lib.rs`; `client/src/network/world.rs` (`send_input` + the
  `JoinResponse` reset + C7 for now); `client/src/network/logging.rs`;
  `client/src/network/transport.rs`.
- **Change**:
  - `world/game.rs`: add the four `pub` fields, initialise
    (`room_graph: RoomGraph::new()`, `pending_room_change: None`,
    `room_change_armed: true`, `room_change_retry: 0`); add
    `pub fn current_room(&self) -> RoomCoords`; replace the
    `check_level_transitions` import/call with `request_room_change`; reword the
    "must run after bullets / `load_current_level`" comment (the request path no
    longer loads a level — the level loads when the ack is applied).
  - `world/lib.rs`: **delete** `check_level_transitions`; add
    `request_room_change` and `apply_room_change_local` per the semantics block;
    add `use game::rect::Rect; use game::room_graph::RoomCoords;` and
    `use crate::network::prediction::InputRingBuffer;`. Update `respawn_local`
    to set `pending = None`, `retry = 0`, `armed = true` (keep everything else).
  - `client/src/lib.rs`: re-export `request_room_change` instead of
    `check_level_transitions`.
  - `network/world.rs`: `send_input` drops `level_x`/`level_y` from the
    `PlayerInput` literal; the `JoinResponse` arm also sets
    `pending_room_change = None`, `room_change_retry = 0`,
    `room_change_armed = true` (alongside the existing session resets).
  - `network/logging.rs`: add both variants to `packet_type_name`,
    `packet_id` (`RequestRoomChange → Some(*id)`, response falls through to
    `None`), `packet_seq` (`_`), `packet_key_fields` (only `packet_type_name`
    and `packet_key_fields` are exhaustive; `packet_id` still needs the request
    arm); extend the four all-variants test bodies per L1–L4 — asserting
    `packet_id(RequestRoomChange) == Some(id)` and
    `packet_id(RoomChangeResponse) == None` — and drop `level_x/level_y` from
    every `PlayerInput` fixture (4 sites) and its key-fields arm.
  - `network/transport.rs`: in both `send` match blocks,
    `GamePacket::JoinRequest | GamePacket::RequestRoomChange { .. } => reliable_ordered`.
  - Add C1–C6 (in `world/lib.rs` `mod tests` with a local recorder) and C7 +
    L1–L4 (in `network/world.rs` / `network/logging.rs`).
- **Acceptance**: `cargo test -p metroidvania-rust` compiles and C1–C7 + L1–L4 +
  all pre-existing client tests pass; `cargo check -p metroidvania-rust --all-targets`
  green. **C8–C11 are expected to FAIL until Box 6** (the `RoomChangeResponse`
  arm does not exist; `handle_packet`'s catch-all swallows the packet). e2e is
  still red.
- **Test**: C1–C7 + L1–L4.
- **Explain**
  - What: `World` gained `room_graph`, `pending_room_change`,
    `room_change_armed`, `room_change_retry` and a `current_room()` helper;
    `check_level_transitions` is deleted and replaced by
    `request_room_change` + `apply_room_change_local` in `world/lib.rs`;
    `respawn_local` resets the request state to an armed spawn; `send_input`
    drops the room fields and `JoinResponse` resets the new state; logging
    covers both new variants; both transport `send` match blocks route
    `RequestRoomChange` to `reliable_ordered`; the re-export in `client/src/lib.rs`
    changed.
  - Why: the client no longer owns membership. It detects a door on its
    **predicted** body and sends one direction request, then waits. The server's
    ack (or its own `PlayerPos`) applies the room. This removes the local
    edge heuristic and its hardcoded offsets, and with them a whole class of
    "the client says it is in room X" desyncs.
  - Rust concepts: adding fields to a struct with one literal initializer means
    the compiler lists every missing field; `current_room(&self)` centralises
    the `usize → u8` casts at the boundary while `current_coords` keeps its
    existing `usize` matrix indexing; `Rect::new(player.pos.x, player.pos.y,
    player.w, player.h)` builds the overlap input from a `Vec2` plus size;
    `world.input_buffer = InputRingBuffer::new()` replaces the buffer wholesale
    (no partial clear API needed); calling `world.network.send(...)` goes
    through `Box<dyn PacketIO>` dynamic dispatch; moving a function between
    sibling modules requires updating the `use`/`pub use` chain; the tests use
    `Rc<RefCell<Vec<GamePacket>>>` as a shared send recorder (single-threaded,
    so `Rc`/`RefCell` rather than `Arc`/`Mutex`); `match` with a
    `Some(_) if !world.room_change_armed` guard expresses the anti-bounce rule.
  - Watch out: the request path must never mutate `current_coords`;
    `door_at` stays on the raw geometric `trigger` (never `server_trigger`);
    `apply_room_change_local` resets `room_change_retry = 0`, so a reject
    handler must set the retry **after** any defensive adoption call; the
    input buffer is cleared explicitly because `load_current_level` does not
    touch it; detection uses the predicted position, so a door request can
    arrive while the server body is a few px short (that is what the server's
    8 px lead tolerance absorbs).

### Box 6 — client network: response handler, own-`PlayerPos` adoption, drop `coords_match`

- [x] done
- **Files**: `client/src/network/world.rs`; `client/src/network/prediction.rs`.
- **Change**:
  - Own-`PlayerPos` branch: after the clock-sync block, compute
    `let stale = !game::is_newer_seq(last_seq, world.last_acked_seq)`. If not
    stale and `(level_x, level_y) != world.current_room()`, call
    `crate::world::lib::apply_room_change_local(world, (level_x, level_y), glam::Vec2::new(x, y))`
    before reconciling. **Delete** the `coords_match` variable/comment; keep the
    bounds guard (`levels_matrix.get`) and `not_in_wall`. Reconciliation then runs
    for any valid room.
  - Add the `GamePacket::RoomChangeResponse { accepted, level_x, level_y, x, y }`
    arm:
    - accepted → `apply_room_change_local(world, room, Vec2::new(x, y))`;
    - rejected → if `room != world.current_room()` adopt via
      `apply_room_change_local` first; then `pending = None`,
      `retry = ROOM_CHANGE_RETRY_TICKS`, set pos, zero vel, snap
      `prev_player_pos/prev_player_dir`, and clear `input_buffer`.
  - Add explicit ignore arms (or document the existing `_`) for inbound
    `RequestRoomChange` and `JoinRequest` on the client.
  - `network/prediction.rs`: reword the AC-38 doc comment (function + test) — the
    client adopts the authoritative room before reconciling, so the level passed
    in is the adopted room's level; it is not "the server lagging behind".
  - Add C8–C11 to the existing `mod tests`.
- **Acceptance**: `cargo test -p metroidvania-rust` green (C1–C11 + existing);
  `grep -RIn 'coords_match' client/` → nothing.
- **Test**: C8–C11.
- **Explain**
  - What: `handle_packet` gained a `RoomChangeResponse` arm (accepted →
    `apply_room_change_local`; rejected → optional defensive adoption, snap
    position/velocity/interpolation, clear the input buffer, clear pending, set
    `ROOM_CHANGE_RETRY_TICKS`); the own-`PlayerPos` branch now adopts a new
    authoritative room when the packet is not stale, and the `coords_match`
    gate is gone; explicit ignore arms were added for inbound
    `RequestRoomChange`; the prediction/prediction-test docs were reworded;
    C8–C11 were added to the existing `mod tests`.
  - Why: the reliable ack and the unreliable `PlayerPos` can arrive in either
    order. Applying either to the same authoritative state (and ignoring stale
    packets) makes the room converge without a "which stream won" hack.
    Removing `coords_match` is safe precisely because the server now teleports
    on acceptance, so a cross-room `PlayerPos` is authoritative rather than a
    lagging leftover.
  - Rust concepts: `if` as a statement inside a match arm (not a guard) keeps
    the accepted/rejected handling in one place; `game::is_newer_seq` is the
    wrapping-aware ordering gate; applying the same `Copy` payload twice is
    modelled as an idempotent state transformation; tuple comparison
    `(u8, u8) != world.current_room()` compares rooms without per-axis `&&`;
    `world.current_room()` borrows `&World` only for the comparison, so the
    later `&mut World` adoption call is accepted by the borrow checker; the
    reject branch deliberately sets `room_change_retry` after the adoption call
    because `apply_room_change_local` resets it to 0 (a real ordering
    dependency, not incidental).
  - Watch out: a stale pre-transition `PlayerPos` must not drag the client back
    — that is why adoption is gated on `!stale`; the bounds guard stays so a
    bogus `(99,99)` cannot index-panic; `not_in_wall` stays as defense in depth;
    the reject path clears the buffer **after** the snap so replay cannot undo
    it; a defensive adoption must not re-arm the retry (set retry afterwards);
    this is the only place that calls `apply_room_change_local` for
    `PlayerPos`, so `request_room_change` still never mutates the room.

### Box 7 — e2e harness + new suite + choreography

- [ ] done
- **Files**: `e2e/src/harness.rs`; `e2e/tests/authoritative_room_membership.rs`
  (new); `e2e/tests/rooms.rs`; `e2e/tests/edge_cases.rs`.
- **Change**:
  - `harness.rs`:
    - import `request_room_change` (replacing `check_level_transitions`); add
      `pub client_room_change_requests: usize`, `pub client_room_change_rejects:
      usize` (both init `0`), and a private per-client count map
      `room_change_response_counts: HashMap<ClientHandle, usize>` plus
      `pub fn room_change_responses(&self, handle: ClientHandle) -> usize`
      (insert `0` in `connect_client`, remove the entry in `disconnect_client`,
      default the getter to `0`).
    - Step 2: drop `level_x`/`level_y` from the `PlayerInput` literal.
    - Step 3: iterate `for (i, rx) in self.from_clients.iter().enumerate()`
      (avoid `needless_range_loop` under `-D warnings`) so the response can
      target `self.to_clients[i]`. The `PlayerInput` arm calls
      `apply_input(id, PlayerInput { .. }, seq)`. The `RequestRoomChange` arm
      resolves `let pid = self.clients[i].player_id;` (mirrors
      `addrs.get(&addr)`), increments `client_room_change_requests`, calls
      `self.server.apply_room_change(pid, direction)`, and on `Some(info)`
      builds one `RoomChangeResponse`, does
      `*self.room_change_response_counts.entry(handle).or_insert(0) += 1;` and
      increments `client_room_change_rejects` when `!info.accepted`, then sends
      the bytes to `self.to_clients[i]` **only**.
    - Step 7c: `request_room_change(&mut client.world)`.
    - `set_server_player_coords`: exact body —
      `let (px, py) = self.server_player_pos(handle); self.server.force_teleport(player_id, (x, y), (px, py));`
      and rewrite the stale doc comment (it used to abuse `apply_input` with
      client-claimed coords).
  - `rooms.rs` / `edge_cases.rs`: the tester's **B1–B9** and **F1** edits (test
    cases above) are the expected diff; the implementer changes the harness, not
    the expectations. Reconcile the lead tolerance with the b-tests as noted
    there (all b-tests stage both sides inside the raw band, so `can_enter`
    passes on the raw band; only E6 relies on the 8 px expansion).
  - Add **E1–E6** in the new file (`E6` is the walk-in; `E1–E5` are the cheat /
    rejection / respawn cases).
- **Acceptance**: `cargo test -p e2e` green (E1–E6, B1–B9, F1, plus movement /
  multiplayer / reconciliation / bullet-spawn / authoritative-death suites), then
  the full gate: `cargo test --workspace`,
  `cargo fmt --check --all`,
  `cargo clippy --all-targets --all-features --workspace -- -D warnings`.
- **Test**: E1–E6 (+ the tester's B1–B9 and F1 pin the choreography).
- **Explain**
  - What:
  - Why:
  - Rust concepts: _(likely: `iter().enumerate()` for per-client routing,
    `HashMap<ClientHandle, usize>` counters, serializing once and cloning bytes,
    `expect` in test/harness code, `force_teleport` with a preserved tuple
    position, struct-field additions)_
  - Watch out: _(Step 3 must answer the requesting client only — a broadcast makes
    every client adopt another player's room; door detection reads the predicted
    position, so the client must be staged too; b2/b4/b6/b8 must leave the entry
    band and outlive the cooldown; the 8 px lead tolerance is server-only and must
    not leak into `door_at`; f1's old assertion and name describe the behavior
    this spec deletes)_

## Definition of done

- [ ] `cargo test --workspace` passes (all named tests above present; existing
      suites green).
- [ ] `cargo fmt --check --all` clean.
- [ ] `cargo clippy --all-targets --all-features --workspace -- -D warnings` clean.
- [ ] Grep-clean over code only (historical `docs/` hits are fine):

  ```bash
  grep -RIn 'check_level_transitions\|coords_match' protocol/ server/ client/ game/ e2e/   # none
  grep -RIn 'test_no_game_or_glam_deps' protocol/                                           # none
  grep -RIn 'level_x\|level_y' protocol/src/lib.rs                                          # only PlayerPos + RoomChangeResponse
  grep -RIn -B2 -A8 'GamePacket::PlayerInput' protocol/ server/ client/ e2e/                # no room fields
  grep -RIn 'apply_input(' server/ e2e/                                                     # 3-arg form only
  grep -RIn 'server_trigger' client/                                                        # none: client detection stays geometric
  grep -RIn 'f1_bogus_level_coords_fall_back_to_origin' e2e/                                # none (renamed)
  ```

- [ ] `PlayerPos.level_x/level_y` still exist and are still used for nemesis
      placement; `RoomGraph` has exactly 8 doors; `Direction` is still
      `North, South, East, West`; `DOOR_LEAD_TOLERANCE_PX == 8.0` exists and
      `door_at` still uses the raw `trigger`.
- [ ] No `GamePacket::PlayerInput` field named `level_x`/`level_y` remains.
- [ ] Every `## Explain` block is filled in each commit body.
- [ ] Memory/process updates:
  - `docs/progress.md` — spec 3 complete (test counts, RED-window notes), next
    actions = ADR-0004 + the deferred lag-compensation/snapshot specs.
  - `docs/lessons.md` — new Rust concepts (enums as wire format, cross-crate
    shared types with `serde`, `Option` as a two-outcome reply,
    `HashMap::values_mut` + `saturating_sub`, an inherent method on a `Copy`
    struct for derived geometry, per-client routing in a test harness).
  - `docs/adr/0004-authoritative-room-membership.md` — door graph, request/ack,
    server teleport, cooldown, `force_teleport` seam, and the server-only
    `DOOR_LEAD_TOLERANCE_PX` asymmetry (Decision 17).
  - Flip the spec's `Status: draft` → `implemented` and this plan's status.
- [ ] Do not merge; hand the branch to the owner for validation.

## Explain (commit body template — one per box)

```text
## What
<the box's changes, one sentence per crate>

## Why
<how this box serves "room membership is server-authoritative for shooter and victim">

## Rust concepts
<e.g. plain-data structs with derived Copy/PartialEq; tuples as room coords;
 Option<RoomChangeInfo> as accept/reject; HashMap::values_mut + saturating_sub;
 serde derives across crates; Box<dyn PacketIO> calls; async-free per-client
 routing with enumerate()>

## Watch out
<the RED window (tester's batch → Box 6, e2e until Box 7); append-at-the-end
 packet variants; Direction wire order; entry-inside-reverse-trigger anti-bounce;
 the 8 px lead tolerance is server-only (`door_at` stays geometric); predicted-
 position gating in e2e; stale own PlayerPos must not undo a room; force_teleport
 bypasses doors; no glam in the server's normal deps>
```

## Open questions / deviations (for the plan reviewer)

1. **`protocol/tests/round_trip.rs::test_no_game_or_glam_deps` contradicts this
   spec.** It asserts protocol does **not** depend on `game`; the spec adds that
   dependency. Resolved: replace it with `protocol_depends_on_game_not_glam`
   using `cargo tree -p protocol --depth 1`, asserting `game v0.1.0` is present
   and no line contains `glam v`. `--depth 1` is load-bearing because `glam` is
   transitive through `game`. The spec's per-file list omits this test, but
   `cargo test --workspace` cannot pass without the rewrite.
2. **`e2e/tests/edge_cases.rs::f1_bogus_level_coords_fall_back_to_origin` also
   contradicts the spec** (it asserts the server stores client-reported coords)
   and its name describes deleted behavior. Resolved: rename to
   `f1_bogus_client_room_is_ignored` and rewrite it to assert the opposite
   (server stays `(0,0)`, movement still works, the own `PlayerPos` re-adopts the
   authoritative room). Not listed in the spec's per-file surface, same reason
   as #1.
3. **Harness `set_server_player_coords` needs a position for `force_teleport`.**
   Resolved: preserve the player's current position and change only the room, so
   existing fixtures (`authoritative_death.rs`) keep their meaning:
   `let (px, py) = self.server_player_pos(handle); self.server.force_teleport(player_id, (x, y), (px, py));`.
4. **Harness `RequestRoomChange` id resolution + response observability.** The
   real server resolves from `addrs.get(&addr)`; the harness resolves from the
   client index (`self.clients[i].player_id`) and ignores the packet id. This is
   required for the malicious-request tests to be meaningful (a client cannot
   name another player). The plan also adds `client_room_change_rejects` (named
   by the spec) and `room_change_responses(handle)` (plan-invented) so E3 can
   prove the ack went to the requester, not to everyone.
5. **`apply_room_change_local` resets `room_change_retry = 0`.** The spec's
   rejection text ("adopt … first, then set `room_change_retry`") only makes
   sense if the adoption clears it; the plan makes that explicit.
6. **`force_teleport` no-ops for an unspawned id** instead of inserting a phantom
   coord. The spec presumes a spawned player; the guard avoids creating state
   without one.
7. **Client test placement.** The spec names the client tests but not a module.
   Resolved: request/apply/respawn tests live in a new `mod tests` in
   `client/src/world/lib.rs` (own recorder); `send_input` and the packet-handler
   tests live in the existing `client/src/network/world.rs` `mod tests` (reuses
   `RecordingPacketIO`/`make_world`). The four logging all-variant tests are L1–L4
   in `client/src/network/logging.rs`. Box 5 (C1–C7 + L1–L4) / Box 6 (C8–C11)
   split keeps each box compilable and testable.
8. **`grounded` after a room teleport.** Recomputed from the target level at the
   canonical entry. It is `true` for the horizontal entries `(10,321)`,
   `(760,321)`, `(10,401)`, `(760,401)` (the row below is wall/exposed floor) and
   `false` for the vertical entries `(305,10)`, `(505,10)`, `(305,432)`,
   `(505,432)` (nothing solid under the shifted rect). S7 asserts the `(10,321)`
   value; S10 asserts `(10,401)`. Review should not read the true/false flip as
   drift.
9. **The spec's rooms.rs note ("b3/b4/b7/b8 previously set only the server
   position") is only accurate for b3/b4 in the current tree** — b7 and b8
   already stage both sides. The plan edits the actual offenders: b2, b3, b4, b6,
   b8 (b2/b4/b6/b8 for the re-arm + cooldown, b3 for the missing client stage).
   The 8 px lead expansion does not change these fixtures: every b-test has both
   sides inside the raw geometric band, so `can_enter` passes on the raw band.
   Only E6 (client leads the server at request time) depends on the expansion.
10. **`e2e/tests/edge_cases.rs::f5_disconnect_during_room_transition`** never
    asserts a transition and only needs the server healthy; it passes without
    edits. Its comment overstates the setup; optional cleanup, not required.
11. **Expected RED window** — the tester's RED batch introduces tests that
    reference the new game API, protocol variants, `World` fields, and harness
    fields, so nothing compiles from the batch through Box 6 (e2e until Box 7).
    Box 2's field removal also breaks the server binary, client crate, and e2e.
    This is inherent to a cross-crate wire change and matches the precedent in
    `docs/plans/authoritative-player-death.md`; each box's narrow command is the
    real gate until Box 7, and Box 5 accepts known C8–C11 failures.
12. **E1's literal `(10, 321)` assertion is unreachable with a 10-frame hold.**
    On the acceptance tick the server teleports to the canonical entry inside
    `apply_room_change` (Step 3) and then `update` (Step 4) applies the still-held
    `move_right`, walking ~6.7 px into the new room before the test can release.
    Resolved: the client starts at `(766,320)` — already inside the 40 px band —
    so a **single** `hold_right` frame keeps the walk-in spirit while leaving no
    held input to move off the entry after the ack; then poll for `(1,0)` and
    release. The assertion stays the spec's `±2 px` of `(10, 321)`. If the
    reviewer prefers the literal 10-frame hold, the assertion must become
    "within 2 px of the server's authoritative position and `x < 40`" instead.
    E6 covers the real walk-in separately (it starts at `700`, holds 25 frames,
    and asserts `client_room_change_rejects == 0`, not the entry position).
13. **The spec's Gotcha "two separate `#[cfg(test)]` blocks" is inaccurate.**
    `server/src/world.rs` actually has one `#[cfg(test)] mod tests`, **two
    crate-level `#[test]` fns** (`shoot_emits_bulletspawninfo`,
    `shoot_cooldown_enforced`), and `mod raycast_and_hit_tests`. The plan sweeps
    all three regions (the ripple table says so); the spec's prose is stale.
14. **Box 4 and the `unwrap` non-goal.** `server/src/main.rs` already serializes
    with `.unwrap()`; the plan scopes "no new unwrap/expect" to the four files
    the quality gate checks and gives the new `RequestRoomChange` arm an
    `if let Ok(bytes)` send, so the rule and the existing style do not conflict.
15. **E6 releases at the transition.** Holding 25 frames after the ack would walk
    the player right into the `(1,0)` interior (and potentially the South door at
    `(480,440)`); the test releases on the transition tick and then ticks 3,
    asserting the coords are never `(0,0)` and end at `(1,0)`. This keeps the
    "never returns to `(0,0)`" assertion about the reject/snap-back loop, not
    about refusing to cross another door.
