# Authoritative player death

- Status: implemented
- Date: 2026-11-26
- Branch: `feat/authoritative-player-death`
- Reference: `docs/specs/netcode-hardening-roadmap.md` (spec 2)

## Summary

Spec 1 (`refactor/remove-pve-enemies`) removed the PvE `Enemy` machinery and left
"shoot" inert: the server still accepts a shot and broadcasts `BulletSpawn`, but
there is no hit-detection target, and death is still a **client self-report**
(`GamePacket::Hit` sent by the dying client, then rebroadcast by the server).

This spec closes that gap for PvP. It makes the server the **sole authority** on
player death:

- rename the protocol packet `Hit { entity_id }` → `PlayerDied { id }`;
- after a shot is accepted, the server casts a horizontal ray from the shooter's
  authoritative bullet origin and hits the **closest other player in the same
  authoritative room**, respecting wall occlusion;
- on a hit, the server respawns the victim, emits `PlayerDied`, and broadcasts it
  to **all** clients;
- the client no longer self-reports death and no longer kills itself on bullet
  contact; it respawns only when `PlayerDied` names its own id.

Room-membership authority (spec 3), lag compensation, and
`PlayerInput.level_x/level_y` removal are explicitly out of scope.

## Goals / non-goals

**Goals**

- Server-side PvP hit detection: `raycast_player` on `ServerWorld` with a
  horizontal ray, a player-sized hitbox, same-room filtering, self exclusion,
  deterministic closest-hit selection, and wall occlusion.
- `ServerWorld::update` returns hits alongside bullet spawns:
  `(Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>)`.
- Server-authoritative respawn: the server resets the victim's `Player` and
  `coords` to the canonical spawn and broadcasts `PlayerDied`.
- Protocol rename `Hit { entity_id }` → `PlayerDied { id }` (in-place, no other
  wire change), with round-trip coverage.
- Client: delete the self-report + client-side death collision; handle
  `PlayerDied` for self (respawn) and for remotes (drop the stale `Nemesis`).
- Update every `update` caller and the e2e harness; add unit and e2e tests.

**Non-goals**

- **No room-membership authority** (spec 3). `coords` stays client-claimed via
  `apply_input`; `level_x`/`level_y` stay in `PlayerInput`; no `RequestRoomChange`,
  no door checks, no snap-back. A cheater can still claim a room.
- **No lag compensation** (deferred). Hits are resolved in the shot's tick
  against the positions produced by that tick's movement pass; the server does
  not rewind.
- **No server-side bullet simulation.** Bullets stay instantaneous hitscan on the
  server and client-rendered projectiles. A `BulletSpawn` is still one packet per
  accepted shot.
- **No invulnerability frames, kill feed, score, death animation, or respawn
  timer.** Death → immediate respawn.
- No room filtering of the death broadcast (spec 3).
- No change to PvE data (`TileType`/`EntityType` variants, map data) or to
  movement/prediction/interpolation behavior.
- No protocol versioning or wire migration (postcard enum, single workspace).

## Behavior

### Authority model

- Only `ServerWorld::update` decides that a player died. It detects the hit and
  returns `PlayerHitInfo`; `server/src/main.rs` turns each one into a
  `PlayerDied` broadcast.
- A client must never send `PlayerDied`. The server receives it only as an
  unknown/ignored packet: the event loop arm is `GamePacket::PlayerDied { .. } =>
  {}` with no state change (defense in depth; the variant is server→client).
- The `Hit` client→server handler (the old self-report path) is deleted.

### Shot resolution (server, same tick as the shoot)

The existing `update` first pass is unchanged: it updates every player, builds
each player's input, consumes the shoot latch, and pushes a `ShootRequest` whose
origin is the shooter's **post-movement** center
`(pos.x + w/2.0, pos.y + h/2.0)` with `dir = player.dir`.

The second pass is replaced. Process `shoot_requests` in **ascending
`player_id`** (sort first — `self.players` is a `HashMap`, so this also makes
bullet-id assignment deterministic). For each request:

1. `let hit = self.raycast_player(req.player_id, req.spawn_x, req.spawn_y, req.dir);`
2. Assign `bullet_id` exactly as today (`next_bullet_id`, `wrapping_add(1)`).
3. Emit exactly one `BulletSpawnInfo`:
   - `x = hit.map_or(req.spawn_x, |(_, hit_x)| hit_x)` — **on a hit the bullet is
     spawned at the impact x, not at the shooter and not at the room edge**
     (owner decision). On a miss it is spawned at the origin, as today.
   - `y = req.spawn_y`, `dir = req.dir`, `owner_id = req.player_id`,
     `seq = req.player_seq` (unchanged).
4. If `hit` is `Some`, append a `PlayerHitInfo` for the victim **unless that
   victim already has one this tick** (first shot wins; at most one death per
   victim per tick).
5. After all requests are resolved, call `self.respawn(victim_id)` for every
   collected hit, then return `(bullet_spawns, player_hits)`.

Consequences that must be stated in code comments:

- All hit tests see the positions produced by this tick's movement pass. Respawns
  are applied only after the loop, so a victim cannot be teleported mid-pass and
  cannot be hit twice in the same tick.
- A victim who was also shooting this tick still fires (their `ShootRequest`
  already exists); death and respawn do not cancel their shot.
- Bullet IDs are now assigned in ascending shooter id order (deterministic).

### Raycast geometry (`ServerWorld::raycast_player`)

Signature (private method, unit-testable inside `server/src/world.rs`):

```rust
fn raycast_player(
    &self,
    shooter_id: u8,
    spawn_x: f32,
    spawn_y: f32,
    dir: f32,
) -> Option<(u8, f32)> // (victim_id, hit_x)
```

Semantics:

- **Direction.** Only the sign matters: `dir >= 0.0` shoots right, `dir < 0.0`
  shoots left. (`Player.dir` is ±1.0 in practice.)
- **Room filter.** The shooter's room is
  `self.coords.get(&shooter_id).copied().unwrap_or((0, 0))`. A candidate is
  considered only if `self.coords.get(&victim_id).copied().unwrap_or((0, 0)) ==
  shooter_room`. The room's `Level` is
  `self.levels[room.1][room.0]`, with the same bounds fallback used by the first
  pass (`levels[0][0]`).
- **Self exclusion.** `victim_id != shooter_id`.
- **Hitbox.** For each other player, center
  `(cx, cy) = (victim.pos.x + victim.w / 2.0, victim.pos.y + victim.h / 2.0)` and
  a player-sized rect centered there:
  `x ∈ [cx - victim.w/2.0, cx + victim.w/2.0]`,
  `y ∈ [cy - victim.h/2.0, cy + victim.h/2.0]`. Today `w = 30.0`, `h = 38.0`;
  use the victim's `w`/`h` rather than a literal 30/38.
- **Vertical overlap (inclusive).** The ray is the horizontal line `y = spawn_y`.
  A candidate is skipped unless `spawn_y >= cy - victim.h/2.0 && spawn_y <= cy +
  victim.h/2.0`.
- **Front/behind.** With `near_edge` = the rect edge facing the shooter and
  `far_edge` = the opposite edge, in ray terms:
  - `d_near = (near_edge - spawn_x) * dir`
  - `d_far  = (far_edge  - spawn_x) * dir`
  - Skip if `d_far < 0.0` (the whole rect is behind the origin).
  - `distance = d_near.max(0.0)` (a shooter overlapping the victim's rect hits at
    distance 0).
  - `hit_x = spawn_x + dir * distance`.
- **Wall occlusion.** Skip the candidate if
  `level.solid_between(spawn_x, hit_x, spawn_y)` is `true` (see below). The wall
  check is per candidate, up to that candidate's `hit_x`; a wall does not stop
  the *scan*, it invalidates candidates behind it.
- **Closest hit wins.** Keep the candidate with the smallest `distance`. Iterate
  victims in **ascending `id`** and replace only on strictly smaller distance, so
  an exact distance tie goes to the lower id. Return `(victim_id, hit_x)` for
  that candidate, or `None` if no candidate survived.

The ray has no explicit max range: a miss simply produces no candidate, and the
`BulletSpawn` stays at the origin (client bullets self-terminate on walls).

### Wall occlusion (`Level::solid_between`)

The existing `Level` API has no point/tile query: `solid_point` is private,
`colision` samples six points and treats out-of-bounds as solid, and
`solid_in_rect(Rect)` walks `y_start..=y_end` — a 1-px-tall rect at
`y = 119.0` (spawn center for the default spawn) spans rows 2 **and 3**
(`TILE_SIZE = 40`, so `floor(120/40) = 3`), which would falsely block the ray on
a wall in the row below. Using `solid_in_rect` for the ray is therefore wrong.

Add one small, focused helper to `game/src/level.rs` (inside `impl Level`):

```rust
/// True if any `TileType::Wall` tile blocks the horizontal segment from
/// `x0` to `x1` (order-insensitive) at height `y`.
pub fn solid_between(&self, x0: f32, x1: f32, y: f32) -> bool
```

Exact semantics:

- `gy = (y / TILE_SIZE).floor() as i32`; if `gy < 0 || gy >= self.height as i32`
  return `false` (out-of-bounds is not solid, matching `solid_in_rect`).
- `let x_min = x0.min(x1); let x_max = x0.max(x1);`
- `gx_start = (x_min / TILE_SIZE).floor() as i32`,
  `gx_end = (x_max / TILE_SIZE).floor() as i32`.
- For `gx in gx_start..=gx_end`: if `gx >= 0 && gx < self.width as i32` and
  `self.grid[gy as usize][gx as usize] == TileType::Wall`, return `true`.
- Otherwise `false`.

This scans only the single tile row containing the ray, includes the origin tile
and the tile containing `x1`, and ignores out-of-grid columns. It is the exact
"walk the room's level grid tiles between origin and hit distance and bail if any
is solid" from the requirement, in one place with its own tests.

### Death, respawn and broadcast

- On a detected hit the server calls `respawn(victim_id)` (private
  `ServerWorld` method):
  - `*self.players.get_mut(&id) = Player::new(id)` — canonical spawn: `pos =
    (100.0, 100.0)`, `vel = (0,0)`, `dir = 1.0`, `grounded = false`, `w/h =
    30/38`, empty bullets.
  - `self.coords.insert(id, (0, 0))`.
  - `self.pending.insert(id, PlayerInput::default())` (drop any held/latched input
    so the first tick after respawn starts clean; the client re-sends its held
    keys on the next input).
  - `shoot_cooldown` and `max_seq` are **not** reset.
- Respawn is immediate (same tick). There is no invulnerability window and no
  respawn delay.
- `server/src/main.rs` broadcasts, for every `PlayerHitInfo`, **one
  `GamePacket::PlayerDied { id: victim_id }` to every connected address,
  including the victim** (reliable-ordered). The broadcast is not room-filtered
  (spec 3's job). Log `"Player {victim} was killed by {shooter}"`.
- Ordering in the tick is: `update` → `PlayerDied` broadcasts → existing
  `PlayerPos` broadcasts (unreliable) → existing `BulletSpawn` broadcasts. The
  respawned position therefore reaches clients in the same tick.

### Client behavior

`client/src/world/lib.rs`:

- **Delete** `check_player_death` and its call in `World::update` (and the import
  in `client/src/world/game.rs`). A remote bullet touching the local player no
  longer teleports or sends anything; `remote_bullets` stays purely visual
  (still simulated, still wall-destroyed).
- **Rename** `handle_death` → `pub fn respawn_local(world: &mut World)` and remove
  the `GamePacket::Hit` send. It keeps the existing reset set (print, `coords =
  (0,0)`, `load_current_level`, bullet-id sets, clock sync, `prev_player_*` snap)
  and additionally rebuilds the player from the canonical spawn:
  `let id = world.player.id; world.player = Player::new(id);` before snapping
  `prev_player_*`. This makes the local state match the server's respawn exactly
  (pos/vel/grounded/dir), so the next authoritative `PlayerPos` reconciles
  cleanly.

`client/src/network/world.rs` — replace the `Hit` arm with:

```rust
GamePacket::PlayerDied { id } if Some(id) == world.id => {
    crate::world::lib::respawn_local(world);
}
GamePacket::PlayerDied { id } => {
    // The server already respawned the victim; drop the stale body/interpolation
    // so nothing is drawn at the death position. The next PlayerPos re-creates
    // the Nemesis at the respawn point.
    world.other_players.remove(&id);
    world.other_interp.remove(&id);
}
```

Notes:

- The old `Hit` arm only reset clock-sync fields for the local player; that reset
  is now subsumed by `respawn_local` (via its own writes and `load_current_level`),
  so no separate clock-reset arm is kept.
- `Nemesis` has no `alive`/health field. "Marking a remote player dead" is
  therefore represented by removing the entry (same handling as `Leave`). Since
  the server respawns in the same tick, the observable result is "the remote
  stops being drawn at the death spot and reappears at spawn on the next
  `PlayerPos`".
- A duplicate `PlayerDied` is harmless: the self path resets to spawn again, the
  remote path is a no-op.

### Per-file surface

#### `protocol/src/lib.rs`

- Replace the `Hit { entity_id: u8 }` variant **in place** (same position in the
  enum, between `Leave` and `PlayerInput`) with `PlayerDied { id: u8 }`.
- No other variant/field changes.

#### `protocol/tests/round_trip.rs`

- Rename `round_trip_hit` → `round_trip_player_died`; round-trip
  `GamePacket::PlayerDied { id: 0 }` and `{ id: 255 }`. No other test changes.

#### `game/src/level.rs`

- Add `pub fn solid_between(&self, x0: f32, x1: f32, y: f32) -> bool` with the
  semantics above, plus unit tests (below).

#### `server/src/world.rs`

- Add `pub struct PlayerHitInfo { pub shooter_id: u8, pub victim_id: u8 }`
  (`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`).
- Change `update` to `-> (Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>)`; rewrite the
  second pass as specified; `self.current_tick += 1` stays; return the tuple.
- Add `fn raycast_player(...)` and `fn respawn(&mut self, id: u8)`.
- Update the stale "next spec re-adds ..." comment.
- Update every `world.update(...)` test call that expects a single return value
  to destructure `let (shots, _hits) = ...` (or `_` for both). Call sites that
  simply discard the return need no change.

#### `server/src/lib.rs`

- `pub use world::{BulletSpawnInfo, PlayerHitInfo, ServerWorld};` (or add a
  separate `pub use world::PlayerHitInfo;`).

#### `server/src/main.rs`

- Rename the `GamePacket::Hit` arms in `packet_type_name` → `"PlayerDied"`,
  `packet_id` → `Some(*id)`, `packet_key_fields` → `json!({ "id": id })`.
- Replace the received `Hit` handling block with
  `GamePacket::PlayerDied { .. } => {}` (ignored; no broadcast, no state change).
- `let (shots, deaths) = world.update(tick_dt);`.
- Add a `for hit in &deaths { ... }` loop that broadcasts
  `GamePacket::PlayerDied { id: hit.victim_id }` to **all** `addrs` via
  `Packet::reliable_ordered`, with `log_packet("sent", ...)`.

#### `client/src/network/transport.rs`

- Both `PacketIO::send` match blocks: remove the `GamePacket::Hit { .. }` arm;
  only `GamePacket::JoinRequest` stays `reliable_ordered`, everything else stays
  `unreliable`. (The client never sends `PlayerDied`.)

#### `client/src/network/logging.rs`

- `packet_type_name` → `"PlayerDied"`; `packet_id` → `Some(*id)`;
  `packet_key_fields` → `json!({ "id": id })`.
- Update the `Hit` assertions in `packet_type_name_all_variants`,
  `packet_id_all_variants`, and `packet_key_fields_all_variants`.

#### `client/src/world/lib.rs`, `client/src/world/game.rs`, `client/src/network/world.rs`

- As described in "Client behavior". Also update the stale `World::update`
  comment about running the transition check after bullet collision, since the
  local death collision no longer exists.

#### `e2e/src/harness.rs`

- Step 4: `let (shots, deaths) = self.server.update(tick_dt);`.
- Track hits: `pub last_deaths: Vec<PlayerHitInfo>` (cleared at the start of each
  `tick` and filled from `deaths`).
- Step 4b (immediately after `world.update`, **before** the Step 5 `PlayerPos`
  loop and Step 6 `BulletSpawn` fan-out, mirroring `main.rs`'s order): for each
  `PlayerHitInfo`, send `GamePacket::PlayerDied { id: victim_id }` to **every**
  client channel.
- Step 3: count client→server death reports — decode each drained packet and
  increment `pub client_death_reports: usize` when it is
  `GamePacket::PlayerDied { .. }` (it should always stay 0).
- Add `pub fn set_server_player_coords(&mut self, handle: ClientHandle, x: u8, y:
  u8)` delegating to `self.server.apply_input(player_id, x, y,
  PlayerInput::default(), 0)` (coords are written unconditionally; the seq is
  irrelevant).
- `total_shots`/`last_shots` are unchanged.

#### `e2e/tests/authoritative_death.rs` (new)

- PvP death + respawn + no self-report e2e (see acceptance criteria).

### Edge cases

- **Victim in another room** → no candidate; bullet stays at the origin.
- **Wall between shooter and victim** (in the ray's tile row) → no hit; a wall in
  another row does not block.
- **Multiple victims in line** → closest `hit_x` wins; a wall between the
  shooter and the closest victim lets the next visible victim be hit.
- **Exact distance tie** → lower victim id wins (deterministic).
- **Shooter overlaps the victim** (distance clamped to 0) → hit at
  `hit_x = spawn_x`.
- **Victim behind the shooter** (`d_far < 0`) → skipped.
- **Shooter only player** → no hit; `update` returns an empty hit list.
- **Two shooters hit the same victim in one tick** → exactly one `PlayerHitInfo`
  and one respawn; the first shot in ascending shooter-id order is credited and
  determines the bullet's impact x.
- **Victim shot again immediately after respawn** → normal hit (no i-frames); they
  are now in room `(0,0)` at `(100,100)`.
- **Shooter claims a bogus room `(255,255)`** → existing `update` fallback to
  `levels[0][0]` is reused; room equality still uses the raw `coords` values, so
  a bogus room simply matches other players claiming the same bogus room. Accepted
  (spec 3 fixes room authority).
- **Client receives `PlayerDied` for itself before `JoinResponse`** → `world.id`
  is `None`, so the remote branch runs and removal is a no-op.
- **`PlayerDied` arrives before the respawned `PlayerPos`** (different
  reliability streams) → the local player already resets via `respawn_local`; a
  remote viewer may lose the `Nemesis` for at most one tick until the next
  `PlayerPos` re-creates it. Accepted.
- **Bullet spawned at the impact x** (owner decision) → the bullet is rendered
  from the impact point onward and keeps flying; the kill is instantaneous either
  way (hitscan; lag compensation is deferred). Accepted visual artifact.

## Data model

```rust
// protocol/src/lib.rs — renamed in place
PlayerDied {
    id: u8,
},

// server/src/world.rs — new
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerHitInfo {
    pub shooter_id: u8,
    pub victim_id: u8,
}

// server/src/world.rs — changed / new
pub fn update(&mut self, dt: f32)
    -> (Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>);
fn raycast_player(&self, shooter_id: u8, spawn_x: f32, spawn_y: f32, dir: f32)
    -> Option<(u8, f32)>; // (victim_id, hit_x)
fn respawn(&mut self, id: u8);

// game/src/level.rs — new
pub fn solid_between(&self, x0: f32, x1: f32, y: f32) -> bool;

// client/src/world/lib.rs — renamed, no network send
pub fn respawn_local(world: &mut World);
```

| Item | Today | After |
| --- | --- | --- |
| `GamePacket::Hit { entity_id }` | present | renamed `PlayerDied { id }` (same enum position) |
| `ServerWorld::update` return | `Vec<BulletSpawnInfo>` | `(Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>)` |
| `server::PlayerHitInfo` | absent | new (`shooter_id`, `victim_id`) |
| `ServerWorld::raycast_player` / `respawn` | absent | new private methods |
| `BulletSpawnInfo` | shape unchanged | shape unchanged; `x` = impact x on a hit, origin on a miss |
| `Level::solid_between` | absent | new public helper |
| `client::world::lib::{check_player_death, handle_death}` | collision death + self-report | `check_player_death` removed; `handle_death` → `respawn_local` (no send) |
| `World.other_players` on remote death | never removed for death | entry removed (re-created by `PlayerPos`) |
| `Player`, `Nemesis`, bullets | unchanged | unchanged |

## Network

| Packet | Before | After |
| --- | --- | --- |
| `Hit { entity_id }` | client → server (self-report of own death), reliable-ordered | **removed** |
| `PlayerDied { id }` | — | server → **all** clients, reliable-ordered, one per detected hit |

- Broadcast scope: every connected address, including the victim. No room filter
  (spec 3).
- The server ignores any inbound `PlayerDied`; there is no client→server death
  packet anymore.
- Packet logging (`RUSTVANIA_PACKET_LOG`) reports type `"PlayerDied"`, `id =
  victim id`, `key_fields = {"id": victim_id}` on both server and client.
- Postcard: a variant rename in place is not a wire change for the other
  variants; the variant payload is a single `u8` in both cases (`entity_id` →
  `id`), so the encoded bytes are identical. No migration is needed.
- `BulletSpawn` keeps its exact payload; only the `x` semantics on a hit change.

## Acceptance criteria

**Build / hygiene**

- `cargo test --workspace` passes.
- `cargo fmt --check --all` clean.
- `cargo clippy --all-targets --all-features --workspace -- -D warnings` clean.
- Grep-clean (code only: `protocol/`, `server/`, `client/`, `game/`, `e2e/`) for
  `GamePacket::Hit`, `round_trip_hit`, `entity_id`, `check_player_death`,
  `handle_death`. (Historical `docs/` hits are fine.)

**Protocol**

- `GamePacket::PlayerDied { id }` round-trips for `0` and `255`;
  `round_trip_player_died` exists and `round_trip_hit` is gone.
- No `Hit` variant remains; other variants are unchanged.

**Game (`game/src/level.rs`)**

- `solid_between` returns `true` for a wall tile between `x0` and `x1` in the
  ray's row.
- It returns `false` when the segment stops before the wall, and when the only
  wall is in an adjacent row (the `solid_in_rect` 1-px regression).
- Out-of-grid rows/columns do not count as solid.

**Server unit tests (`server/src/world.rs`)**

- `raycast_hits_player_same_room` — shooter and victim in `(0,0)`, aligned on the
  ray → `Some((victim_id, hit_x))`, `hit_x == victim.pos.x` for a rightward shot
  (near edge = body left edge).
- `raycast_misses_player_in_other_room` — same geometry, victim `coords = (0,1)`
  → `None`.
- `raycast_blocked_by_wall` — set `world.levels[0][0].grid[row][col] =
  TileType::Wall` between the two players → `None`; clearing it hits again.
- `raycast_first_hit_wins` — two victims in line → the nearer victim id/hit_x.
- `raycast_never_hits_shooter` — only the shooter present → `None`; with both
  present, the shooter id is never returned.
- `raycast_tie_breaks_on_lower_id` — two victims at the same hit x → lower id.
- `update_returns_bullet_and_hit_info` — one shoot input with a victim in line →
  `hits == vec![PlayerHitInfo { shooter_id, victim_id }]`, `shots.len() == 1`,
  `shots[0].x` is the impact x (not the origin).
- `update_miss_emits_bullet_at_origin` — no victim in line → empty hits,
  `shots[0].x == spawn_x`.
- `update_respawns_victim_at_origin` — after the hit,
  `players[&victim].pos == (100.0, 100.0)`, `coords[&victim] == (0, 0)`,
  all pending input flags for the victim are `false` (`PlayerInput` has no
  `PartialEq`), and the shooter is untouched.
- `update_dedupes_one_victim_per_tick` — two shooters, same victim, one tick →
  exactly one `PlayerHitInfo` for that victim.

**Client unit tests**

- `player_died_for_self_respawns_local_player` — `handle_packet(&mut world,
  PlayerDied { id: my_id })` → `player.pos == (100.0, 100.0)`,
  `current_coords == (0,0)`, clock-sync fields zeroed, no packet sent.
- `player_died_for_remote_removes_nemesis` — insert a `Nemesis` in
  `other_players` (and `other_interp`), handle `PlayerDied { id: remote }` → both
  entries gone.
- `remote_bullet_contact_does_not_kill_or_send` — a `remote_bullets` entry
  overlapping the local player, `world.update(TICK_DT)` → the player is not
  teleported and the `PacketIO` recorded zero sends.

**E2E (`e2e/tests/authoritative_death.rs`)**

- `e2e_shoot_kills_player_in_same_room`:
  - two clients; server players at `(100, 201)` (shooter) and `(400, 201)`
    (victim), both grounded, in room `(0,0)` (row 5 of `LVL_0_0` is open); matching
    client positions; `tick_n(3)` to settle.
  - `press_shoot(shooter)`; tick until `h.last_deaths` is non-empty (≤ 5 ticks).
  - assert exactly one hit: `shooter_id == shooter`, `victim_id == victim`.
  - assert the victim's client respawned: `client_player_pos(victim)` within
    2 px of `(100, 100)` and `client_player_coords(victim) == (0, 0)`.
  - assert the shooter did not respawn (still near its pre-shot position).
- `e2e_victim_does_not_self_report`: in the same scenario,
  `h.client_death_reports == 0` (no client ever sent `PlayerDied`).
- The existing movement, multiplayer, reconciliation, rooms, shoot,
  bullet_spawn, and edge-case suites still pass unchanged (only the `update`
  tuple ripple and the `log` renames touch shared code).

## Gotchas (for the implementer / planner)

- **`client/src/network/transport.rs` is a missed surface in the brief**: two
  `GamePacket::Hit { .. }` reliability arms must be removed or the crate will not
  compile. `client/src/network/logging.rs` has six `Hit` sites (three code, three
  test assertions) with the same problem; `server/src/main.rs` has five (three
  logging helpers + the inbound handler's pattern and its outgoing packet
  construction).
- **The brief's claim that `client/src/network/world.rs` "marks a remote Nemesis
  dead" is not what the code does** — the `Hit` arm only resets the local
  player's clock-sync fields, and `Nemesis` has no `alive` field. This spec
  defines remote death as entry removal (the `Leave` path); do not add an
  `alive` flag.
- `check_player_death` is the only caller of `handle_death` and the only caller
  of `Bullet::check_collision`; removing it is intentional. `check_collision`
  stays public on `Bullet` (no dead-code warning).
- **`solid_in_rect` is not usable for the ray**: with `TILE_SIZE = 40` and
  `spawn_y = 119` (default spawn center), a 1-px rect spans tile rows 2 and 3.
  Use the new `solid_between`.
- `TILE_SIZE` is **40**, not 32. Room `(0,0)` is 800×480 px. Use row 5
  (`y ≈ 201`) in e2e tests: it is fully open between the border walls, and it
  avoids the default `(100,100)` spawn row, whose center (`y = 119`) sits on the
  row 2 / row 3 boundary that makes `solid_in_rect` misbehave.
- `self.players` is a `HashMap`: sort `shoot_requests` by `player_id` before the
  second pass or both bullet ids and multi-victim results become
  nondeterministic.
- The `update` return-type change ripples to **every** `let shots =
  world.update(...)` in `server/src/world.rs`'s tests (≈6 sites), plus
  `server/src/main.rs` and `e2e/src/harness.rs`. Compiler-driven, but the test
  sites are easy to miss because they live in two separate `#[cfg(test)]`
  regions.
- `PlayerInput.level_x/level_y` and client-claimed `coords` remain (spec 3).
  Do not "fix" room authority here.
- The victim's respawn position is duplicated by design: the server resets
  `Player`/`coords`, and the client applies the same canonical spawn locally for
  instant feedback. Both use `Player::new` (100,100)/(0,0), so the next
  `PlayerPos` reconciles without a jump.
- Keep the respawn reset set in sync with `load_current_level` (bullets,
  `spawned_bullet_ids`, `confirmed_bullets`, clock sync, sticky edges) — the
  existing `handle_death` already calls it; just stop sending.

## Follow-up

- `feat/authoritative-room-membership` (spec 3): make `coords` server-authoritative
  and room-filter the `PlayerDied` broadcast if desired.
- `feat/lag-compensation` (deferred): rewind victim positions to the shooter's
  render time; today hits use the current tick's authoritative positions.
- Optional visual polish: render a bullet as a tracer from the shooter to the
  impact x instead of spawning it at the impact (the owner accepted the current
  spawn-at-impact behavior for this spec).
- Optional gameplay: respawn invulnerability, kill feed, score, respawn delay.
