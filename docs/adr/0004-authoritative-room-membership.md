# 0004. Room membership is server-authoritative

- Status: Accepted
- Date: 2026-11-26

## Context

Before spec 3, room membership was **client-authoritative**: the client detected a
room edge in `check_level_transitions`, mutated `current_coords`, and sent
`level_x`/`level_y` inside every `PlayerInput`; the server's `apply_input`
blindly stored them. That gave a modified client authority over which room it
was in. Spec 2's QA found the consequence (MAJOR-2): `raycast_player` filters
candidates by `coords`, so a victim could claim an empty room and become
unkillable while still shooting others. The respawn's `coords = (0,0)` was also
overwritten by the next inbound input, so death did not even guarantee a return
to room `(0,0)`.

## Decision

**The server is the sole authority on room membership for both shooter and
victim.** The client may only *ask to use a direction*; the server validates the
request against its authoritative body, the shared door graph, and a per-player
cooldown, then teleports to a canonical entry.

Durable decisions, recorded so a future change has a reference point:

1. **Door graph in `game`.** `game/src/room_graph.rs` holds `Direction`,
   `Door`, `RoomCoords`, and `RoomGraph` with the current map's eight directed
   doors (one per `(from, direction)`, in table order). `Direction` is
   `North, South, East, West` and that order is the postcard wire format.
2. **Direction requests, never target rooms.** `GamePacket::PlayerInput` lost
   `level_x`/`level_y`; `RequestRoomChange { id, direction }` is appended after
   `BulletSpawn` (index stability). There is no packet field through which a
   client can name a room. `PlayerPos` keeps its room fields for nemesis
   rendering.
3. **Explicit ack.** `RoomChangeResponse { accepted, level_x, level_y, x, y }`
   is sent to the **requesting address only**, reliable-ordered. It carries the
   canonical entry on acceptance and the unchanged authoritative state on
   rejection; the client applies or snaps back from it.
4. **Server teleport on acceptance.** `ServerWorld::apply_room_change` sets
   `coords`, moves `pos` to the door entry, zeroes velocity, and recomputes
   `grounded` in the target room. It **preserves** `dir`, `bullets`, `pending`,
   `shoot_cooldown`, and `max_seq` — unlike `respawn`.
5. **`coords` has a closed set of writers.** `spawn`, `remove`, `respawn`,
   `force_teleport`, and `apply_room_change`. `apply_input` no longer touches it.
   `respawn`/join use `force_teleport` semantics and bypass doors, so a death is
   never rejectable.
6. **`force_teleport(id, room, pos)` bypasses `can_enter` and the cooldown.** It
   is `pub` for join/respawn/test setup only; no packet reaches it. It no-ops for
   an unspawned id.
7. **Anti-ping-pong = client disarm + server cooldown.**
   `ROOM_CHANGE_COOLDOWN_TICKS = 12` applies to *accepted* changes only; the
   client sets `room_change_armed = false` after an accepted transition and
   re-arms only after its body leaves every trigger band. Every current entry
   sits inside the reverse door's band, so both halves are required.
8. **Server-only lead tolerance.** `can_enter` validates against
   `Door::server_trigger()`: the geometric `trigger` grown by
   `DOOR_LEAD_TOLERANCE_PX = 8.0` on the approach edge (East: left, West: right,
   South: top, North: bottom). The client's `door_at` detection uses the raw
   `trigger`. The client predicts ~one tick ahead (~6.7 px), so the expansion
   accepts an honest walk-in's first request without granting an arbitrary
   teleport. Both sides derive "at the door" from the same data.
9. **Own `PlayerPos` is authoritative for room adoption too.** The reliable ack
   and the unreliable `PlayerPos` can arrive in either order; the client adopts a
   new room from its own `PlayerPos` only when
   `game::is_newer_seq(last_seq, last_acked_seq)`, so a stale pre-transition
   packet cannot drag it back. The old `coords_match` reconciliation skip is
   gone; the bounds and `not_in_wall` guards remain.
10. **Input buffers are cleared on any room change** (accepted or rejected) so
    old-room inputs are not replayed into the new room and a snap-back is not
    undone by replay.

## Consequences

- **Positive:** room membership is no longer spoofable; the spec-2 victim-side
  MAJOR-2 is closed for both roles; the respawn room survives later inputs; door
  geometry is data in one shared crate instead of hardcoded `10.0`/`-5.0`
  heuristics; the client and server share one door model.
- **Negative / deliberate behavior change:** immediate re-crossing is no longer
  possible — after a transition the player is disarmed while their body overlaps
  any band, so they must step out of the entry band before reversing. This is the
  anti-bounce rule; the server cooldown is the malicious-client backstop.
- **No lag compensation, still.** Hits resolve against the current tick's
  authoritative positions; a rejected door request snaps back and retries after
  `ROOM_CHANGE_RETRY_TICKS = 6`. If real-world lead exceeds 8 px, an honest
  client can still be briefly rejected (accepted; lag compensation is a separate
  spec).
- **No wire migration.** Removing `PlayerInput`'s room fields changes that
  variant's payload and both sides are rebuilt from the same workspace.
- **`protocol` now depends on `game`** for the shared `Direction` (no cycle;
  `game` does not know `protocol`). `glam` therefore becomes transitive through
  `game` for `protocol`; the dependency test checks direct edges only
  (`cargo tree --depth 1`).

## Open questions

- If a future map has two doors in the same direction from one room, the request
  needs a door id (or `door_at` must return one) so `can_enter` stays
  unambiguous. Out of scope for the 2×2 map.
- Whether to room-filter `PlayerPos`/`PlayerDied` broadcasts. Not needed for
  authority and explicitly out of scope; `feat/world-snapshot-batching` is the
  natural home.
- A request timeout. Owner decision 4 chose an explicit ack over a timeout; with
  reliable-ordered streams a lost ack is a transport failure, not a normal case.
