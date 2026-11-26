# Phase 2c Bugfixes — Implementation Plan

- Status: draft
- Spec: `docs/specs/phase-2c-bugfixes.md`
- Date: 2026-11-24
- Branch: `fix/phase-2c-regressions`

## Overview

Phase 2c (client prediction + reconciliation + entity interpolation) landed with
two multiplayer-blocking regressions, both found during manual testing at the
owner's retuned 60 Hz `TICK_HZ`:

### Bug 1 — Remote players invisible

`server/src/main.rs` lines 134–149 iterate `for (&addr, &id) in &addrs`, sending
only each address's own `PlayerPos`. The Phase 2b plan intended a nested loop
(all players to all addresses), but the implementation accidentally flattened it.
Client A never receives client B's position — the orange `Nemesis` rectangle never
appears.

**Fix:** restore the nested loop — outer over destination addresses, inner over
all connected players — so every address receives every player's `PlayerPos` every
tick.

### Bug 2 — Stuck in walls on room transition

When the client's `check_level_transitions` teleports the player to a new room's
entry, the server's next authoritative `PlayerPos` carries the **old** room's
boundary position (e.g. x=790) but with the **new** room's `level_x`/`level_y`.
Reconciliation rewinds the player to that position in the new room's collision
grid — a solid-tile or out-of-bounds position. The player is stuck.

The server catches up on `coords` within one tick (when it processes the client's
`PlayerInput` with the new room indices), but the server's `player.pos` is never
teleported (room transitions are client-authoritative per D9 scope boundary).
The server position remains at the old-room boundary until client inputs gradually
move it into valid new-room territory — a window that can be indefinite.

**Fix:** guard reconciliation in `client/src/network/world.rs` `handle_packet`
with **two** conditions that must both be true before reconciliation engages:

1. Server-reported `level_x`/`level_y` match client `current_coords`, **AND**
2. The server position `(x, y)` is within the **bounds** of the client's current
   room's collision level AND is **not inside a solid tile**
   (`!level.colision(x, y, player.w, player.h)`).

If either check fails, skip reconciliation entirely — do not rewind, do not
replay. The client's predicted position is more correct than the server's at
that instant.

### Why no automated tests catch these

Both bugs require a running server and multiple clients — they are not
unit-testable in isolation. e2e/integration test infrastructure is deferred to
a follow-up phase (D3). Verification is by manual smoke test.

### Packet count change

The nested broadcast loop increases per-tick packets from N to N²
(N = connected players). At 2–4 players: ~304 bytes/tick at 60 Hz = ~18 KB/s —
negligible for LAN.

---

## Public API (targets for the tester)

**None.** No new types, functions, structs, or protocol variants. The fixes are
internal changes to existing code paths:

- `server/src/main.rs`: loop structure change (single → nested).
- `client/src/network/world.rs`: guard condition added before calling
  `prediction::reconcile`.

---

## Test cases (tester writes these first)

**No new automated tests this phase.** Both bugs require a running server + two
clients, and the infrastructure for headless multi-client testing is deferred
(D3). The existing 73 tests remain the automated safety net.

### Manual smoke test (required before merge)

This is the verification. The implementer must perform it before checking the
final box.

| # | Step | Expected |
|---|---|---|
| MS1 | Start server: `cargo run -p metroidvania-server` | Server prints listening address |
| MS2 | Start client 1: `cargo run -p metroidvania-rust` | Connects, player visible |
| MS3 | Start client 2: `cargo run -p metroidvania-rust` | Connects, player visible |
| MS4 | **Bug 1:** both clients see each other's orange `Nemesis` rectangles moving on screen | Two orange rectangles visible, tracking each other's movement |
| MS5 | **Bug 2:** navigate player 1 right through a room boundary | Player appears at new room's left entry, is **not** stuck in a wall, can move freely in all directions |
| MS6 | Navigate player 1 left back through the boundary to the original room | Player appears at original room's right entry, can move freely |
| MS7 | Navigate player 1 down through a bottom boundary | Player appears at new room's top entry, can move freely |

### Non-regression checks (manual)

| # | Behavior | Expected |
|---|---|---|
| NR1 | Own-player movement | Responsive (local prediction working) |
| NR2 | Remote player movement | Smooth (entity interpolation working) |
| NR3 | Local jump (Space) | Works immediately on keypress |
| NR4 | Local shoot (Z) | Bullet spawns, moves, collides with enemies |
| NR5 | `Nemesis` shoot bullets | Appear in the correct room (broadcast fix shouldn't break this) |

---

## Implementation checklist

Each box is a small, independently verifiable step in dependency order. The
`implementer` fills the `Explain` fields.

---

### Box 1 — Restore nested broadcast loop (`server/src/main.rs`)

- [ ] done
- **Acceptance**: `server/src/main.rs` lines 134–149 changed from a single
  `for (&addr, &id) in &addrs` loop to a nested loop — outer over destination
  addresses `for (&dest_addr, _) in &addrs`, inner over all players
  `for (&player_id, player) in world.players()`. Each inner iteration serializes
  and sends `PlayerPos { id: player_id, ... }` to `dest_addr`.
  `cargo build -p metroidvania-server` compiles with no new warnings.
- **Test**: Manual MS4 (two clients see each other). Code-read:
  `grep "for.*&addr.*&id.*in.*&addrs" server/src/main.rs` → zero matches.
  `grep "for.*&dest_addr.*in.*&addrs" server/src/main.rs` → one match (new outer).
  `grep "for.*world.players()" server/src/main.rs` → one match (new inner,
  inside the outer loop body).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 2 — Add coords + solid-tile guard before reconciliation (`client/src/network/world.rs`)

- [ ] done
- **Acceptance**: In `handle_packet` `PlayerPos` own-player branch (around line
  88), before calling `prediction::reconcile`, the code checks:
  1. `level_x as usize == world.current_coords.0 && level_y as usize == world.current_coords.1`
  2. `x >= 0.0 && x < level.pixel_width() && y >= 0.0 && y < level.pixel_height()`
  3. `!level.colision(x, y, world.player.w, world.player.h)`
  `prediction::reconcile` is only called when **all three** are true. Otherwise
  the `PlayerPos` is silently skipped (stale position from a different room's
  coordinate space, or inside a solid tile in the current room).
  The `level` binding is reused from the variable already in scope at that point
  (`world.levels_matrix[world.current_coords.1][world.current_coords.0]`).
  `cargo build -p metroidvania-rust` compiles with no new warnings.
- **Test**: Manual MS5, MS6, MS7 (room transitions in all directions — player
  not stuck). Code-read:
  `grep "current_coords.0.*level_x\|level_x.*current_coords.0" client/src/network/world.rs` →
  at least one match in the guard condition.
  `grep "colision" client/src/network/world.rs` → at least one match in the
  guard condition.
  `grep "prediction::reconcile" client/src/network/world.rs` → the call is
  inside an `if coords_match && not_in_wall { ... }` block.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

### Box 3 — Quality gates + manual smoke test

- [ ] done
- **Acceptance**:
  1. `cargo build --workspace` succeeds with no new warnings.
  2. `cargo test --workspace` passes (all existing 73 tests green, or however
     many exist after any intervening work).
  3. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
  4. `cargo fmt --check --all` reports no diffs.
  5. Manual smoke test (MS1–MS7) passes with two clients and one server.
- **Test**: All quality gate commands exit 0. Manual smoke test checklist (MS1–MS7
  + NR1–NR5) verified by visual inspection.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

---

## Open questions

None. The spec resolves all decisions:

- **D1:** nested broadcast loop (all players to all addresses), N² packets per tick.
- **D2:** coords-match **AND** solid-tile guard before reconciliation (not just
  coords-match — the draft plan's simpler guard is insufficient per the spec's
  root-cause analysis of the server position lag).
- **D3:** e2e/integration test infrastructure deferred to a follow-up phase.