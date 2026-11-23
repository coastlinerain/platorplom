# Phase 1 — Tests and dead-code cleanup

- Status: accepted
- Date: 2026-11-23
- Branch: `test/phase-1-tests-cleanup`

## Summary

Add the first real test coverage to the codebase, fix one latent spawn bug, and
remove the dead code that was deliberately parked behind `#[allow(dead_code)]`
in Phase 0. Three test deliverables: (1) postcard round-trip tests for every
`GamePacket` variant in the `protocol` crate; (2) mapping/parsing tests for the
`From<u8>` impls of `TileType`/`EntityType` and for `Map::new` grid parsing;
(3) unit tests for the `Level` collision primitives (`solid_in_rect`,
`grounded`, `colision`). Two hardening/cleanup deliverables: (4) fix the
initial-room spawn bug so the starting room's entities are spawned when the
world first loads (currently only on transitions/death); (5) delete the two dead
functions (`Level::try_move` and `World::spawn_entities`) and their
`#[allow(dead_code)]` attributes.

This is a hardening phase: the tests document and lock **current** behavior, the
dead-code removal deletes **unreachable** code, and the spawn fix is a small,
currently-latent behavior correction (no visible change today because all
`DATA_*` grids are empty, but the code becomes correct for when they hold real
entity data).

## Goals / non-goals

### Goals

1. **Protocol round-trip tests** — for every `GamePacket` variant, serialize with
   `postcard::to_allocvec`, deserialize with `postcard::from_bytes`, and assert
   the decoded value equals the original. The wire format under test is
   `postcard`, not `serde_json`.
2. **`From<u8>` mapping tests** — assert the exact `u8` → `TileType` and
   `u8` → `EntityType` mappings, including the fallback of every unmapped value
   to `Empty`.
3. **Maps parsing tests** — assert `Map::new()` produces the correct
   four-dimensional shape (`grids` and `data`) and that the raw `u8` grids are
   converted to the correct `TileType`/`EntityType` values.
4. **Level collision tests** — unit-test `solid_in_rect`, `grounded`, and
   `colision` with small hand-built grids, covering walls at edges, out-of-bounds
   coordinates, and grounding.
5. **Dead-code removal** — delete `Level::try_move` and `World::spawn_entities`
   (see "Dead-code decisions" below), removing their `#[allow(dead_code)]`
   attributes.
6. **Initial-room entity spawning** — spawn the starting room's entities when the
   world first loads (not only on transitions/death). Extract the
   "iterate `level.data` → `Vec<Enemy>`" loop into a pure, headlessly-testable
   function shared by `load_current_level` and the initial-load path (see
   "Initial-room spawn fix" below).

### Non-goals (explicitly out of scope)

- **ADR-001 netcode finalization** — a separate, later task.
- **Netcode hardening** — fixed tick rate, channel selection, interpolation,
  server-authoritative simulation, lag compensation — later phase.
- **Any game-behavior change** — with one sanctioned exception. This is a
  hardening phase. The `try_move` edge-snapping collision resolver is **not**
  wired into `player.rs` (that would change how the player collides with walls),
  and `spawn_entities` is **not** wired into world loading. The single exception
  is the initial-room spawn fix: it is a small, currently-latent correction
  (today all `DATA_*` grids are empty, so it changes nothing the player can see;
  it only makes the spawn path correct for future entity data).
- **Renaming `colision`** (misspelling of "collision"). It is a public method
  called from `player.rs`; renaming is a mechanical refactor outside this phase.
- **Fixing the `solid_in_rect` vs `solid_point` out-of-bounds asymmetry** (see
  Behavior / edge cases). It is existing behavior, documented and locked by test,
  not changed.

## Behavior

No player-visible change today (the spawn fix is latent because all `DATA_*`
grids are empty). Observable behavior is for developers and CI:

- `cargo test --workspace` from the repo root runs the new tests headlessly (no
  window/GPU) and passes.
- The `protocol` crate's round-trip tests exercise all six `GamePacket` variants.
- The `client` crate's unit tests exercise the mapping, parsing, collision, and
  spawn functions.
- On world construction, the starting room `(0, 0)`'s entities are spawned once,
  via the same pure spawn logic used by `load_current_level`.
- The two dead functions (`try_move`, `spawn_entities`) no longer exist in the
  tree; `grep -rn "allow(dead_code)"` finds no remaining allow on either
  function.

### Test placement (recommendation)

- **Protocol round-trip tests → integration test file
  `protocol/tests/round_trip.rs`.** They use only the public `GamePacket` API
  plus `postcard`, so an integration test is the right shape. This keeps
  `postcard` a `[dev-dependencies]` entry in `protocol/Cargo.toml` (the library's
  normal build stays lean — it still depends only on `serde`).
- **Client-side unit tests → inline `#[cfg(test)] mod tests`** in
  `client/src/types.rs`, `client/src/maps.rs`, and `client/src/level.rs`. These
  tests must reach private items that a separate `tests/` crate cannot: the
  private `convert` helper and the `LVL_*_*`/`DATA_*_*` constants in `maps.rs`,
  and the private `solid_point` in `level.rs`. Inline child modules can access
  these; integration tests cannot.
- **Spawn helper unit test → inline `#[cfg(test)] mod tests` in
  `client/src/world/lib.rs`.** The extracted pure spawn function is a private
  module function, so it is tested inline (a separate `tests/` crate cannot see
  it). Full `World` construction is deliberately **not** tested: `World::new`
  requires a `NetworkClient`, which binds a socket and spawns a thread.

### Edge cases (assert current behavior; do not change it)

- **`From<u8>` fallback.** `TileType::from` maps `0→Empty, 1→Wall,
  2→EnemySpawn, 3→BossSpawn`, and **everything else (4…255) → `Empty`**.
  `EntityType::from` maps `0→Empty, 1→EnemySpawn, 2→BossSpawn, 3→PotionSpawn`,
  and **everything else → `Empty`**. Note the two enums deliberately use the
  same discriminant `1`/`2` for different meanings (Wall vs EnemySpawn, etc.);
  tests must assert each enum independently against its own table.
- **`Map::new` shape.** `grids` is `Vec<Vec<Vec<Vec<TileType>>>>` with outer
  dimensions `2 × 2` (two rows of two levels), each inner grid `MAP_HEIGHT`
  rows × `MAP_WIDTH` columns; `data` has the same outer shape with
  `EntityType`. `MAP_WIDTH == 20`, `MAP_HEIGHT == 12`, `TILE_SIZE == 40.0`.
- **`solid_in_rect` vs out-of-bounds.** `solid_in_rect` iterates the grid cells
  a rect covers and returns `true` only for an **in-bounds wall cell**; a rect
  whose only overlap is outside the grid returns `false` (out-of-bounds is
  treated as *not solid*).
- **`solid_point` vs out-of-bounds (used by `colision`).** `solid_point` returns
  `true` for coordinates outside the grid (the `else` branch), and its
  `(coord / TILE_SIZE) as usize` cast **saturates negative floats to 0**. So
  `colision` treats a rect at/beyond the map edge as colliding, and a rect with
  negative x/y samples against row/column 0. This is the opposite of
  `solid_in_rect`'s out-of-bounds behavior — the asymmetry is documented and
  locked by test, not fixed.
- **`colision` sample points.** `colision` tests six points derived from the
  rect's edges with a hard-coded ±2.0 px horizontal inset (two top corners, two
  bottom corners, two mid-height side points). A 2 px-wide wall penetration at
  the top/bottom edge is therefore **not** detected; tests assert this.
- **`grounded`.** `grounded` re-uses `solid_in_rect` on a rect shifted down by
  exactly 1.0 px at full height (not a thin probe). A rect whose bottom edge is
  within 1 px above a wall row reports grounded; a rect floating further above
  does not.
- **Spawn mapping.** The spawn loop maps `EntityType::EnemySpawn →
  Enemy(pos, BLUE)`, `EntityType::PotionSpawn → Enemy(pos, GREEN)`, and
  `EntityType::BossSpawn` / `EntityType::Empty` → nothing. Positions are
  computed as `vec2(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE)`.

## Data model

### Enabling change: `PartialEq` on `GamePacket`

`GamePacket` currently derives only `Debug, Serialize, Deserialize`. The
round-trip test asserts equality of the decoded value, which requires
`PartialEq`. This phase adds `PartialEq` to the `GamePacket` derive in
`protocol/src/lib.rs`:

```rust
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket { /* unchanged variants */ }
```

- `PartialEq` only, **not `Eq`**: the `f32` fields (`x`, `y`, `dir`) do not
  implement `Eq`.
- No variant, field, or ordering changes. Variant order must stay exactly as-is;
  postcard encodes variants by index.

### `protocol` dev-dependency

`protocol/Cargo.toml` gains a `[dev-dependencies]` entry:
`postcard = { version = "1.1.3", features = ["alloc"] }` (the `alloc` feature is
required for `postcard::to_allocvec`). This does not add `postcard` to the
library's normal dependency graph.

## Network (if any)

No wire change. The wire format (postcard) and the `GamePacket` variant set,
order, and field types are unchanged. The only protocol-adjacent change is the
addition of `PartialEq` (a compile-time derive, no wire effect) and the
dev-dependency on `postcard` for tests.

The round-trip tests verify **within-crate** serialization consistency (encode →
decode → equal). They do not, by themselves, detect drift between two independent
copies of the enum (there is only one copy now, per Phase 0); cross-crate
interoperability remains covered by server↔client sharing the single
`protocol::GamePacket`.

## Dead-code decisions

### `Level::try_move` → **DELETE** (do not wire up)

`try_move` is an unused, untested edge-snapping collision resolver marked
`#[allow(dead_code)]` with a "wire up in Phase 1" TODO. The recommendation is to
**delete** it rather than wire it up, because:

1. **Wiring it up is a game-behavior change.** `player.rs` currently resolves
   collisions by "move, then fully reverse the axis" (`pos += vel; if colision
   { pos -= vel; vel.y = 0 }`). `try_move` instead **snaps** the player to the
   tile edge, which changes how the player feels against walls — a user-visible
   behavior change, explicitly excluded by the "hardening only" non-goal.
2. **It is unverified.** Promoting an untested resolver into the hot path would
   trade dead code for unproven behavior, contradicting the phase's intent that
   tests are the deliverable.
3. **Deletion is the clean, reversible action.** The code is recoverable from git
   history.

Follow-up (recorded, not in this phase): a dedicated "collision feel" phase may
later reintroduce an edge-snapping resolver deliberately, with player-behavior
tests.

### `World::spawn_entities` → **DELETE**

`spawn_entities` duplicates the live spawn loop in `load_current_level`
(`client/src/world/lib.rs`). Recommendation: **delete** it, because:

1. **`load_current_level` is the canonical path.** It is the function actually
   called (from `check_level_transitions` and `handle_death`); `spawn_entities`
   is called nowhere.
2. **It is not even a drop-in duplicate.** `spawn_entities` takes `&Level` and
   does not clear `world.enemies`/`world.player.bullets`, so it cannot replace
   `load_current_level` as-is.
3. **No reachable behavior is lost.** All four `DATA_*` level-data constants are
   currently all `0` (`EntityType::Empty`), so neither function spawns any entity
   today; deleting `spawn_entities` removes zero currently-observable behavior.

Note: `load_current_level` is only called on level transitions and death, so the
initial room's entities were never spawned. This phase fixes that gap (see
"Initial-room spawn fix" below); `spawn_entities` itself is still deleted.

## Initial-room spawn fix

`load_current_level` (`client/src/world/lib.rs`) is the only spawn path, and it
is called only from `check_level_transitions` and `handle_death` — never when the
world first loads. Result: the starting room `(0, 0)`'s entities are never
spawned. Today this is invisible because all four `DATA_*` grids are all `0`
(`EntityType::Empty`), but the code must be correct for when `DATA` holds real
entity data.

Fix shape (recommendation):

- Extract the body of the spawn loop into a **pure, private function** in
  `client/src/world/lib.rs`, e.g. `fn enemies_from_data(data: &[Vec<EntityType>])
  -> Vec<Enemy>`: for each cell, `EntityType::EnemySpawn → Enemy::new(vec2(x as
  f32 * TILE_SIZE, y as f32 * TILE_SIZE), BLUE)` and `EntityType::PotionSpawn →
  Enemy::new(vec2(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE), GREEN)`; all other
  variants (`BossSpawn`, `Empty`) are skipped.
- `load_current_level` keeps its clear-and-assign responsibilities
  (`world.enemies.clear(); world.player.bullets.clear();`) and assigns
  `world.enemies = enemies_from_data(&level.data)`.
- The initial-load path (world construction) runs the spawn **once** for the
  starting room `(0, 0)` by calling the same helper and assigning its result.
- The helper is pure (no `World`, no `NetworkClient`, no socket, no thread), so
  it is unit-testable headlessly. Tests target the extracted function only —
  they do **not** construct a full `World` via `World::new`.

## Acceptance criteria

Each item is a checkable statement of "done". Not an implementation plan.

### Protocol round-trip tests

1. `protocol/tests/round_trip.rs` exists and, for **each of the six**
   `GamePacket` variants, serializes with `postcard::to_allocvec`, deserializes
   with `postcard::from_bytes`, and asserts the decoded value equals the
   original (`assert_eq!`).
2. `GamePacket` derives `PartialEq` (added in this phase); the crate compiles
   with `derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)` and does
   **not** derive `Eq`.
3. Round-trip coverage includes boundary values for `u8` fields (`0` and `255`)
   and a non-ASCII `String` for `Action.kind` (to prove `String` round-trips by
   length + bytes, not by char count).
4. `protocol/Cargo.toml` declares `postcard` under `[dev-dependencies]` with the
   `alloc` feature; `postcard` is **not** a normal dependency of `protocol`.
5. `cargo test -p protocol` passes.

### `From<u8>` mapping tests

6. Inline `#[cfg(test)] mod tests` in `client/src/types.rs` asserts the full
   `TileType` mapping: `0→Empty`, `1→Wall`, `2→EnemySpawn`, `3→BossSpawn`, and
   at least two unmapped values (e.g. `4`, `255`) → `Empty`.
7. The same module asserts the full `EntityType` mapping: `0→Empty`,
   `1→EnemySpawn`, `2→BossSpawn`, `3→PotionSpawn`, and at least two unmapped
   values → `Empty`.

### Maps parsing tests

8. Inline `#[cfg(test)] mod tests` in `client/src/maps.rs` asserts `Map::new()`
   returns `grids` with outer shape `2 × 2`, each inner grid `MAP_HEIGHT` rows ×
   `MAP_WIDTH` columns, and `data` with the same outer shape (`EntityType`).
9. The same module asserts the private `convert` helper maps known raw values to
   the expected `TileType`/`EntityType` (spot checks on real `LVL_*_*` /
   `DATA_*_*` constants, e.g. the corner cells of `LVL_0_0` are `Wall` and an
   interior `0` cell is `Empty`).

### Level collision tests

10. Inline `#[cfg(test)] mod tests` in `client/src/level.rs` builds a small
    hand-built `Level` (e.g. a 3×3 grid with a wall ring around an empty center,
    `TILE_SIZE == 40.0`) and asserts `solid_in_rect`:
    - `true` for a rect overlapping a wall cell,
    - `false` for a rect fully inside an empty cell,
    - `true` for a rect straddling a wall/empty boundary,
    - `false` for a rect whose only overlap is outside the grid (out-of-bounds
      is not solid for `solid_in_rect`).
11. The same module asserts `grounded`: a rect whose bottom edge is within 1 px
    above a wall row is grounded (`true`); a rect floating further above is not
    (`false`).
12. The same module asserts `colision` (existing behavior, unchanged):
    - `false` for a rect fully inside an empty area,
    - `true` for a rect overlapping a wall,
    - `true` for a rect at/beyond a map edge (out-of-bounds is solid for
      `colision`, via `solid_point`), and
    - the ±2 px horizontal inset is honored (a 2 px wall penetration at a
      top/bottom edge is not detected).
13. The `solid_in_rect` vs `solid_point` out-of-bounds asymmetry (criteria 10 and
    12) is asserted as-is and commented; no code is changed to unify them.

### Initial-room entity spawning

14. The initial room `(0, 0)`'s entities are spawned when the world first loads:
    the spawn path runs exactly once for the starting room, not only on
    `check_level_transitions`/`handle_death`. With the current all-empty
    `DATA_*` grids this spawns zero entities; the deliverable is the code path
    (the spawn runs on initial load), not a visible enemy.
15. The "iterate `level.data` → `Vec<Enemy>`" loop is extracted into a pure
    function in `client/src/world/lib.rs` (e.g.
    `enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>`), and both
    `load_current_level` and the initial-load path call it. `load_current_level`
    still clears `world.enemies`/`world.player.bullets` and then assigns the
    helper's result; the initial-load path assigns the helper's result for the
    starting room.
16. An inline `#[cfg(test)] mod tests` in `client/src/world/lib.rs` unit-tests
    the extracted pure function headlessly (no `World`, no `NetworkClient`, no
    socket, no thread), asserting: `EnemySpawn` produces one enemy whose `pos`
    equals `(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE)`; `PotionSpawn` produces
    one enemy at the same position; `BossSpawn` and `Empty` produce none; and the
    total enemy count matches the number of `EnemySpawn` + `PotionSpawn` cells in
    the input grid.

### Dead-code removal

17. `Level::try_move` no longer exists in `client/src/level.rs`; its
    `#[allow(dead_code)]` attribute is removed.
18. `World::spawn_entities` no longer exists in `client/src/world/game.rs`; its
    `#[allow(dead_code)]` attribute is removed.
19. `grep -rn "try_move\|spawn_entities" client/src` returns no matches, and
    `grep -rn "allow(dead_code)" client/src` returns no matches for these two
    functions.

### Quality gates

20. `cargo fmt --check --all` reports no diffs.
21. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
22. `cargo test --workspace` passes (all new tests green, no rendering/window
    code exercised — collision tests construct `Rect`/`Vec2` values only and
    never call `Level::draw` or any macroquad drawing function).
