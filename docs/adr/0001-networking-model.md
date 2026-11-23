# 0001. Networking model

- Status: Proposed
- Date: 2026-11-23

## Context

Multiplayer is core to Rustvania. Currently the client sends its position every
frame over an unreliable channel, and the server rebroadcasts to the other
players. The protocol (`GamePacket`) is duplicated in `server/` and `client/`
and has already drifted (`level_x: usize` vs `u8`, `Hit { id }` vs
`Hit { entity_id }`). There is no fixed tick rate, interpolation, or
reconciliation.

## Decision (proposed)

1. Extract a single shared `protocol/` crate that both server and client depend
   on, so the `GamePacket` type can never drift.
2. Keep movement **client-authoritative** for now (simple, fits the current
   design), but formalize:
   - a fixed send rate (e.g. 20 Hz) instead of every frame;
   - reliable-ordered for join/leave/actions, unreliable for position;
   - linear interpolation of remote players on the client.
3. Defer server-authoritative simulation and lag compensation to a later ADR.

## Consequences

- Positive: single source of truth for the protocol; deterministic tests possible.
- Positive: simpler reasoning about netcode while learning.
- Negative: client-authoritative movement is trivially cheat-prone (acceptable
  for a learning project).
- Negative: snapshot interpolation adds client complexity.

## Open questions

- Tick rate and packet size budget.
- Entity handling across level transitions (partially covered by `level_x`/`level_y`).
