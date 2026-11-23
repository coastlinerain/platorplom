# 0001. Networking model

- Status: Accepted
- Date: 2026-11-23

## Context

Multiplayer is core to Rustvania. It starts as co-op PvE (trusted friends) but is
planned to grow into PvP and public servers, and the owner wants to learn proper
netcode. The current design is client-authoritative: the client sends its position
every frame over an unreliable channel and the server merely rebroadcasts. This is
cheat-prone and would become tech debt for PvP.

## Decision

**Server-authoritative movement.**

- The client sends **inputs** (not positions); the server simulates movement and
  collision at a fixed tick and broadcasts authoritative state back.
- **Fixed tick rate**: 20 Hz, expressed as a named constant (`TICK_HZ`), so a later
  move to 30/60 Hz is a one-line change — no hardcoded `20.0` scattered in the code.
- **Channels**: reliable-ordered for join/leave/actions/hit (state-changing),
  unreliable for position broadcasts (latest-wins).
- **Client prediction + reconciliation**: added in a later step to hide the
  round-trip latency of server-authoritative movement.

## Roadmap

1. **2a — Extract a shared `game` core crate**: move level/physics/collision/
   entities into a library both server and client depend on. (Prerequisite: the
   server cannot simulate without the same simulation code the client uses.)
2. **2b — Server-authoritative movement**: server tick loop (20 Hz) applies
   inputs → positions → broadcasts; client sends inputs only.
3. **2c — Client prediction + reconciliation**: client predicts its own movement
   locally and reconciles when authoritative state arrives.

## Consequences

- Positive: anti-cheat (server validates); correct foundation for PvP and public
  servers; the `game` crate also makes game logic unit-testable.
- Negative: significantly more complex (server simulation loop, prediction +
  reconciliation); higher round-trip latency until 2c lands.
- Tick rate is configurable, so tuning 20→60 Hz later is trivial.

## Open questions

- Packet size budget for position broadcasts.
- Entity handling across level transitions (partially covered by `level_x`/`level_y`).
