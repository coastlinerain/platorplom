# Phase 2c Bugfixes — Broadcast and room-transition regressions

- Status: draft
- Date: 2026-11-24
- Branch: `fix/phase-2c-regressions`

## Summary

Phase 2c (client prediction + reconciliation + entity interpolation) landed two
regressions that block multiplayer playability. Both were found during manual
testing at the owner's retuned 60 Hz `TICK_HZ`:

1. **Remote players not visible.** Players connect, receive their own `PlayerPos`
   (local movement works), but cannot see other players. Other players' `Nemesis`
   rectangles never appear because the server broadcasts each address only the
   `PlayerPos` matching that address's own id — never cross-players.

2. **Stuck in walls on room transition.** When a player walks past a room
   boundary, the client-side `check_level_transitions` teleports them to the new
   room's entry, but the server's next authoritative `PlayerPos` still carries
   the old-room coordinates. Reconciliation rewinds the player to those
   coordinates in the **new** room's collision grid, which is a solid-tile or
   out-of-bounds position. The player is stuck.

Neither regression is caught by existing unit tests because both require a
running server and multiple clients. The fixes are small (two code changes).
e2e/integration test infrastructure is deferred to a follow-up phase (D3).

## Decisions

### D1. Broadcast: nested loop (all players to all addresses)

**Decision:** restore the nested broadcast loop — outer over destination
addresses, inner over all connected players — so that every address receives a
`PlayerPos` for every player in every tick.

The Phase 2b plan *intended* this nested loop but the implementation accidentally
flattened it: a single `for (&addr, &id) in &addrs` iterates pairs, sending
`PlayerPos { id }` to `addr`, which means address A only ever sees its own
player. The correct form is:

```text
for each destination address:
    for each (player_id, player) in world.players():
        serialize and send PlayerPos{player_id} to destination address
```

This is the simplest fix. The number of packets per tick grows from N to N²
(where N is the number of connected players), but at a handful of players and
60 Hz this is negligible.

**Rejected alternative:** a separate "remote positions" packet sent only to
addresses that do not own each player. This adds branching and a new packet
type for no real benefit — the own-player echo is needed for reconciliation
anyway, and combining both in one loop is the standard approach.

### D2. Reconciliation guard: skip when server coords ≠ client coords

**Decision:** in `handle_packet`, when the own-player `PlayerPos` arrives, if the
server-reported `level_x`/`level_y` do not match the client's `current_coords`,
**skip reconciliation entirely** — do not rewind, do not replay. The client's
position (post-teleport by `check_level_transitions`) is more correct than the
server's at that instant.

The reconciliation resumes automatically on the next tick once the server catches
up (the server receives the client's `PlayerInput.level_x`/`level_y` in the new
room, updates its `coords` map, and begins simulating from the entry position sent
by `check_level_transitions` on the client side... wait — the server does NOT
receive the entry position. The server's `player.pos` is still at the old boundary
position, and it will stay there until the client's inputs move it. This means
there is a multi-tick window where the server position is wrong for the new room.

**Correction to the plan draft:** the guard alone is insufficient because the
server never teleports the player — it only updates `coords` when it receives
`PlayerInput` with the new room indices, but the server's `player.pos` is at
the old-room boundary and will remain there until client inputs gradually walk it
into valid new-room territory. During this window (which may be indefinite if the
new room entry is far from the old position), every `PlayerPos` carries a stale
x/y in the new room's coordinate space.

**The guard must remain in effect until one of these conditions is met:**

1. The server-reported `level_x`/`level_y` matches the client's
   `current_coords`, **AND**
2. The server-reported `(x, y)` is **within the collision bounds** of the
   client's current room AND **not inside a solid tile**.

Condition (2) prevents the player from being rewound to a wall position when the
server finally catches up on coords but still reports a boundary position that
happens to be inside a solid tile in the new room.

When both conditions are met, reconciliation engages normally: rewind, replay
unacknowledged inputs, and continue.

**Rejected alternative:** make the server authoritative for room transitions
(move `check_level_transitions` logic into the server's tick loop). This is the
correct long-term fix but requires the server to run collision-aware room
transition checks and breaks the D9 scope boundary from the 2b/2c spec. It
belongs in a future phase alongside server-authoritative enemies and bullets.

**Rejected alternative:** have the client send its post-teleport position to the
server in a new packet. This re-introduces client-authoritative position claims
and undermines the server-authoritative model. The guard approach is a
pragmatic, local, reversible fix.

**Rejected alternative:** always skip reconciliation during the coords-mismatch
window. This is simpler but means that when the server's coords match again but
its position is still at the old room's boundary (inside a solid tile in the new
room), the player gets immediately stuck. The solid-tile check prevents this.

### D3. e2e/integration test infrastructure: deferred

**Decision:** this phase does **not** add multi-client integration test
infrastructure. Neither bug is unit-testable in isolation (both require a running
server + two clients). The fixes are verified by manual smoke test (see
Acceptance criteria). A separate follow-up phase will add:

- A headless server that runs in the same process as a test.
- A headless "fake client" that sends inputs and reads state.
- At minimum: a test that two fake clients can see each other's positions.
- A test that room-transition teleport correctness is maintained.

The absence of such tests directly allowed these regressions to land undetected,
so the follow-up phase is high-priority.

## Goals / non-goals

### Goals

1. Two players connected to the same server see each other's `Nemesis` rectangles
   on screen (Bug 1 fixed).
2. A player crossing a room boundary appears at the new room's entry and is not
   stuck in a wall (Bug 2 fixed).
3. All existing quality gates pass: `cargo build`, `cargo test`, `cargo clippy`,
   `cargo fmt`.
4. A manual smoke test (two clients, one server, room transition) passes.

### Non-goals

- No new protocol types or packet variants.
- No server-side room transition logic (stays client-authoritative, scope
  boundary unchanged from 2b/2c).
- No e2e/integration test infrastructure (D3).
- No tick rate changes (`TICK_HZ` stays at 60 as the owner retuned).
- No changes to enemy, bullet, or death handling.

## Behavior

### Bug 1 — Server broadcast

**Current behavior (broken):** server loop iterates `for (&addr, &id) in &addrs`
and sends `PlayerPos{id}` only to `addr`. Player A receives only its own
position. Player B receives only its own position. No cross-player visibility.

**Fixed behavior:** server iterates `for (&dest_addr, _) in &addrs` (outer),
then `for (&player_id, player) in world.players()` (inner), sending
`PlayerPos{player_id}` to `dest_addr`. Every connected address receives every
player's `PlayerPos` every tick.

**Edge case — self:** a player receives its own `PlayerPos` as it always did.
The own-player reconciliation path in `handle_packet` is unchanged by the
broadcast fix.

**Edge case — join sequence:** a newly-joined player receives existing players'
positions in the first tick after join. Existing players receive the new player's
position in the same tick. This is the same timing as the Phase 2b broadcast
behavior, just now working for all players instead of only the self address.

### Bug 2 — Room transition reconciliation guard

**Current behavior (broken):** on room transition, `check_level_transitions`
teleports the player (e.g., from x=790 in room 0 to x=10 in room 1) and sends
`PlayerInput { level_x: 1, level_y: 0 }`. The server updates `coords[id] = (1,0)`
but `player.pos` stays at ~(790, y). The server broadcasts `PlayerPos { x: 790,
level_x: 1 }`. Reconciliation rewinds to (790, y) in room 1's collision grid —
inside a solid tile. Player stuck.

**Fixed behavior:** in `handle_packet` (own-player `PlayerPos` branch), before
calling `prediction::reconcile`, check:

1. `server_level_x as usize == world.current_coords.0` **AND**
   `server_level_y as usize == world.current_coords.1`
2. **AND** the server position `(x, y)` is within the **bounds** of the client's
   current room's collision level AND is **not inside a solid tile**.

If either check fails, skip reconciliation (do not rewind, do not replay). The
`PlayerPos` is stale because the server position is in a different room's
coordinate space or inside a solid tile.

If both checks pass, proceed with normal reconciliation.

**Collision check implementation:** use `Level::colision(x, y, player_w, player_h)`
on the client's current room's `Level`. If `colision` returns `true` (solid), skip
reconciliation. If `false` (empty/air), proceed. Also check that `x` and `y` are
within `[0, pixel_width]` and `[0, pixel_height]` (bounds guard — `solid_point`
returns `true` for out-of-bounds, which double-covers this).

**Edge case — coords re-match before position is valid:** eventually the server's
`PlayerInput` processing catches up (the server sets `coords` to the new room
within one tick), but `player.pos` may still be at the old boundary x=790. The
solid-tile check catches this: x=790 in a 480px-wide room is out of bounds (or
inside a solid tile), so reconciliation stays skipped. The client continues
predicting locally. Once the client's inputs through the server move
`player.pos` into valid new-room territory, the guard lifts and reconciliation
engages.

**Edge case — server pos is in valid territory of new room:** if the room
transition involves a very short room (e.g., the old boundary position x=790
happens to be within the new room's bounds and not solid), the guard lifts
immediately and reconciliation engages. This is rare but harmless — the server
position is technically "wrong" but legally within the new room, so the player
is rewound and then replays unacknowledged inputs from there.

**Edge case — entering a room where the entry is inside a solid tile:** the
client's `check_level_transitions` teleports the player to a hard-coded entry
position (e.g., x=10, y=100). If that position is inside a solid tile in the
new room, `check_level_transitions` itself is buggy (map design issue), not a
reconciliation problem. The player's local `Player::update` would push them out
of the solid tile. The guard's solid-tile check would prevent the server's
position from overriding the client's corrected position.

**Edge case — multiple room transitions in rapid succession:** the guard
evaluates each `PlayerPos` independently. If the coords don't match, skip. If
they match but the position is solid, skip. Only when both checks pass does
reconciliation engage. This handles chained transitions correctly.

## Data model

No new types. No type changes. Two existing code paths are modified.

### `server/src/main.rs` — broadcast loop

The loop at lines 134–149 is changed from a single loop to a nested loop.

**Before (broken):**
```rust
for (&addr, &id) in &addrs {
    if let Some(player) = world.players().get(&id) {
        // sends PlayerPos{id} to addr
    }
}
```

**After (fixed):**
```rust
for (&dest_addr, _) in &addrs {
    for (&player_id, player) in world.players() {
        let (lx, ly) = world.coords().get(&player_id).copied().unwrap_or((0, 0));
        let last_seq = world.max_seq().get(&player_id).copied().unwrap_or(0);
        let bytes = postcard::to_allocvec(&GamePacket::PlayerPos {
            id: player_id,
            x: player.pos.x,
            y: player.pos.y,
            dir: player.dir,
            level_x: lx,
            level_y: ly,
            last_seq,
        }).unwrap();
        let _ = packet_sender.send(Packet::unreliable(dest_addr, bytes));
    }
}
```

The `shots` rebroadcast loop (lines 152–168) is **unchanged** — it already
uses a correct nested loop pattern (`for &(shooter_id, dir) in &shots` outer,
`for (&addr, &player_id) in &addrs` inner with `continue` for the shooter).

### `client/src/network/world.rs` — `handle_packet`, own-player branch

Add a coords + collision guard before the reconciliation call at lines 88–100.

**Current:**
```rust
if Some(id) == world.id {
    let level = &world.levels_matrix[world.current_coords.1][world.current_coords.0];
    prediction::reconcile(
        &mut world.player,
        &mut world.input_buffer,
        &mut world.last_acked_seq,
        x, y, dir, last_seq,
        level,
    );
}
```

**After (fixed):**
```rust
if Some(id) == world.id {
    // Guard: skip reconciliation if the server position is in a different
    // room's coordinate space or inside a solid tile (Bug 2 fix — room
    // transitions are client-authoritative, so the server's position lags
    // behind the client's teleport).
    let coords_match = level_x as usize == world.current_coords.0
        && level_y as usize == world.current_coords.1;
    let level = &world.levels_matrix[world.current_coords.1][world.current_coords.0];
    let in_bounds = x >= 0.0
        && x < level.pixel_width()
        && y >= 0.0
        && y < level.pixel_height();
    let not_in_wall = in_bounds && !level.colision(x, y, world.player.w, world.player.h);

    if coords_match && not_in_wall {
        prediction::reconcile(
            &mut world.player,
            &mut world.input_buffer,
            &mut world.last_acked_seq,
            x, y, dir, last_seq,
            level,
        );
    }
    // else: skip — server position is stale (different room or inside wall).
}
```

No other files change. No new types, constants, or protocol variants.

## Network

No protocol changes. No new or modified packet variants. No channel changes.

The **packet count** per tick increases from N to N² (N = connected players) due
to the nested broadcast loop (D1). At typical player counts (2–4), total
bandwidth remains negligible: ~19 bytes per `PlayerPos` (postcard) × 4 players ×
4 destinations = ~304 bytes per tick at 60 Hz = ~18 KB/s. Well within LAN bounds.

## Acceptance criteria

### Bug 1 — Remote player visibility

1. Two players connected to the same server see each other's `Nemesis` rectangles
   on screen (orange rectangles, drawn at the remote player's interpolated
   position).
2. When a third player joins, all three players see each other's rectangles.
3. When a player disconnects, the `Leave` packet removes the remote `Nemesis`
   (existing behavior, verified not broken).

### Bug 2 — Room transition

4. A player walking past a room's right boundary (x > pixel_width - 5) appears at
   the new room's left entry (x ≈ 10) and can move freely in all directions —
   not stuck.
5. A player walking past a room's left boundary (x < -5) appears at the new
   room's right entry and can move freely.
6. A player falling past a room's bottom boundary (y > pixel_height - 5) appears
   at the new room's top entry and can move freely.
7. After the server catches up (coords match, position is in valid territory),
   normal reconciliation engages — the player's position is corrected by
   authoritative `PlayerPos` packets as usual.

### Quality gates

8. `cargo build --workspace` succeeds with no new warnings.
9. `cargo test --workspace` passes (all existing tests green — 73 as of last
   count, or however many exist after any intervening work).
10. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
11. `cargo fmt --check --all` reports no diffs.

### Manual smoke test (required before merge)

12. Start server: `cargo run -p metroidvania-server`
13. Start two clients: `cargo run -p metroidvania-rust` (×2 in separate
    terminals/windows).
14. **Bug 1:** both clients see each other's orange `Nemesis` rectangles moving
    on screen.
15. **Bug 2:** navigate one player through a room boundary (walk right until the
    level changes). The player appears at the new room's left edge, is not stuck
    in a wall, and can move freely in all directions.
16. Repeat the room transition in the opposite direction (walk left back to the
    original room). The player appears at the original room's right edge and can
    move freely.

### Non-regression

17. Own-player movement remains responsive (local prediction).
18. Entity interpolation for remote players remains smooth.
19. Reconciliation works normally when coords match and position is valid (the
    common case — most game time is spent stationary in one room).
20. `Nemesis` shoot bullets (`Action { kind: "shoot" }`) still appear in the
    correct room (existing behavior, verified not broken by the broadcast fix).