# Authoritative room membership

- Status: implemented (2026-11-26)
- Date: 2026-11-26
- Branch: `feat/authoritative-room-membership`
- Reference: `docs/specs/netcode-hardening-roadmap.md` (spec 3); spec 2 QA
  MAJOR-2; ADR 0003 ("Player death is server-authoritative")

## Summary

Room membership is currently **client-authoritative**. The client detects a
room edge in `check_level_transitions` (`client/src/world/lib.rs`), mutates
`world.current_coords` locally, and sends `level_x`/`level_y` inside every
`PlayerInput`. The server's `apply_input` (`server/src/world.rs`) blindly
executes `self.coords.insert(id, (x, y))`.

That gives a modified client authority over which room it is in:

- it can claim any room, including rooms that are not connected to its current
  room, with no movement;
- worse (spec-2 QA MAJOR-2), `raycast_player` filters candidates by
  `coords == shooter_room`, so a victim can claim an empty room and become
  **unkillable while still shooting other players**;
- the respawn's `coords = (0, 0)` is overwritten by the next inbound
  `PlayerInput`, so death does not even guarantee the victim is back in room
  `(0, 0)`.

This spec makes room membership **server-authoritative for both shooter and
victim**. It replaces the client-claimed coordinates with:

- an explicit `RoomGraph` of doors (shared pure logic in `game`);
- a `RequestRoomChange { direction }` client request (a direction, never a
  target room);
- a server-side `can_enter` check (door exists + the player's authoritative
  hitbox is at, or within the lead tolerance of, that door);
- an explicit `RoomChangeResponse` ack carrying the canonical entry position;
- a server teleport on acceptance, and a client snap-back on rejection.

The old `PlayerInput.level_x/level_y` fields, the client-side room transition
heuristic (including its hardcoded `10.0` entry offsets and `-5.0` edge
threshold), and the `coords_match` reconciliation skip are all removed.

## Goals / non-goals

**Goals**

- Add a shared `RoomGraph` + `Direction` + `Door` model to the `game` crate,
  encoding the four connected rooms as eight directed doors with per-door
  trigger bands and canonical entry positions.
- Remove `level_x`/`level_y` from `PlayerInput`; add
  `RequestRoomChange { id, direction }` and
  `RoomChangeResponse { accepted, level_x, level_y, x, y }` to `GamePacket`.
- Make `coords` mutable **only** by server-controlled paths: `spawn`,
  `remove`, `respawn`, `force_teleport`, and the validated
  `apply_room_change`. `apply_input` no longer touches `coords`.
- Add `ServerWorld::apply_room_change(id, direction)` with connectivity,
  position-at-door, and anti-spam validation; on acceptance it teleports the
  player to the door's canonical entry position.
- Add `ServerWorld::force_teleport` for join/respawn/test setup: a
  server-forced teleport that **bypasses** `can_enter` (a respawn must never be
  rejectable).
- Client: detect a door using its **predicted** position, send one request at a
  time, apply the ack, snap back on rejection, and re-arm door detection only
  after leaving the entry band.
- Remove the `coords_match` reconciliation skip; own `PlayerPos` becomes
  authoritative for room adoption as well (covers reliable-vs-unreliable
  packet ordering).
- Keep the existing behavior/feel of the four rooms: walking through a real
  opening changes rooms; walking into a wall or a non-door edge does not.
- Unit tests for `game::room_graph`, protocol round-trips, server
  validation/teleport, client request/ack/snap-back, and e2e coverage for the
  victim-side cheat and non-door teleport attempts.

**Non-goals**

- **No lag compensation** (deferred spec). Hits are resolved against the
  current tick's authoritative positions.
- **No PvE** (spec 1 removed it; PvP-only until a future PvE phase).
- **No checkpoint/room-local respawns.** Respawn stays the global canonical
  spawn `(0,0)` at `(100,100)`; "checkpoint spawns" is a later gameplay spec.
- **No world-snapshot batching / packet coalescing.** `PlayerPos` stays one
  packet per player per destination.
- **No room filtering of `PlayerDied`/`PlayerPos` broadcasts.** The fix is
  authoritative membership, not broadcast scoping.
- No new rooms, tiles, doors, or map edits; the existing 2×2 map is the
  complete graph for this spec.
- No diagonal transitions, sub-room doors, or multiple doors in the same
  direction from one room. `door(from, direction)` assumes at most one;
  the current map satisfies that. A future map with two east doors needs a
  door id, which is out of scope.
- No door animation, sound, fade, or transition delay.
- No protocol versioning or wire migration (single workspace; both sides are
  rebuilt together).
- No change to movement, prediction replay dt, interpolation, or the combat
  raycast geometry.

## Behavior

### Authority model (the invariant)

After this spec:

- `coords` is mutated only by `ServerWorld::{spawn, remove, respawn,
  force_teleport, apply_room_change}`.
- `apply_input` accepts movement/shoot flags and a sequence number only. There
  is no packet field through which a client can name a room.
- The only client-originated room request is
  `RequestRoomChange { id, direction }`, validated against the server's
  authoritative position, the door graph, and a short per-player cooldown.
- `raycast_player` continues to read `coords` for **both** the shooter and the
  victim. Because `coords` is now authoritative, neither side can spoof its
  room to dodge or land a hit.
- `respawn` and join are server-forced teleports to `(0,0)`; they never call
  `can_enter` and can never be rejected. The subsequent `PlayerInput` no longer
  carries coordinates, so it cannot overwrite the respawn room.

### Room graph and door geometry

`game` gains `game/src/room_graph.rs` containing `Direction`, `Door`,
`RoomCoords`, and `RoomGraph`. It is pure data/logic with no network or
rendering dependency.

A `Door` is a **directed** edge for one room edge. Bidirectional openings are
represented by two doors (A→B East and B→A West), each with its own trigger
band and entry position. A door stores:

- `from`: the source room;
- `direction`: which edge of `from` the door is on;
- `target`: the room entered;
- `trigger`: the **geometric** rectangle in the source room where the player
  must be to use the door. The server validates against `server_trigger()` (this
  rect plus `DOOR_LEAD_TOLERANCE_PX` on the approach edge); the client's
  `door_at` detection uses this raw rect.
- `entry`: the canonical player **top-left** position inside `target` after the
  teleport (the owner's "entry offset", expressed as an absolute position in
  the target room; the target's pixel origin is `(0,0)`).

Trigger semantics are deliberately shared between client and server, with a
single intentional asymmetry:

- **Client detection (geometric):** a door is detected when the client's
  **predicted** hitbox intersects the raw `trigger`:
  `trigger.overlaps(&Rect::new(player.pos.x, player.pos.y, w, h))`.
- **Server validation (lead-expanded):** `can_enter` tests the player's
  **authoritative** hitbox against `Door::server_trigger()`, which is the raw
  `trigger` grown by `DOOR_LEAD_TOLERANCE_PX` (8.0 px) on the approach edge:
  East grows the left edge inward (`x0 -= tol`), West the right edge
  (`x1 += tol`), South the top edge (`y0 -= tol`), North the bottom edge
  (`y1 += tol`). This absorbs ~one prediction tick of lead (~6.667 px at
  `VELOCIDAD = 400` and 60 Hz), so a normal walk-in's first request is
  accepted. The client never uses `server_trigger`; the asymmetry is server
  validation only and grants no arbitrary teleport.
- `Rect::overlaps` is strict (`<`, not `<=`); the trigger bands are one tile
  deep and include the reachable edge position, so the player's rect overlaps
  before the movement collision stops them at the level boundary.
- Using the full player rect (not its center) gives ~one player-width of
  tolerance and matches how the client and server derive "at the door" from
  the same data. There is no direction/facing heuristic.

`RoomCoords` is `(u8, u8)` with the same convention as `ServerWorld::coords`
and `PlayerPos`: `(level_x, level_y) = (column, row)`, indexing the level
matrix as `levels[level_y][level_x]`.

The current map is a ring. The complete door table (pixel coordinates; room is
800×480, `TILE_SIZE = 40`, player is 30×38) is:

| `from` | `direction` | `target` | `trigger` (x, y, w, h) | `entry` (x, y) | Opening in the source room |
| --- | --- | --- | --- | --- | --- |
| `(0, 0)` | East | `(1, 0)` | `(760, 320, 40, 40)` | `(10, 321)` | row 8, east edge |
| `(1, 0)` | West | `(0, 0)` | `(0, 320, 40, 40)` | `(760, 321)` | row 8, west edge |
| `(0, 0)` | South | `(0, 1)` | `(280, 440, 80, 40)` | `(305, 10)` | cols 7–8, south edge |
| `(0, 1)` | North | `(0, 0)` | `(280, 0, 80, 40)` | `(305, 432)` | cols 7–8, north edge |
| `(1, 0)` | South | `(1, 1)` | `(480, 440, 80, 40)` | `(505, 10)` | cols 12–13, south edge |
| `(1, 1)` | North | `(1, 0)` | `(480, 0, 80, 40)` | `(505, 432)` | cols 12–13, north edge |
| `(0, 1)` | East | `(1, 1)` | `(760, 400, 40, 40)` | `(10, 401)` | row 10, east edge |
| `(1, 1)` | West | `(0, 1)` | `(0, 400, 40, 40)` | `(760, 401)` | row 10, west edge |

Derivation of the values (so they are data, not magic heuristics):

- Horizontal doors occupy the one tile row `r` where the source edge tile is
  `Empty`: `y = r * 40`, height `40`; the trigger is the last/first tile
  column at the edge (`x = 760` for East, `x = 0` for West).
- Vertical doors span the one or two tile columns `c0..=c1` where the source
  edge is open: `x = c0 * 40`, width `(c1 - c0 + 1) * 40`; the trigger is one
  tile deep at the edge (`y = 440` for South, `y = 0` for North).
- Horizontal entry: `x = 10` for an Eastward target, `x = 760` for a Westward
  target; `y = r * 40 + 1` (the 38 px player centered in the 40 px gap).
- Vertical entry: `y = 10` for a Southward target, `y = 432` for a Northward
  target; `x = gap_center_x - 15` (305 for cols 7–8, 505 for cols 12–13).

Room-coordinate mapping for the table (the comments above the arrays in
`game/src/maps.rs` are misleading; the matrix indexing is authoritative):
`(0,0)=LVL_0_0`, `(1,0)=LVL_0_1`, `(0,1)=LVL_1_0`, `(1,1)=LVL_1_1`.

`RoomGraph::new` builds the eight doors from this table. Doors are unique per
`(from, direction)`; `door(from, direction)` returns the unique match.
`door_at(from, player_rect)` returns the door whose **raw geometric** trigger
overlaps the rect (used by the client to decide which request to send); table
order breaks a tie, but the current map has no overlapping doors. The server's
`can_enter` uses `server_trigger()` instead.

### Server: `apply_room_change`

`ServerWorld` owns a `RoomGraph` and a per-player `room_change_cooldown:
HashMap<u8, u8>` (ticks). `server/src/main.rs` handles each inbound
`RequestRoomChange`:

1. Resolve the player id from the transport map (`addrs.get(&addr)`), exactly as
   `PlayerInput` does. Ignore a mismatched `id`.
2. Call `world.apply_room_change(id, direction)`.
3. If it returns `Some(info)`, send **one** `RoomChangeResponse` to the
   **requesting address only**, reliable-ordered, and log the outcome:

   ```rust
   let response = GamePacket::RoomChangeResponse {
       accepted: info.accepted,
       level_x: info.room.0,
       level_y: info.room.1,
       x: info.pos.0,
       y: info.pos.1,
   };
   ```

`apply_room_change(id, direction) -> Option<RoomChangeInfo>` semantics:

1. If `players` has no `id`, return `None` (no state, no response).
2. `from = coords.get(&id).copied().unwrap_or((0, 0))`.
3. Build the player's authoritative rect from `player.pos`, `player.w`,
   `player.h`.
4. **Cooldown:** if `room_change_cooldown[id] > 0`, return `None` (no
   response). The client already has one outstanding request, so an honest
   client never triggers this; returning `None` (rather than a rejection)
   prevents a modified client from eliciting a response per packet.
5. **Connectivity + position:** if
   `!room_graph.can_enter(from, direction, rect)`, arm the cooldown
   (`room_change_cooldown.insert(id, ROOM_CHANGE_COOLDOWN_TICKS)`) and return a
   rejection (current room + `(player.pos.x, player.pos.y)`). `can_enter`
   validates against `Door::server_trigger` (the geometric band plus
   `DOOR_LEAD_TOLERANCE_PX` on the approach edge), not the raw `trigger`.
6. **Accept:**
   - `let door = room_graph.door(from, direction)` (guaranteed `Some` by step
     5);
   - `coords.insert(id, door.target)`;
   - `player.pos.x = door.entry.0; player.pos.y = door.entry.1;`
     (field assignment, so `server` keeps `glam` as a dev-dependency only);
   - `player.vel.x = 0.0; player.vel.y = 0.0;`
   - recompute `player.grounded` from the **target** level at the entry rect
     (the entry is canonical, so this is stable; it avoids a stale airborne
     jump flag for one tick);
   - preserve `player.dir`, `player.bullets`, `pending` input, `shoot_cooldown`,
     and `max_seq` — this is a room change, not a respawn;
   - `room_change_cooldown.insert(id, ROOM_CHANGE_COOLDOWN_TICKS)`;
   - return `RoomChangeInfo { accepted: true, room: door.target,
     pos: (player.pos.x, player.pos.y) }`.

Rejections never mutate `coords`, `player.pos`, or velocity.

The response's `level_x/level_y/x/y` are always the **current authoritative**
state: the target room + entry on acceptance, or the unchanged room + current
position on rejection (so the client has a precise snap-back target).

`update` decrements every entry of `room_change_cooldown` with
`saturating_sub(1)` once per tick. The cooldown exists to stop a modified
client from ping-ponging through a door at packet rate to flicker its room and
dodge a shooter's raycast, and to rate-limit rejected requests (response
amplification). It is set on **both** acceptance and rejection, so a player
elicits at most one `RoomChangeResponse` (accept or reject) per window.

`spawn` initializes `room_change_cooldown[id] = 0`; `remove` drops the entry.

### Server: forced teleports and respawn

`ServerWorld::force_teleport(id, room, pos)`:

- sets `coords[id] = room`;
- sets `player.pos.x = pos.0`, `player.pos.y = pos.1`, and zeroes both
  velocity components (field assignment; no new `glam` dependency);
- recomputes `grounded` from the target level;
- does **not** check `can_enter`, does not check the cooldown, and does not
  send a packet.

It is the seam for join/respawn/test setup. `spawn` continues to insert the
canonical `(0,0)` / `(100,100)` state directly. `respawn` continues to rebuild
`Player::new(id)`, set `coords = (0,0)`, and clear `pending` — and it must not
call `can_enter`. A respawn from any room to `(0,0)` is never rejectable
(owner decision 11). Because `PlayerInput` no longer carries room fields, the
next input cannot overwrite the respawn room.

`apply_input` loses its `x, y` parameters and the `coords.insert` line:

```rust
pub fn apply_input(&mut self, id: u8, input: PlayerInput, seq: u16)
```

All server unit tests, `server/src/main.rs`, and `e2e/src/harness.rs` call
sites must drop the two arguments.

### Client: request flow (gated on the predicted position)

The client never changes `current_coords` on its own movement. It keeps
`current_coords` for rendering and door lookup, and changes it only when the
server says so (`RoomChangeResponse` or an own `PlayerPos` with a new room) or
on a local respawn.

New `World` state (all `pub`, like the existing fields; `world::lib` and
`network::world` are sibling modules):

- `pub room_graph: RoomGraph`;
- `pub pending_room_change: Option<Direction>` — at most one outstanding
  request;
- `pub room_change_armed: bool` — false immediately after an accepted
  transition;
- `pub room_change_retry: u8` — ticks to wait after a rejection.

Add a small `World::current_room(&self) -> RoomCoords` helper that casts
`current_coords` from `(usize, usize)` to `(u8, u8)`; use it at every
`RoomGraph` call site (keeps the casts in one place).

Each simulation tick, where `check_level_transitions` used to run, call
`request_room_change(world)`:

1. If `world.id` is `None`, return.
2. If `pending_room_change.is_some()`, return (wait for the ack).
3. If `room_change_retry > 0`, decrement it and return.
4. Build a rect from the **predicted** `world.player.pos` and the local
   player's `w/h`, and test it against the door's raw geometric `trigger`.
   This matches the pre-spec walk-through feel: the request fires the moment
   the predicted body enters the band. Prediction runs about one tick ahead of
   the server, so when the request arrives the authoritative position can still
   be short of the geometric band; `can_enter` validates against
   `Door::server_trigger` (geometric band + `DOOR_LEAD_TOLERANCE_PX` on the
   approach edge), which absorbs that normal lead and accepts the first
   request. If real-world lead exceeds the tolerance, the server rejects, the
   client snaps back, and after `ROOM_CHANGE_RETRY_TICKS` it requests again once
   the predicted position is back in the band.
5. `let door = room_graph.door_at(current_room, rect)`.
   - `None`: `room_change_armed = true` (the player has left every trigger
     band); return.
   - `Some(door)` and `!room_change_armed`: return. The player is still inside
     the entry band of the door it just came through; do not bounce.
   - `Some(door)` and `room_change_armed`: send
     `GamePacket::RequestRoomChange { id, direction: door.direction }`
     (reliable-ordered), set `pending_room_change = Some(door.direction)`.

There is no local teleport and no local `current_coords` mutation in this path.
The client must not send a second request until the first is answered.

**Immediate re-crossing (deliberate behavior change):** every entry position
sits inside the reverse door's trigger band. After an accepted change the
client is disarmed while its rect overlaps any band, so a player who reverses
without leaving the doorway gets no request; they must step out of the entry
band first. This is the anti-bounce rule. The server cooldown additionally
rate-limits a quick out-and-back. Test choreography: `rooms.rs` b2/b4/b6/b8
(which reverse immediately) must leave the entry band for at least one tick
and let `ROOM_CHANGE_COOLDOWN_TICKS` expire before staging the reverse door;
because door detection now reads the predicted position, b3/b4/b7/b8 must also
stage the client position at the door (they previously set only the server
position). b1, b5, and b9 keep their outcomes.

### Client: applying `RoomChangeResponse`

`GamePacket::RoomChangeResponse { accepted, level_x, level_y, x, y }` is the
explicit ack (owner decision 4):

- **Accepted:** call `apply_room_change_local(world, (level_x, level_y),
  Vec2::new(x, y))`. This sets `current_coords`, sets the player's top-left to
  the entry position with zero velocity (preserving `dir`), snaps
  `prev_player_pos/prev_player_dir` so `draw(alpha)` does not lerp from the old
  room, clears the unacknowledged input buffer (old-room inputs must not be
  replayed in the new room), clears `pending_room_change`, sets
  `room_change_armed = false`, and calls `load_current_level` (bullets,
  bullet-id sets, clock sync, sticky edges).
- **Rejected:** clear `pending_room_change`, set
  `room_change_retry = ROOM_CHANGE_RETRY_TICKS`, snap the player to
  `(x, y)` with zero velocity, snap the
  interpolation source, and clear the unacknowledged input buffer so the
  snap-back is not immediately undone by replay. If the response's room
  differs from `current_coords` (defensive; the server does not mutate state on
  a rejection), adopt that room with `apply_room_change_local` first, then set
  `room_change_retry = ROOM_CHANGE_RETRY_TICKS` so the adoption does not
  re-arm the retry.

Applying the same accepted response twice is harmless (the state is
idempotent).

### Client: own `PlayerPos` and reconciliation

`PlayerPos` keeps `level_x`/`level_y` (server→client, needed for nemesis
rendering). For the local player's `PlayerPos`:

1. If the packet is stale (`!game::is_newer_seq(last_seq,
   world.last_acked_seq)`), do **not** adopt its room and let `reconcile`
   ignore it as today. A stale pre-transition packet must not drag the client
   back to the old room.
2. If `(level_x, level_y) != current_coords` (and the packet is not stale), the
   server has already moved the player to another room (the response may still
   be in flight on the reliable stream while `PlayerPos` is unreliable). Treat
   it as authoritative: call `apply_room_change_local` for the reported
   room/position before reconciling.
3. Remove the `coords_match` guard: reconciliation now runs whenever the room
   is valid and the position is not inside a wall. Keep the existing bounds
   guard (a bogus `(99,99)` must not index-panic) and the `not_in_wall` guard
   (defense in depth; the server's teleport entry is canonical, but a stale or
   malformed packet must not rewind the player into geometry).

The own `PlayerPos` is no longer used for door detection (that now reads the
predicted `world.player.pos`); it is still authoritative for room adoption and
position reconciliation.

This is why `RoomChangeResponse` and own `PlayerPos` can arrive in either
order: the response is the explicit ack, and the own `PlayerPos` is a
fallback that adopts the same authoritative state.

Remote `PlayerPos` handling is unchanged: update the `Nemesis` room and
interpolation snapshot as today. The existing `pos_in_viewer_bounds` check is
kept.

### Client: respawn / join

- `respawn_local` continues to rebuild `Player::new(id)`, set
  `current_coords = (0,0)`, and `load_current_level`. It additionally sets
  `pending_room_change = None`, `room_change_retry = 0`, and
  `room_change_armed = true` (the spawn is not in a trigger band).
- `JoinResponse` initializes the same new fields (`pending = None`,
  `retry = 0`, `armed = true`), in addition to the existing session resets.

### Edge cases

- **Player at a non-door edge** (e.g. West from `(0,0)`, which has no west
  door): `door_at` returns `None`; no request. The wall collision already
  prevents leaving; the client stays put.
- **Malicious request for a non-connected direction** (North/West from
  `(0,0)`): `can_enter` is false; the server rejects, coords are unchanged, and
  the response is `accepted: false` with the current room/position.
- **Malicious request for a real door while far from it** (East from
  `(100,200)` in `(0,0)`): the rect does not overlap the door's
  `server_trigger`; rejected.
- **Request arrives after the player moved away** (stale client view): the
  server validates the current authoritative rect against `server_trigger`;
  rejected (unless still within the lead tolerance), client snaps back to the
  server position.
- **Two doors in one room have overlapping triggers**: not present in this
  map. `door_at` returns the first in table order; `can_enter` stays exact per
  direction. A future map must add a door id (non-goal).
- **Player enters a room at the canonical entry that lies inside the reverse
  door's trigger** (all current entries do): the client is disarmed until it
  leaves the band, so no immediate bounce; the server cooldown is the
  anti-cheat backstop.
- **Repeated accepted requests within the cooldown**: rejected until the
  cooldown expires. The client's retry cooldown makes an honest client retry
  shortly after a rejection.
- **Victim tries to claim an empty room while a shooter is in the old room**:
  rejected (no door/not at door); the victim stays in the shooter's room and
  remains hittable. This is the spec-2 MAJOR-2 closure.
- **Victim is legitimately in another room**: `raycast_player` does not
  consider them, as today. This is correct room filtering, not a cheat.
- **Respawn while holding movement into a door**: respawn places the player at
  `(0,0)`/`(100,100)`, away from every trigger; the next `PlayerInput` cannot
  change the room.
- **`RoomChangeResponse` arrives before the respawned `PlayerPos`** or after
  the own `PlayerPos`: both paths converge on the same authoritative state; a
  duplicate application is idempotent.
- **Response lost?** The request and response are reliable-ordered, so this is
  a transport failure, not a normal case; the client's `pending` state would
  block further requests until the link is re-established. (This spec does not
  add a timeout; owner decision 4 chose an explicit ack rather than a timeout
  mechanic.)
- **Client sends a `RoomChangeResponse` or server sends a
  `RequestRoomChange`**: ignored (defense in depth; variants are
  direction-specific).

## Data model

```rust
// game/src/room_graph.rs (new)
use crate::rect::Rect;

pub type RoomCoords = (u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    North, // variant index 0 — do not reorder after release
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
    pub trigger: Rect,
    /// Canonical player top-left position inside `target` after teleport.
    pub entry: (f32, f32),
}

impl Door {
    /// Server-side validation rect: the geometric `trigger` grown by
    /// `DOOR_LEAD_TOLERANCE_PX` on the approach edge (East: left, West: right,
    /// South: top, North: bottom), so a predicted-first request that is ~one
    /// tick short of the door still validates. The client's `door_at` uses the
    /// raw `trigger`, never this.
    pub fn server_trigger(&self) -> Rect {
        let tol = crate::config::DOOR_LEAD_TOLERANCE_PX;
        let t = self.trigger;
        match self.direction {
            Direction::East => Rect::new(t.x - tol, t.y, t.w + tol, t.h),
            Direction::West => Rect::new(t.x, t.y, t.w + tol, t.h),
            Direction::South => Rect::new(t.x, t.y - tol, t.w, t.h + tol),
            Direction::North => Rect::new(t.x, t.y, t.w, t.h + tol),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoomGraph {
    doors: Vec<Door>,
}

impl RoomGraph {
    pub fn new() -> Self;
    pub fn doors(&self) -> &[Door];
    pub fn door(&self, from: RoomCoords, direction: Direction) -> Option<Door>;
    pub fn door_at(&self, from: RoomCoords, player_rect: Rect) -> Option<Door>;
    pub fn can_enter(
        &self,
        from: RoomCoords,
        direction: Direction,
        player_rect: Rect,
    ) -> bool;
}

impl Default for RoomGraph {
    fn default() -> Self {
        Self::new()
    }
}
```

`can_enter` is
`door(from, direction).map(|d| d.server_trigger().overlaps(&player_rect)).unwrap_or(false)`.
`door_at` (client detection, raw geometric band) is
`doors.iter().find(|d| d.from == from && d.trigger.overlaps(&player_rect)).copied()`.

```rust
// server/src/world.rs
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomChangeInfo {
    pub accepted: bool,
    pub room: RoomCoords,
    /// Authoritative player top-left (x, y); a plain pair so the server crate
    /// does not need a new normal `glam` dependency for its public API.
    pub pos: (f32, f32),
}

impl ServerWorld {
    pub fn apply_room_change(
        &mut self,
        id: u8,
        direction: Direction,
    ) -> Option<RoomChangeInfo>;
    pub fn force_teleport(&mut self, id: u8, room: RoomCoords, pos: (f32, f32));
    pub fn apply_input(&mut self, id: u8, input: PlayerInput, seq: u16);
}
```

`ServerWorld` also gains private `room_graph: RoomGraph` and
`room_change_cooldown: HashMap<u8, u8>`.

```rust
// client/src/world/lib.rs
pub fn request_room_change(world: &mut World);
pub fn apply_room_change_local(
    world: &mut World,
    room: RoomCoords,
    pos: glam::Vec2,
);
```

```rust
// protocol/src/lib.rs
PlayerInput {
    id: u8,
    // level_x/level_y REMOVED
    move_left: bool,
    move_right: bool,
    jump: bool,
    shoot: bool,
    seq: u16,
},

// appended after BulletSpawn (new)
RequestRoomChange {
    id: u8,
    direction: game::room_graph::Direction,
},
RoomChangeResponse {
    accepted: bool,
    level_x: u8,
    level_y: u8,
    x: f32,
    y: f32,
},
```

`PlayerPos` is unchanged and **keeps** `level_x`/`level_y`.

New named constants in `game/src/config.rs` (no magic numbers at call sites):

```rust
/// Server: minimum ticks between room-change responses (accept or reject) for
/// one player. ~100 ms at 60 Hz. Arms on both acceptance and rejection, so a
/// modified client cannot spam rejected requests for response amplification.
pub const ROOM_CHANGE_COOLDOWN_TICKS: u8 = 6;

/// Client: ticks to wait after a rejected request before retrying.
pub const ROOM_CHANGE_RETRY_TICKS: u8 = 6;

/// Server-side only: extra px added to a door's geometric trigger on the
/// approach edge before `can_enter` validates. Covers one prediction tick
/// (~VELOCIDAD / TICK_HZ ≈ 6.667 px) with margin. The client's `door_at`
/// detection does not use this.
pub const DOOR_LEAD_TOLERANCE_PX: f32 = 8.0;
```

Crate dependencies:

- `game/Cargo.toml` adds `serde = { version = "1.0.228", features = ["derive"] }`
  for `Direction`'s derive (the owner requires `Direction` in `game` and
  serializable).
- `protocol/Cargo.toml` adds `game = { path = "../game" }` so the packet can
  carry `game::room_graph::Direction`. There is no cycle: `game` does not
  depend on `protocol`.

| Item | Today | After |
| --- | --- | --- |
| `PlayerInput.level_x/level_y` | present, client-claimed | removed |
| `PlayerPos.level_x/level_y` | present | unchanged |
| `ServerWorld.coords` mutation | `apply_input` writes it every input | only `spawn`/`remove`/`respawn`/`force_teleport`/`apply_room_change` |
| `Direction`, `Door`, `RoomGraph` | absent | new in `game` |
| `ServerWorld::apply_room_change` | absent | new (validated request path) |
| `ServerWorld::force_teleport` | absent | new (bypasses doors) |
| `room_change_cooldown` | absent | new per-player tick cooldown |
| `client::check_level_transitions` | local edge heuristic + teleport | replaced by `request_room_change` (no local room mutation) |
| `client` own-`PlayerPos` `coords_match` skip | skips reconciliation across rooms | removed; the client adopts the server room |
| `client::World` | no room-request state | `room_graph`, `pending_room_change`, `room_change_armed`, `room_change_retry` |
| `e2e set_server_player_coords` | abuses `apply_input` | `force_teleport` |

## Network

| Packet | Before | After |
| --- | --- | --- |
| `PlayerInput` | client → server, unreliable, includes `level_x/level_y` | client → server, unreliable, **no room fields** |
| `RequestRoomChange { id, direction }` | — | client → server, **reliable-ordered**, at most one outstanding per client |
| `RoomChangeResponse { accepted, level_x, level_y, x, y }` | — | server → **requesting client only**, **reliable-ordered** |
| `PlayerPos` | server → all, unreliable, has `level_x/level_y` | unchanged (keeps the room fields) |
| `PlayerDied` | server → all, reliable-ordered | unchanged |

- `server/src/main.rs` resolves the id from the transport map for
  `RequestRoomChange` (do not trust the packet id for state lookup) and sends
  the response only to `addr`, not to all clients.
- The server ignores inbound `RoomChangeResponse`; the client ignores inbound
  `RequestRoomChange` (explicit arms or the existing catch-all, documented).
- Postcard: append the two new variants at the **end** of `GamePacket` (after
  `BulletSpawn`) so the existing variants keep their indices. Removing the two
  `PlayerInput` fields changes that variant's payload; both sides are rebuilt
  from the same workspace, so no wire migration is needed.
- `Direction` is encoded by postcard as its variant index; the order
  `North, South, East, West` is part of the wire format. Do not reorder.
- Packet logging (`RUSTVANIA_PACKET_LOG`) in both `server/src/main.rs` and
  `client/src/network/logging.rs` must report `"RequestRoomChange"` and
  `"RoomChangeResponse"`, with `packet_id = Some(id)` for the request and
  `None` for the response, and key fields
  `{ "id": id, "direction": direction }` /
  `{ "accepted": accepted, "level": [level_x, level_y], "pos": [x, y] }`.
- Reliability of the request/response is deliberate: the request is a one-shot
  state transition, not a per-tick input; losing it would leave the client
  `pending` until reconnect. `PlayerPos` stays unreliable.

## Per-file surface (for the planner)

- `game/Cargo.toml`: add `serde`.
- `game/src/lib.rs`: `pub mod room_graph;`.
- `game/src/room_graph.rs` (new): `Direction`, `Door` (including
  `server_trigger` using `crate::config::DOOR_LEAD_TOLERANCE_PX`), `RoomCoords`,
  `RoomGraph`, the eight-door table, unit tests.
- `game/src/config.rs`: add `ROOM_CHANGE_COOLDOWN_TICKS`,
  `ROOM_CHANGE_RETRY_TICKS`, `DOOR_LEAD_TOLERANCE_PX`.
- `protocol/Cargo.toml`: add `game = { path = "../game" }`.
- `protocol/src/lib.rs`: remove `PlayerInput.level_x/level_y`; append
  `RequestRoomChange` / `RoomChangeResponse`; import `Direction`.
- `protocol/tests/round_trip.rs`: update the `PlayerInput` fixtures; add
  round-trips for the two new variants (all four directions; accepted
  true/false).
- `server/src/world.rs`: `apply_input` signature; remove the `coords.insert`;
  add `use game::room_graph::{Direction, RoomCoords, RoomGraph};`, `room_graph`,
  `room_change_cooldown`, `apply_room_change`, `force_teleport`,
  `RoomChangeInfo`; decrement cooldowns in `update`; initialize/remove
  cooldowns in `spawn`/`remove`; keep `raycast_player` reading authoritative
  `coords`; update every test call to `apply_input`.
- `server/src/lib.rs`: export `RoomChangeInfo`.
- `server/src/main.rs`: logging helpers for the new variants; drop
  `level_x/level_y` from the `PlayerInput` arm and `apply_input` call; add the
  `RequestRoomChange` arm (id resolution, `apply_room_change`, response to the
  requester, log); ignore inbound `RoomChangeResponse`.
- `client/src/world/game.rs`: add the new `World` fields and initialize them;
  call `request_room_change` where `check_level_transitions` was; update the
  stale comment about `load_current_level` clearing bullets (the request path
  no longer loads a level).
- `client/src/world/lib.rs`: delete `check_level_transitions`; add
  `request_room_change` and `apply_room_change_local`; update `respawn_local`
  for the new state; keep `load_current_level` as the shared level reset.
- `client/src/lib.rs`: update the re-export (`check_level_transitions` →
  `request_room_change`).
- `client/src/network/world.rs`: `send_input` drops the room fields; own
  `PlayerPos` adopts a new room and removes the `coords_match` skip (it no
  longer feeds door detection); add the `RoomChangeResponse` handler; add
  explicit ignore arms; tests.
- `client/src/network/transport.rs`: both `PacketIO::send` match blocks send
  `RequestRoomChange` reliable-ordered (with `JoinRequest`).
- `client/src/network/logging.rs`: new variants in
  `packet_type_name` / `packet_id` / `packet_key_fields` and their
  all-variants tests; drop `level_x/level_y` from every `PlayerInput` fixture.
- `client/src/network/prediction.rs`: update the AC-38 doc comment that
  describes the server lagging across a room transition (the client now adopts
  the authoritative room; the comment must not claim the old behavior).
- `e2e/src/harness.rs`: drop the room fields from the Step 2 `PlayerInput`;
  handle `RequestRoomChange` in Step 3 and send the response to **that client**
  (iterate with the client index); count requests for cheats and rejected
  responses (`client_room_change_rejects`, used by the walk-in test); change
  `set_server_player_coords` to `force_teleport`; keep Step 5 `PlayerPos`
  broadcasts unchanged; Step 7c now calls `request_room_change`.
- `e2e/tests/rooms.rs`: update the b1 comment that names
  `check_level_transitions`; because door detection now uses the predicted
  position, b3/b4/b7/b8 must also stage the client position at the door (they
  previously set only the server position). b2/b4/b6/b8 must leave the entry
  band for a tick (to re-arm) and tick past `ROOM_CHANGE_COOLDOWN_TICKS` before
  staging the reverse door, because of the anti-bounce rule. b1, b5, b9 keep
  their outcomes.
- `e2e/tests/authoritative_room_membership.rs` (new): valid transition,
  non-door teleport rejection, victim cheat scenario, respawn-room stability,
  snap-back injection.

## Acceptance criteria

**Build / hygiene**

- `cargo test --workspace` passes.
- `cargo fmt --check --all` clean.
- `cargo clippy --all-targets --all-features --workspace -- -D warnings` clean.
- Code-only grep (`protocol/`, `server/`, `client/`, `game/`, `e2e/`) is clean
  for `check_level_transitions` and for `PlayerInput` constructions that still
  pass `level_x`/`level_y`. `PlayerPos.level_x/level_y` must still exist.
- No `GamePacket::PlayerInput` field named `level_x`/`level_y` remains.

**Game (`game/src/room_graph.rs`)**

- `RoomGraph::new().doors().len() == 8`.
- Every `(from, direction)` pair is unique.
- Every door has a reverse counterpart (`target` has a door in the opposite
  direction — East↔West, North↔South — pointing back to `from`), and the
  reverse trigger/entry are the expected values from the table.
- `door(from, direction)` returns the table entry; `door` for a non-existent
  direction (North/West from `(0,0)`) returns `None`.
- `Door::server_trigger` grows only the approach edge by
  `DOOR_LEAD_TOLERANCE_PX`: East `(0,0)` → `(752, 320, 48, 40)`, West `(1,0)`
  → `(0, 320, 48, 40)`, South `(0,0)` → `(280, 432, 80, 48)`, North `(0,1)` →
  `(280, 0, 80, 48)`; assert the analogous values for the other four doors.
- `can_enter` is true for a 30×38 rect placed at the edge inside each of the
  eight geometric trigger bands (e.g. `Rect::new(766, 320, 30, 38)` for East
  from `(0,0)`), and false for the same direction when the rect is far away
  (e.g. `(100, 200)`).
- `can_enter` accepts a rect whose near edge is within the lead tolerance but
  outside the geometric band: East from `(0,0)`
  `Rect::new(730, 320, 30, 38)` → true (right edge 760; expanded left edge
  752), while `Rect::new(700, 320, 30, 38)` → false. The same `(730, 320)` rect
  is `door_at == None` — client detection stays on the geometric trigger.
- One boundary case per direction: West `(1,0)`
  `Rect::new(42, 320, 30, 38)` true / `(50, 320, 30, 38)` false; South `(0,0)`
  `Rect::new(305, 400, 30, 38)` true / `(305, 390, 30, 38)` false; North
  `(0,1)` `Rect::new(305, 44, 30, 38)` true / `(305, 50, 30, 38)` false.
- `door_at` returns the expected `Direction` for each edge and `None` in the
  middle of each room; it uses the raw geometric `trigger`, not
  `server_trigger`.
- For each door, the point at the trigger's center is `TileType::Empty` in the
  source level (validate against `Map::new()`; use a point/tile check, not
  `solid_in_rect(trigger)`, because the trigger's inner boundary intentionally
  shares a tile with the floor row).
- For each door, `entry` is in bounds and
  `Level::solid_in_rect(Rect::new(entry.0, entry.1, 30.0, 38.0))` is false in
  the target level.

**Protocol**

- `GamePacket::PlayerInput` round-trips with no room fields (all flags
  true/false combinations used by the existing test style, plus `seq`).
- `GamePacket::RequestRoomChange` round-trips for all four `Direction`
  variants and ids `0`/`255`.
- `GamePacket::RoomChangeResponse` round-trips for `accepted: true` and
  `accepted: false` with room/position fields.
- The client logging all-variants tests cover the two new variants;
  `packet_id(RequestRoomChange)` is `Some(id)` and
  `packet_id(RoomChangeResponse)` is `None`.

**Server unit tests (`server/src/world.rs`)**

- `apply_room_change_accepts_at_door`: player in `(0,0)` at `(766, 320)` with
  `w=30/h=38`; `apply_room_change(id, East)` → `accepted: true`,
  `room == (1,0)`, `pos == (10, 321)`; `coords[id] == (1,0)`.
- `apply_room_change_accepts_within_lead_tolerance`: player in `(0,0)` at
  `(730, 320)` (one prediction tick short of the geometric East band, inside
  `server_trigger`); `apply_room_change(id, East)` → `accepted: true`,
  `coords == (1,0)`, `pos == (10, 321)`.
- `apply_room_change_rejects_beyond_lead_tolerance`: same setup at
  `(700, 320)`; `East` → `accepted: false`, coords/pos unchanged.
- `apply_room_change_rejects_no_door`: player at `(100,100)` in `(0,0)`;
  `North` and `West` → `accepted: false`, `coords` unchanged, `pos` unchanged.
- `apply_room_change_rejects_off_trigger`: player in `(0,0)` at `(100, 200)`;
  `East` → rejected, coords/pos unchanged.
- `apply_room_change_rejects_during_cooldown`: accept one change, then
  immediately request the reverse while still at the entry →
  `accepted: false`; tick `ROOM_CHANGE_COOLDOWN_TICKS` times and the same
  request is accepted (the reverse door is valid from the entry position, so
  the cooldown is the only blocker).
- `apply_room_change_teleports_to_entry`: on acceptance the player's `pos` is
  exactly the door `entry`, `vel == Vec2::ZERO`, `dir` is unchanged,
  `pending` held flags are preserved (not reset), `shoot_cooldown`/`max_seq`
  are unchanged.
- `apply_input_never_changes_coords`: `force_teleport` a player to `(1,0)`,
  then call `apply_input` with movement/shoot flags; `coords` stays `(1,0)`
  (the input has no room field to overwrite it). After `respawn`, call
  `apply_input`; `coords` stays `(0,0)` (the spec-2 respawn-overwrite bug).
- `respawn_ignores_door_validation`: respawn a player from `(1,1)` to
  `(0,0)`/`(100,100)` without a request; it succeeds and `coords == (0,0)`.
- `force_teleport_bypasses_doors`: place a player in `(0,0)` and
  `force_teleport(id, (1,1), (10.0, 401.0))`; coords/pos change without a
  door check.
- `raycast_victim_cannot_hide_by_room_claim`: shooter and victim in `(0,0)`
  aligned on row 5; the victim's attempt to `apply_room_change` to
  `(1,0)`/`(0,1)` without being at a door is rejected; `raycast_player` still
  returns the victim.

**Client unit tests**

- `room_change_request_sent_when_predicted_pos_at_door`: `World` with a
  recording `PacketIO`, `id = Some(1)`, `current_coords = (0,0)`,
  `player.pos = (766, 320)`, `pending = None`, `armed = true`;
  `request_room_change` sends exactly one
  `RequestRoomChange { id: 1, direction: East }` and sets
  `pending_room_change = Some(East)`.
- `room_change_not_sent_away_from_door`: same setup with
  `player.pos = (300, 200)` → no send, `pending = None`.
- `room_change_not_sent_when_disarmed`: same door setup with
  `room_change_armed = false` → no send.
- `room_change_rearms_after_leaving_band`: start disarmed at the entry, call
  `request_room_change` (no send); move `player.pos` away, call (armed becomes
  true); move back, call (sends).
- `room_change_response_accepted_applies_room_and_entry`: handle
  `RoomChangeResponse { accepted: true, level_x: 1, level_y: 0, x: 10.0, y:
  321.0 }` → `current_coords == (1,0)`, `player.pos == (10, 321)`,
  `pending == None`, `room_change_armed == false`, input buffer empty, bullets
  cleared.
- `room_change_response_rejected_snaps_back`: player at `(500, 300)` in
  `(0,0)`; handle `accepted: false` with `(760, 321)` and room `(0,0)` →
  `player.pos == (760, 321)`, `pending == None`,
  `room_change_retry == ROOM_CHANGE_RETRY_TICKS`, room unchanged.
- `own_player_pos_with_new_room_adopts_it`: handle an own `PlayerPos` with a
  newer `last_seq`, room `(1,0)` and `(10, 321)` while `current_coords ==
  (0,0)` → room/pos adopted, previous interpolation snapped, then the normal
  reconcile path is safe.
- `stale_own_player_pos_does_not_undo_room_change`: after an accepted
  `RoomChangeResponse` to `(1,0)`, handle an own `PlayerPos` with the old room
  `(0,0)` and an older `last_seq` → the client stays in `(1,0)`.
- `send_input_omits_room_fields`: the recorded `PlayerInput` built by
  `send_input` has no room data (compile-time by the struct; assert the other
  flags/seq still flow).
- `respawn_local_resets_room_request_state`: after `PlayerDied` for self, the
  room is `(0,0)`, `player.pos == (100,100)`, `pending == None`,
  `armed == true`.

**E2E (`e2e/tests/authoritative_room_membership.rs` and existing suites)**

- `e2e_valid_door_transition_changes_room_once`: one client at the `(0,0)`
  East edge (`server` and `client` pos ~`(766, 320)`); hold right until the
  client's coords become `(1,0)` (poll, ≤10 ticks), then release input and
  tick twice. The client's `current_coords` is `(1,0)`, the server's coords are
  `(1,0)`, and the client position is within 2 px of `(10, 321)`. No second
  transition happens while the client stands at the entry.
- `e2e_walk_in_first_request_accepted` (the plan-review gap): stage **both**
  client and server at `(700, 320)` on row 8 of `(0,0)`, hold right, and tick
  until the client's coords become `(1,0)` (≤25 ticks). Assert the transition
  happens on the first request: the harness's `client_room_change_rejects` is
  `0` (no `accepted: false` response for that client), and the client never
  returns to `(0,0)` after entering `(1,0)`. This is the deterministic
  ~1-tick-lead walk-in that the `DOOR_LEAD_TOLERANCE_PX` band must absorb.
- `e2e_non_door_teleport_is_rejected`: place a client in `(0,0)` at
  `(300, 200)`, send `RequestRoomChange { id, direction: North }` (and
  separately `West`, and `East` from the middle); tick; the server/client
  coords stay `(0,0)`, the client's position matches the server's
  authoritative position after the rejection (within 2 px), and the client did
  not change room.
- `e2e_spoofed_room_cannot_dodge_death` (spec-2 QA MAJOR-2): two clients in
  `(0,0)` on row 5 (shooter at `(100, 201)`, victim at `(400, 201)`); the
  victim sends a malicious `RequestRoomChange` for a non-door direction
  (and/or East while not at the door); tick so the rejection is processed;
  then the shooter shoots and the victim must die exactly once
  (`PlayerHitInfo { shooter_id, victim_id }`), with the victim respawned at
  `(100,100)`/`(0,0)`. The victim is never unkillable.
- `e2e_respawn_room_survives_later_inputs`: kill the victim, then tick several
  times while the victim's client sends normal `PlayerInput`s; the server's
  victim coords stay `(0,0)` (the respawn is not overwritten).
- `e2e_rejection_snaps_client_back`: place the client at `(500, 300)` and the
  server player at the same target `(760, 321)` in `(0,0)` (so the same-tick
  `PlayerPos` agrees), then inject a
  `RoomChangeResponse { accepted: false, level_x: 0, level_y: 0, x: 760.0, y:
  321.0 }`; after the next tick the client's position is within 2 px of
  `(760, 321)` and its coords are `(0,0)`.
- Existing `e2e/tests/rooms.rs` b1, b5, and b9 pass unchanged in outcome;
  b3/b4/b7/b8 additionally stage the client position at the door (predicted
  gating); b2/b4/b6/b8 leave the entry band (re-arm) and wait out the cooldown
  before reversing. Existing movement, multiplayer, reconciliation,
  bullet-spawn, and authoritative-death suites pass.

## Decisions

Owner decisions captured before this spec (do not re-litigate):

1. Branch from `dev` (specs 1 and 2 are merged there).
2. Use an explicit **room graph** as the door model, not geometry derived from
   tile gaps.
3. **Client requests, server validates**: `RequestRoomChange { direction }`.
4. Send an explicit **`RoomChangeResponse`** ack, not a silent success or a
   timeout-based mechanism.
5. The request carries a **`Direction` enum** (`North`/`South`/`East`/`West`),
   not target coordinates.
6. `can_enter` is **connectivity + position-at-door**: the server checks the
   player's authoritative position is at (or within `DOOR_LEAD_TOLERANCE_PX`
   of) that door's trigger band before granting.
7. The **server teleports** to a canonical entry position on an accepted
   change. This removes the old "D9: server never teleports on room
   transition" boundary and the client's `coords_match` reconciliation skip.
8. `RoomGraph` lives in the **`game` crate** (shared pure logic, near
   `Map`/`Level`).
9. **Door-anchored geometry**: the graph stores per-door direction, entry
   offset, and a trigger band, so both sides derive "is the player at this
   door" from the same data — no magic-number heuristic.
10. `RoomChangeResponse` on acceptance carries the **entry position** (x, y +
    room).
11. **Respawn/join = server-forced teleport**, bypassing `can_enter`. A
    respawn to `(0,0)` from any room must never be rejectable.

Decisions made while writing this spec:

12. **Client request gating:** the client sends `RequestRoomChange` when its
    **predicted** position (`world.player.pos`) overlaps a door trigger. This
    matches the current instant walk-through feel. Prediction runs about one
    tick ahead of the server; the server validates against `server_trigger`
    (geometric band + `DOOR_LEAD_TOLERANCE_PX`), so the normal lead is accepted
    on the first request. Lead beyond the tolerance can still reject → snap
    back → retry after `ROOM_CHANGE_RETRY_TICKS`.
13. **Trigger rule:** "at the door" is the player's full hitbox overlapping
    the door's one-tile trigger band (`Rect::overlaps`), not the center point.
    The client applies the overlap rule to its **predicted** hitbox against the
    raw **geometric** `trigger`; the server applies it to its **authoritative**
    hitbox against the **lead-expanded** `server_trigger` (the geometric trigger
    grown by `DOOR_LEAD_TOLERANCE_PX` on the approach edge). The client never
    uses `server_trigger`. The shared rect-overlap geometry provides about one
    player-width of tolerance; there is no direction/facing heuristic.
14. **Reliability:** `RequestRoomChange` and `RoomChangeResponse` are
    reliable-ordered; `PlayerPos` stays unreliable. Own-`PlayerPos` room
    adoption covers the cross-stream ordering case.
15. **Anti-ping-pong + anti-amplification:** room changes are rate-limited by
    `ROOM_CHANGE_COOLDOWN_TICKS` (= 6) per player, armed on **both** acceptance
    and rejection, so at most one `RoomChangeResponse` per player per window.
    While the cooldown is active the server returns `None` (no response).
    Rejections return the current authoritative state so the client can snap
    back.
16. **Both sides authoritative:** the shooter's and the victim's rooms are
    read from `coords` for hit detection, and `coords` now changes only via
    the validated paths. This closes QA MAJOR-2 for both roles.
17. **Server lead tolerance (Q13):** `can_enter` validates against
    `Door::server_trigger()`, the geometric trigger grown by
    `DOOR_LEAD_TOLERANCE_PX` (8.0 px) on the approach edge (East: left, West:
    right, South: top, North: bottom). The client's `door_at` stays on the raw
    geometric trigger. This absorbs ~one tick of prediction lead so a normal
    walk-in's first request is accepted; authority is preserved because the
    player must still be at/near the door (no arbitrary teleport).

Decisions made during the code-review / QA follow-up (after implementation):

18. **Grounded-gated re-arm (QA M1):** `request_room_change` re-arms
    (`room_change_armed = true`) only when the predicted body is outside every
    band **and** `world.player.grounded`. Without this, a jump out of the entry
    band re-arms mid-air and re-entering the band on descent sends a reverse
    request, bouncing the player back to the previous room.
19. **Bounds guard on room adoption (QA m1):** `apply_room_change_local`
    early-returns when the server-reported room is outside `levels_matrix`
    bounds, so a bogus room cannot index-panic the client on the next frame.
20. **Cooldown value (QA m5):** `ROOM_CHANGE_COOLDOWN_TICKS` was 12 (~200 ms);
    tuned to 6 (~100 ms) to reduce the immediate-backtrack stutter.
21. **Rate-limit on rejection (QA m2):** rejections arm the cooldown and an
    active cooldown returns `None` (no response), closing the response-
    amplification hole where a modified client spams invalid requests.

Known limitation (QA M2, recorded not fixed): the 8 px lead tolerance
(`DOOR_LEAD_TOLERANCE_PX`) absorbs only ~1.2 ticks (~40 ms RTT) of prediction
lead. On the internet the first request can still reject (self-heals via
`ROOM_CHANGE_RETRY_TICKS`), and past ~75 ms RTT the player visibly stalls at
the door wall. This is out of scope for this spec; see Follow-up.

## Gotchas (for the implementer / planner)

- **The `game/src/maps.rs` comments name the wrong rooms.** The comment
  `// Level (0,1)` sits above `LVL_0_1`, but the matrix is `[row_y][col_x]`, so
  `LVL_0_1` is `(1,0)` and `LVL_1_0` is `(0,1)`. Trust the matrix and the door
  table in this spec, not those comments.
- **Do not drop `PlayerPos.level_x/level_y`.** Only `PlayerInput` loses the
  room fields. `PlayerPos` is how remote nemeses are placed in a room.
- **`apply_input` has ~20 call sites in `server/src/world.rs` tests**, plus
  `server/src/main.rs` and `e2e/src/harness.rs`. Removing `x, y` is a
  compile-error-driven change, but the test region has two separate
  `#[cfg(test)]` blocks — sweep both.
- **`PlayerInput` literals are scattered** across server tests, client
  logging tests, and the e2e harness. Removing the fields produces compile
  errors at every old literal; fix them all rather than adding a compatibility
  struct.
- **`Rect::overlaps` is strict.** A player whose edge exactly touches a
  trigger boundary is *not* in the band; the next sub-pixel of movement
  enters it. The client tests the raw geometric `trigger`; the server tests
  `server_trigger` (the same overlap rule against the lead-expanded rect). Do
  not expand `door_at` — the tolerance is server validation only.
- **`solid_in_rect(trigger)` is not a valid "trigger is empty" test**: the
  trigger's far edge can share a tile row with the floor (e.g. East row 8
  trigger at `y=320..360` includes row 9 in `solid_in_rect`). Validate the
  trigger **center** tile, or the specific edge tile(s), instead.
- **`protocol` now depends on `game`** for `Direction`. Keep one shared
  `Direction`; do not define a second protocol-local enum. `game` must add
  `serde` for the derive.
- **`Direction` variant order is wire format.** Append the new packet variants
  at the end of `GamePacket`; keep `Direction` as
  `North, South, East, West`.
- **`current_coords` is `(usize, usize)`** on the client while `RoomCoords` is
  `(u8, u8)`. Cast at the boundary (`current_room()` helper); do not change
  the client's matrix indexing type in this spec.
- **The entry position is inside the reverse door's trigger.** Every current
  entry sits in the band of the door that leads back. The client's
  `room_change_armed = false` (re-armed only when grounded and out of every
  band — QA M1) and the server's cooldown are both required; without them an
  honest client would bounce back on a jump and a malicious one could ping-pong.
- **`apply_room_change` must preserve `pending` input.** Unlike `respawn`, it
  must not reset `pending`, `shoot_cooldown`, or `max_seq`; the player is
  still moving through the door.
- **Recompute `grounded` at the entry.** Leaving a stale `grounded = true`
  from the old room lets a jump edge fire on the first tick after a vertical
  entry.
- **Own `PlayerPos` room adoption is mandatory**, but gate it on the packet's
  `last_seq` (`game::is_newer_seq`) so a stale pre-transition `PlayerPos`
  arriving after the reliable response cannot drag the client back to the old
  room. The response is on a reliable stream and `PlayerPos` on an unreliable
  one; either can arrive first. Do not reintroduce the old `coords_match`
  skip.
- **Clearing the input buffer on a room change** prevents replaying old-room
  inputs against the new level. On a rejection, clear it too so the snap-back
  is not immediately undone. `load_current_level` itself does not clear the
  buffer; do it in `apply_room_change_local` / the reject path.
- **`e2e` Step 3 must respond to the requesting client only.** The current
  loop is `for rx in &self.from_clients` without an index; use
  `enumerate()` (or a paired loop) so the `RoomChangeResponse` goes into the
  matching `to_clients[i]`. A broadcast would make every client adopt another
  player's room.
- **Do not send `RoomChangeResponse` to every address** in `server/src/main.rs`
  either; it is a per-client ack.
- **`force_teleport` is not a client-callable path.** It is `pub` for
  join/respawn/test setup only; there is no packet that reaches it.
- **Door detection is gated on the predicted position; the server validates
  with a lead tolerance.** `request_room_change` builds the rect from
  `world.player.pos` against the raw geometric `trigger`, so the `e2e` tests
  that previously staged only the server position (b3/b4/b7/b8) must also stage
  the client position at the door. `apply_room_change` validates the
  authoritative rect against `Door::server_trigger()` (the geometric band grown
  by `DOOR_LEAD_TOLERANCE_PX` on the approach edge). The deterministic harness's
  ~1-tick prediction lead is therefore accepted on the first request. Lead
  beyond the tolerance can still reject → snap back → retry after
  `ROOM_CHANGE_RETRY_TICKS`; that is acceptable and out of scope (lag
  compensation is deferred).
- **`docs/adr/0004-*.md` is recorded after implementation**, not part of the
  code task itself.

## Follow-up

- Record `docs/adr/0004-authoritative-room-membership.md` after implementation
  (door graph, request/ack, server teleport, cooldown).
- **Known limitation (QA M2): door transitions under real network latency.** The
  8 px lead tolerance absorbs ~40 ms RTT; beyond that the first request rejects
  (self-heals), and past ~75 ms RTT the player visibly stalls at the door wall.
  A future fix is a server-side request grace window (accept once the
  authoritative body reaches the band within N ticks) plus a configurable
  harness one-way-delay knob so this is testable. Until then, walk-through feel
  is a loopback/LAN property.
- `feat/lag-compensation` (deferred): rewind victim positions to the shooter's
  render time. This spec resolves hits against current authoritative positions.
- `feat/world-snapshot-batching` (deferred): one snapshot per tick instead of
  O(n²) `PlayerPos`.
- Checkpoint/room-local respawns and spawn protection (gameplay, separate
  spec). The current global `(0,0)` respawn is intentional here.
- Optional: room-filter `PlayerDied`/`PlayerPos` broadcasts; not needed for
  authority, and explicitly out of scope.
- Optional: transition fade/sound; the room change is currently instantaneous
  once the ack arrives.
- If a future map has two doors in the same direction from one room, extend
  the request with a door id (or make `door_at` return an id) and keep
  `can_enter` unambiguous.
