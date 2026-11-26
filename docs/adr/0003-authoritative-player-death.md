# 0003. Player death is server-authoritative

- Status: Accepted
- Date: 2026-11-26

## Context

After spec 1 (`refactor/remove-pve-enemies`) removed the PvE `Enemy` machinery,
"shoot" became inert (a `BulletSpawn` with no hit target) and death was a
**client self-report**: the dying client collided bullets locally, sent
`GamePacket::Hit { entity_id }`, and the server merely rebroadcast it. That gives
the client authority over its own death — a cheat vector — and leaves no PvP
combat loop.

## Decision

**The server is the sole authority on player death.** When a shot is accepted,
`ServerWorld::update` casts a horizontal ray (`raycast_player`) from the shooter's
authoritative bullet origin and, on a hit, respawns the victim and emits a
`PlayerDied { id }` broadcast. The client no longer self-reports and only respawns
on receiving `PlayerDied` for its own id.

Five durable decisions, recorded here so a future change has a reference point:

1. **Hitscan, not projectile simulation.** Hits are resolved instantaneously in
   the shot's tick against that tick's post-movement positions. Bullets stay
   client-rendered; `BulletSpawn` stays one packet per accepted shot.
2. **`BulletSpawn.x` is the impact x on a hit.** The bullet is spawned at the
   victim's near edge (not the shooter and not the room edge), so it appears at
   the victim; on a miss it spawns at the origin. (Accepted visual artifact: no
   tracer from shooter to victim — a future render pass may add one.)
3. **Respawn is a global, immediate reset.** The server sets `Player::new(id)`
   (pos `(100,100)`, zero velocity, `dir = 1.0`) and `coords = (0,0)`, clears
   `pending`, and leaves `shoot_cooldown`/`max_seq` untouched. No invulnerability
   frames and no respawn timer. (Known feel risk — see Consequences.)
4. **Inbound `PlayerDied` is ignored.** The variant is server→client only; the
   event loop arm is `GamePacket::PlayerDied { .. } => {}` (defense in depth).
5. **No room filtering of the broadcast.** `PlayerDied` is sent to every connected
   address; room-authoritative membership is spec 3's job.

Raycast geometry (the reference implementation): victim-sized (`w×h`) hitbox,
inclusive vertical overlap, `d_far >= 0` front test, `distance = d_near.max(0.0)`,
closest-hit-wins with ascending-id tie-break, self-exclusion, and per-candidate
wall occlusion via `Level::solid_between`.

## Consequences

- **Positive:** player death can no longer be spoofed or suppressed by the victim's
  client; the combat loop is deterministic (sorted shoot requests, deterministic
  bullet-id and hit order); the new `solid_between` helper correctly rejects
  `solid_in_rect` for a 1-px ray and is independently unit-tested.
- **Negative:** the raycast filters by the *victim's client-claimed* `coords`, so a
  modified client can still dodge death by claiming a room nobody else is in (and
  the respawn's `coords = (0,0)` is overwritten by the next inbound `PlayerInput`).
  This is the **spec-3 acceptance criterion** captured in the roadmap. Death also
  has no lag compensation (a moving victim is hit at current position, not the
  shooter's rendered position) — deferred.
- **Deferred (QA follow-ups):** the fixed global spawn + no i-frames makes a
  spawn-kill loop possible at `(0,0)`; room-local/checkpoint spawns and an
  `invuln_until_tick` map are future gameplay work. Dropping a victim's in-flight
  bullets on every death (remote `Nemesis` removal detaches bullet ownership) is a
  cosmetic follow-up. The `raycast_player` `dir` invariant is implicit
  (`Player.dir` is ±1.0); a `dir.signum()` normalization is a cheap hardening.

## Open questions

- Whether PvP with untrusted clients is acceptable until spec 3 lands (owner
  decision — QA recommends promoting spec 3 and documenting the victim-side cheat).
