# Issue: Door transitions stall under real network latency (8 px lead tolerance)

- **Status**: open
- **Type**: netcode
- **Severity**: major
- **Created**: 2026-11-26
- **Closed**: —
- **Related**: spec 3 (`feat/authoritative-room-membership`), ADR-0004, QA M2

## Description

Spec 3 made room membership server-authoritative. The client requests a room
change when its **predicted** body overlaps a door's geometric trigger band, and
the server validates the request against its own (lagging) **authoritative**
body using `Door::server_trigger()` — the geometric band grown by
`DOOR_LEAD_TOLERANCE_PX = 8.0` on the approach edge.

8 px ≈ 1.2 ticks ≈ **~20 ms one-way / ~40 ms RTT** of prediction lead. Beyond
that:

- the first `RequestRoomChange` is rejected (the authoritative body is still
  short of the band), the client snaps back and retries after
  `ROOM_CHANGE_RETRY_TICKS = 6` (self-heals on the second attempt);
- separately, the client no longer teleports locally on a transition, and the
  canonical entry is ~30 px (~75 ms at `VELOCIDAD = 400`) inside the target
  room, so once RTT > ~75 ms the player visibly stops against the door wall and
  pops one-way-latency later.

The "instant walk-through" feel is therefore a **loopback/LAN-only** property.
The e2e harness (`e2e/src/harness.rs`) has no delay/jitter knob, so E6
(`e2e_walk_in_first_request_accepted`) only proves the deterministic ~1-tick
lead case, not a real-latency walk-through.

## Root Cause

- The server validates against a fixed 8 px lead tolerance that is sized for
  one prediction tick, not for a configurable network RTT.
- The harness has no one-way-delay simulation, so real-latency behavior is
  untested.

## Impact (if not fixed)

- On the internet, every doorway stutters (first-request reject → snap-back →
  retry), and past ~75 ms RTT the player stalls at the door wall.
- The unkillable-victim cheat remains closed regardless — this is a *feel*
  regression, not an authority hole.

## Fix Plan

- [ ] Add a server-side **request grace window**: on a rejected-solely-by-lead
      request, remember the `(from, direction)` for N ticks and accept once the
      authoritative body actually reaches the band. Authority is preserved — the
      player still has to walk there.
- [ ] Add a configurable one-way-delay knob to `e2e/src/harness.rs` (delay/jitter
      on the `from_clients` → server path, or the server → `to_clients` path) so
      real-latency walk-throughs are testable.
- [ ] Add an e2e test: "walk-through under 3 ticks of one-way delay yields ≤1
      reject and reaches the target room".
- [ ] Re-tune `DOOR_LEAD_TOLERANCE_PX` (or replace it with the grace window) once
      the intended RTT envelope is decided.

## Verification

- [ ] cargo test --workspace
- [ ] cargo clippy --all-targets --all-features --workspace -- -D warnings
- [ ] cargo fmt --check --all

## Commits

- (none yet)

## Lessons Learned

- A fixed lead tolerance is a *loopback* approximation, not a latency model; a
  "first request accepted" e2e test with no delay knob proves only the
  deterministic single-tick case.
- Recording a known limitation as an issue (rather than silently accepting it)
  keeps the feel regression visible and scopes the fix for a later spec.
