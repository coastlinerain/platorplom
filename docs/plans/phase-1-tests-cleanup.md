# Phase 1 — Tests and dead-code cleanup — Implementation Plan

- Status: done
- Spec: `docs/specs/phase-1-tests-cleanup.md`
- Date: 2026-11-23

## Overview

Hardening phase. Three test deliverables (postcard round-trip for every
`GamePacket` variant; `From<u8>` mapping + `Map::new` parsing tests; `Level`
collision tests) plus two hardening deliverables (fix the initial-room spawn
gap by extracting a pure `enemies_from_data` helper and wiring it into
`World::new`; delete the two dead functions `Level::try_move` and
`World::spawn_entities`).

**TDD ordering (who does what, in order):**

1. **Tester** writes *all* tests first. Some are **green immediately** because
   they lock existing behavior (`From<u8>` mappings, `Map::new` shape, collision
   primitives). Some are **red** because they reference things that don't exist
   yet (`assert_eq!` on `GamePacket` needs `PartialEq`; the spawn-helper test
   needs `enemies_from_data`).
2. **Implementer** then, in dependency order: adds `PartialEq` to `GamePacket`,
   adds the `postcard` dev-dependency, extracts `enemies_from_data`, wires the
   initial-load spawn, deletes `try_move` / `spawn_entities`, and makes every
   test green.

**Coordination note:** the red tests break compilation of their whole test
target. Once the `enemies_from_data` test is written into
`client/src/world/lib.rs` (and the round-trip tests into
`protocol/tests/round_trip.rs`), `cargo test` for that crate is red until the
implementer lands `PartialEq` / `postcard` / `enemies_from_data`, so the "green
immediately" tests cannot be run individually in the interim. The tester should
verify the green tests *before* adding the red ones, or accept a red
`cargo test` until implementation.

No player-visible behavior change today (all `DATA_*` grids are empty). The
spawn fix is a latent correctness correction.

## Public API (targets for the tester)

### (a) `PartialEq` on `GamePacket` (`protocol/src/lib.rs`)

Add `PartialEq` to the existing derive. **Not `Eq`** — the `f32` fields
(`x`, `y`, `dir`) do not implement `Eq`. Variant set, order, and field types are
unchanged (postcard encodes variants by index, so order is part of the wire
contract).

```rust
// protocol/src/lib.rs — derive line ONLY; variants unchanged
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    JoinRequest,
    JoinResponse { assigned_id: u8 },
    PlayerPos { id: u8, x: f32, y: f32, dir: f32, level_x: u8, level_y: u8 },
    Action { id: u8, kind: String, dir: f32 },
    Leave { id: u8 },
    Hit { entity_id: u8 },
}
```

### (b) Extracted pure spawn helper (`client/src/world/lib.rs`)

```rust
/// Iterate one room's `data` grid and build the enemies it declares.
/// Pure: no `World`, no `NetworkClient`, no socket, no thread.
pub(crate) fn enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>;
```

- Maps `EntityType::EnemySpawn → Enemy::new(vec2(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE), BLUE)`.
- Maps `EntityType::PotionSpawn → Enemy::new(vec2(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE), GREEN)`.
- `BossSpawn` and `Empty` are skipped.
- `pub(crate)` (not `pub`): it must be callable from the sibling module
  `world/game.rs` (where `World::new` lives) and from the inline test child
  module. A plain `fn` would be invisible to `game.rs`; `pub(crate)` works, and
  `pub(super)` would also suffice (both `world::lib` and `world::game` are
  children of `world`).
- `load_current_level` keeps its clear-and-assign duties
  (`world.enemies.clear(); world.player.bullets.clear();`) and then assigns
  `world.enemies = enemies_from_data(&level.data);`.
- `World::new` assigns `enemies = enemies_from_data(&levels[0][0].data);` **after**
  `levels` is built but **before** it is moved into the struct literal (the
  borrow of `levels[0][0].data` must end before `levels` is moved).

### (c) Test layout

| Deliverable | Location | Kind |
| --- | --- | --- |
| Protocol round-trip | `protocol/tests/round_trip.rs` | integration test (uses only public `GamePacket` + `postcard`) |
| `From<u8>` mappings | `client/src/types.rs` | inline `#[cfg(test)] mod tests` |
| `Map::new` parsing | `client/src/maps.rs` | inline `#[cfg(test)] mod tests` |
| `Level` collision | `client/src/level.rs` | inline `#[cfg(test)] mod tests` |
| `enemies_from_data` | `client/src/world/lib.rs` | inline `#[cfg(test)] mod tests` |

The client crate is a binary (`src/main.rs`, no `lib.rs`), so client tests must
be inline `#[cfg(test)]` modules (integration tests under `client/tests/` would
need a library target). Inline child modules can reach the module-level
`LVL_*_*` / `DATA_*_*` constants in `maps.rs`, the private `solid_point` in
`level.rs`, and the `pub(crate)` helper in `world/lib.rs`. (The `convert`
functions are nested inside `get_world_matrix` / `get_world_data`, so they are
**not** reachable; the parsing tests assert through `Map::new()` output instead.)

The `postcard` dev-dependency is added to `protocol/Cargo.toml`:

```toml
[dev-dependencies]
postcard = { version = "1.1.3", features = ["alloc"] }
```

The `alloc` feature is required for `postcard::to_allocvec`.

## Test cases (tester writes these first)

Legend: **[PASS NOW]** = locks existing behavior, green before any code change.
**[FAIL →]** = red until the noted implementation lands.

### Protocol round-trip — `protocol/tests/round_trip.rs`

All six use `postcard::to_allocvec(&pkt)` then `postcard::from_bytes::<GamePacket>(&bytes)`
and `assert_eq!(decoded, pkt)`. These **[FAIL →]** until `GamePacket: PartialEq`
(Box 1) and the `postcard` dev-dependency (Box 2) exist — `assert_eq!` requires
`PartialEq`, and the file won't even compile without `postcard`.

- `round_trip_join_request` — `GamePacket::JoinRequest`.
- `round_trip_join_response` — `JoinResponse { assigned_id: 0 }` and
  `{ assigned_id: 255 }` (u8 boundary values).
- `round_trip_player_pos` — `PlayerPos { id: 0, x: -1.5, y: 100.25, dir: 3.1415,
  level_x: 0, level_y: 255 }` and the `id/level_x/level_y = 255` boundary case.
- `round_trip_action` — `Action { id: 7, kind: "☃é🔥".to_string(), dir: -0.5 }`
  (non-ASCII `String`, proving length+bytes round-trip) plus `kind: ""`.
- `round_trip_leave` — `Leave { id: 0 }` and `{ id: 255 }`.
- `round_trip_hit` — `Hit { entity_id: 0 }` and `{ entity_id: 255 }`.

### `From<u8>` mappings — `client/src/types.rs` (inline)

Both **[PASS NOW]** — `TileType`/`EntityType` already derive `PartialEq` and the
`From<u8>` impls already exist. Each enum is asserted independently (they reuse
discriminants `1`/`2` with different meanings).

- `tile_type_from_u8` — `0→Empty, 1→Wall, 2→EnemySpawn, 3→BossSpawn`, and
  unmapped `4` and `255` → `Empty`.
- `entity_type_from_u8` — `0→Empty, 1→EnemySpawn, 2→BossSpawn, 3→PotionSpawn`,
  and unmapped `4` and `255` → `Empty`.

### Maps parsing — `client/src/maps.rs` (inline)

Both **[PASS NOW]** — `Map::new()` and the private `convert` helpers already
exist and work. `convert` is a nested `fn` (not directly callable), so the tests
assert through `Map::new()` output, cross-checked against the `LVL_*_*` /
`DATA_*_*` raw constants.

- `map_new_shape` — `Map::new().grids` has outer `len() == 2`, each row
  `len() == 2`, each inner grid `MAP_HEIGHT` (12) rows × `MAP_WIDTH` (20)
  columns; `data` has the same outer shape with `EntityType`.
- `map_new_conversion_spot_checks` — corner `grids[0][0][0][0] == TileType::Wall`
  (raw `LVL_0_0[0][0] == 1`); interior `grids[0][0][1][1] == TileType::Empty`
  (raw `LVL_0_0[1][1] == 0`); `data[0][0][0][0] == EntityType::Empty` (raw
  `DATA_0_0[0][0] == 0`).

### Level collision — `client/src/level.rs` (inline)

All **[PASS NOW]** — `Level::new`, `solid_in_rect`, `grounded`, and `colision`
already exist with the documented behavior. Tests construct only `Rect`/`Vec2`
values; they never call `Level::draw` or any macroquad drawing function.

Fixture helper `fn wall_ring_level() -> Level` builds a 3×4 grid
(`TILE_SIZE == 40.0`, `width = 3`, `height = 4`):

```text
row0: W W W      y ∈ [  0,  40)
row1: W E W      y ∈ [ 40,  80)
row2: W E W      y ∈ [ 80, 120)
row3: W W W      y ∈ [120, 160)
```

- `solid_in_rect_overlaps_wall` — `Rect::new(0.0, 0.0, 40.0, 40.0)` → `true`.
- `solid_in_rect_empty_interior` — `Rect::new(45.0, 80.0, 10.0, 10.0)` (inside
  empty cell `(1,2)`) → `false`.
- `solid_in_rect_straddles_boundary` — `Rect::new(35.0, 85.0, 10.0, 10.0)`
  (spans wall column 0 / empty column 1) → `true`.
- `solid_in_rect_out_of_bounds_not_solid` — `Rect::new(-100.0, -100.0, 50.0,
  50.0)` (only overlap is outside the grid) → `false`.
- `grounded_within_one_px_above_floor` — `Rect::new(45.0, 79.0, 10.0, 40.0)`
  (bottom edge at y=119, 1 px above the floor row 3) → `true`.
- `grounded_floating_above` — `Rect::new(45.0, 40.0, 10.0, 40.0)` (bottom edge
  at y=80, 40 px above the floor) → `false`.
- `colision_empty_area` — `colision(45.0, 80.0, 10.0, 10.0)` → `false`.
- `colision_overlaps_wall` — `colision(0.0, 0.0, 40.0, 40.0)` → `true`.
- `colision_out_of_bounds_is_solid` — `colision(200.0, 40.0, 10.0, 10.0)`
  (fully beyond the right edge; `solid_point` returns `true` out of bounds) →
  `true`.
- `colision_ignores_two_px_corner_inset` — uses a second fixture with a single
  isolated wall cell (`[W E E / E E E / E E E]`) and asserts
  `colision(38.0, 30.0, 42.0, 60.0)` → `false`, even though the rect's top-left
  corner overlaps wall cell `(0,0)`. This locks the ±2.0 horizontal inset of the
  six sample points (the top corner samples are at `x+2` / `x+w-2`, and the
  mid-side samples are at mid-height, so a 2 px corner penetration is missed).

The `solid_in_rect` vs `solid_point` out-of-bounds asymmetry (criterion 13) is
asserted as-is by `solid_in_rect_out_of_bounds_not_solid` (`false`) vs
`colision_out_of_bounds_is_solid` (`true`); no code unifies them.

### `enemies_from_data` — `client/src/world/lib.rs` (inline)

All **[FAIL →]** until `enemies_from_data` exists (Box 11). `Enemy` does **not**
derive `PartialEq`, so tests assert on `enemy.pos` (a `Vec2`, which is
`PartialEq`) and the count; if `Color` is not `PartialEq`, compare
`enemy.color` field-wise or skip the color assertion.

- `enemies_from_data_spawns_enemy_and_potion` — input
  `[[EnemySpawn, Empty], [BossSpawn, PotionSpawn]]` yields 2 enemies:
  `enemies[0].pos == vec2(0.0, 0.0)` and `enemies[1].pos == vec2(40.0, 40.0)`
  (`x as f32 * TILE_SIZE`, `y as f32 * TILE_SIZE`).
- `enemies_from_data_skips_boss_and_empty` — `[[BossSpawn, Empty]]` yields 0
  enemies; `[[Empty, Empty]]` yields 0 enemies.
- `enemies_from_data_count_matches_spawn_cells` — a grid with N `EnemySpawn` +
  M `PotionSpawn` cells yields exactly `N + M` enemies.

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Group 1 — Protocol round-trip (criteria 1–5)

#### Box 1 — Add `PartialEq` to `GamePacket`

- [x] done
- **Acceptance**: `protocol/src/lib.rs` shows
  `#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]` with **no**
  `Eq`; variant order/fields unchanged; `cargo build -p protocol` succeeds.
- **Test**: `protocol/tests/round_trip.rs` compiles (after Box 2) and all
  `round_trip_*` tests pass.
- **Explain**
  - What: Added `PartialEq` to the `GamePacket` derive line (variants unchanged).
  - Why: The round-trip tests use `assert_eq!(decoded, pkt)`, which requires the
    decoded value to be comparable with the original.
  - Rust concepts: `derive` macros (a proc-macro implementing `PartialEq` for all
    six variants); trait impls are additive and have no wire-format effect.
  - Watch out: Only `PartialEq`, not `Eq` — `f32` does not implement `Eq`, and
    postcard encodes variants by index so variant order/fields must stay frozen.

#### Box 2 — Add `postcard` `[dev-dependencies]` with `alloc`

- [x] done
- **Acceptance**: `protocol/Cargo.toml` has
  `[dev-dependencies] postcard = { version = "1.1.3", features = ["alloc"] }`;
  `postcard` is **not** under `[dependencies]` (grep confirms); `cargo test -p
  protocol` compiles the test target.
- **Test**: `cargo test -p protocol` (links `postcard::to_allocvec` /
  `postcard::from_bytes`).
- **Explain**
  - What: Added a `[dev-dependencies]` entry for `postcard` with the `alloc`
    feature.
  - Why: The integration test needs `postcard::to_allocvec`/`from_bytes`; keeping
    it a dev-dependency leaves the library's normal build dependent only on serde.
  - Rust concepts: Cargo dependency sections (dev-dependencies are only linked for
    tests/examples, not for downstream consumers); cargo features.
  - Watch out: `alloc` is required for `to_allocvec`; must not be added under
    `[dependencies]` or it would leak into the library's normal graph.

#### Box 3 — Round-trip tests green (6 variants + boundaries + non-ASCII)

- [x] done
- **Acceptance**: `cargo test -p protocol` passes; `round_trip.rs` covers all six
  variants, `u8` boundary `0`/`255`, and a non-ASCII `String` for
  `Action.kind`.
- **Test**: `cargo test -p protocol` (all `round_trip_*` tests).
- **Explain**
  - What: Enabled the existing `round_trip.rs` tests (written by the tester) by
    providing `PartialEq` and the `postcard` dev-dependency.
  - Why: They encode→decode each variant and assert round-trip equality, proving
    the postcard wire format is stable across the six variants and edge values.
  - Rust concepts: integration tests (`tests/` dir uses only the public API),
    `assert_eq!` (needs `PartialEq`), `Option`/`Result` via `.unwrap()` in the
    test helper.
  - Watch out: `f32` fields are compared bit-exactly; postcard round-trips them
    byte-for-byte so equality holds.

### Group 2 — `From<u8>` mapping tests (criteria 6–7)

#### Box 4 — `TileType` mapping test passes

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust tile_type_from_u8` passes.
- **Test**: `client/src/types.rs` → `tile_type_from_u8`. **[no production change;
  locks existing behavior]**
- **Explain**
  - What: No production change; the inline `tile_type_from_u8` test locks the
    existing `From<u8> for TileType` mapping.
  - Why: Documents that `0→Empty, 1→Wall, 2→EnemySpawn, 3→BossSpawn` and any
    unmapped value falls through to `Empty`.
  - Rust concepts: `From<u8>` trait; inline `#[cfg(test)] mod tests` reaching
    private items; `assert_eq!` on `PartialEq` enums.
  - Watch out: `TileType` and `EntityType` reuse discriminants with different
    meanings; the test asserts this enum against its own table only.

#### Box 5 — `EntityType` mapping test passes

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust entity_type_from_u8` passes.
- **Test**: `client/src/types.rs` → `entity_type_from_u8`. **[no production change;
  locks existing behavior]**
- **Explain**
  - What: No production change; the inline `entity_type_from_u8` test locks the
    existing `From<u8> for EntityType` mapping.
  - Why: Documents `0→Empty, 1→EnemySpawn, 2→BossSpawn, 3→PotionSpawn` plus the
    `Empty` fallback for unmapped values.
  - Rust concepts: `From<u8>` trait; `#[cfg(test)]` module; `assert_eq!`.
  - Watch out: Same discriminant values map differently than `TileType`; asserting
    independently avoids coupling the two tables.

### Group 3 — Maps parsing tests (criteria 8–9)

#### Box 6 — `Map::new` shape test passes

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust map_new_shape` passes.
- **Test**: `client/src/maps.rs` → `map_new_shape`. **[no production change;
  locks existing behavior]**
- **Explain**
  - What: No production change; `map_new_shape` locks the `Map::new` 4-D shape.
  - Why: Documents outer `2×2`, inner `MAP_HEIGHT`×`MAP_WIDTH` for both `grids`
    and `data`.
  - Rust concepts: `Vec` of `Vec` nesting; `#[cfg(test)]` module; range iteration.
  - Watch out: The nested `convert` helpers are not directly callable from the
    test, so the test asserts through `Map::new()` output only.

#### Box 7 — `Map::new` conversion spot-check test passes

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust map_new_conversion_spot_checks`
  passes.
- **Test**: `client/src/maps.rs` → `map_new_conversion_spot_checks`. **[no
  production change; locks existing behavior]**
- **Explain**
  - What: No production change; spot-checks the `u8`→enum conversion through
    `Map::new` against the raw `LVL_0_0`/`DATA_0_0` constants.
  - Why: Confirms the private `convert` helpers map wall (`1`) and empty (`0`)
    cells correctly without exposing them publicly.
  - Rust concepts: `assert_eq!` on derived-`PartialEq` enums; indexing into nested
    `Vec`s.
  - Watch out: The test reads only the `(0,0)` level grid; corner cell `(0,0)` is
    `Wall` while interior `(1,1)` is `Empty`.

### Group 4 — Level collision tests (criteria 10–13)

#### Box 8 — `solid_in_rect` tests pass

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust solid_in_rect` passes (wall
  overlap `true`, empty interior `false`, straddle `true`, out-of-bounds
  `false`).
- **Test**: `client/src/level.rs` → `solid_in_rect_*` tests. **[no production
  change; locks existing behavior]**
- **Explain**
  - What: No production change; the four `solid_in_rect_*` tests lock the
    existing cell-iteration behavior on a hand-built 3×4 wall ring.
  - Why: Documents the in-bounds-only semantics (out-of-bounds overlap is `false`)
    which the spec calls out as an intentional asymmetry.
  - Rust concepts: `Rect`/`Vec2` values (no drawing); private `solid_point` is
    reachable from the inline test module; `assert!`/`assert!(!…)`.
  - Watch out: Tests build `Rect` values only and never call `Level::draw`, so no
    GPU/window is needed.

#### Box 9 — `grounded` tests pass

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust grounded` passes (within 1 px
  above floor `true`; floating `false`).
- **Test**: `client/src/level.rs` → `grounded_*` tests. **[no production change;
  locks existing behavior]**
- **Explain**
  - What: No production change; `grounded_*` tests lock the 1 px probe-shift
    semantics of `grounded`.
  - Why: Documents that a rect within 1 px above a wall row reads as grounded,
    while one floating further above does not.
  - Rust concepts: `Rect` reconstruction (`Rect::new(rect.x, rect.y + 1.0, …)`);
    reuse of `solid_in_rect`; boolean assertions.
  - Watch out: `grounded` re-uses `solid_in_rect` on a full-height shifted rect,
    not a thin probe — the test matches that exact behavior.

#### Box 10 — `colision` tests pass (incl. asymmetry + ±2 inset)

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust colision` passes (empty
  `false`, wall overlap `true`, out-of-bounds `true`, ±2 inset corner penetration
  `false`); the out-of-bounds asymmetry vs `solid_in_rect` is asserted as-is and
  commented, not changed.
- **Test**: `client/src/level.rs` → `colision_*` tests. **[no production change;
  locks existing behavior]**
- **Explain**
  - What: No production change; `colision_*` tests lock the six-sample-point
    collision check plus the `solid_point` out-of-bounds behavior.
  - Why: Documents that out-of-bounds is *solid* for `colision` (opposite of
    `solid_in_rect`) and that the ±2 px horizontal inset misses a 2 px corner
    penetration.
  - Rust concepts: `solid_point`'s `(coord / TILE_SIZE) as usize` cast (negative
    floats saturate to 0); iterating sample points; `assert!`.
  - Watch out: A second fixture (`single_wall_cell_level`) pins the ±2 inset
    behavior; the asymmetry is asserted, not fixed, per the spec's non-goal.

### Group 5 — Initial-room spawn fix (criteria 14–16)

#### Box 11 — Extract `enemies_from_data` pure helper

- [x] done
- **Acceptance**: `client/src/world/lib.rs` defines
  `pub(crate) fn enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>`
  with the EnemySpawn→BLUE / PotionSpawn→GREEN mapping; `load_current_level`
  clears `world.enemies` / `world.player.bullets` and assigns
  `world.enemies = enemies_from_data(&level.data)`; the crate compiles.
- **Test**: `cargo test -p metroidvania-rust enemies_from_data` (after Box 13).
- **Explain**
  - What: Extracted the spawn loop from `load_current_level` into a pure
    `pub(crate) fn enemies_from_data`; `load_current_level` now clears and assigns
    its result.
  - Why: Shares one spawn implementation between the transition/death path and the
    new initial-load path, and makes it unit-testable without a socket/thread.
  - Rust concepts: function extraction; `&[Vec<EntityType>]` slice parameter
    (borrow, not owned); `Vec<Enemy>` return; `pub(crate)` visibility for
    sibling-module + test access; `enumerate` + `match` pattern matching.
  - Watch out: `pub(crate)` (not `pub`) keeps the helper internal but reachable
    from `world/game.rs` and the inline test module.

#### Box 12 — Wire initial-load spawn in `World::new`

- [x] done
- **Acceptance**: `client/src/world/game.rs`'s `World::new` builds `levels`, then
  assigns `enemies = enemies_from_data(&levels[0][0].data)` (borrow ends before
  `levels` is moved into the struct literal), so room `(0,0)` spawns once on
  construction; the crate compiles and `cargo test -p metroidvania-rust` passes.
- **Test**: `cargo test -p metroidvania-rust` (the pure helper is covered by
  `enemies_from_data_*`; the wiring itself is a compile + behavior check — no
  full `World::new` test, which would need a socket/thread).
- **Explain**
  - What: In `World::new`, computed `let enemies = enemies_from_data(&levels[0][0].data);`
    before the struct literal and assigned it to the `enemies` field.
  - Why: Spawns the starting room `(0,0)` entities on construction, fixing the
    latent gap where `load_current_level` only ran on transitions/death.
  - Rust concepts: borrow ordering — the shared borrow of `levels[0][0].data` ends
    before `levels` is moved into `Self`; owned `Vec<Enemy>` returned by the
    helper decouples the borrow from the later move.
  - Watch out: If `enemies_from_data(&levels[0][0].data)` were inlined into the
    struct literal, the borrow would conflict with moving `levels` — hence the
    separate `let` binding first.

#### Box 13 — `enemies_from_data` unit test passes

- [x] done
- **Acceptance**: `cargo test -p metroidvania-rust enemies_from_data` passes
  (spawn counts, `BossSpawn`/`Empty` skipped, positions = `x/y as f32 *
  TILE_SIZE`).
- **Test**: `client/src/world/lib.rs` → `enemies_from_data_*` tests.
- **Explain**
  - What: The three `enemies_from_data_*` tests (written by the tester) now
    compile and pass against the extracted helper.
  - Why: Verifies the mapping — `EnemySpawn`/`PotionSpawn` → enemy at
    `(x*TILE_SIZE, y*TILE_SIZE)`, `BossSpawn`/`Empty` skipped, and count ==
    N+M spawn cells.
  - Rust concepts: `#[cfg(test)] mod tests` reaching `super::enemies_from_data`;
    `assert_eq!` on `Vec2` (`enemy.pos`); `TILE_SIZE` comparison.
  - Watch out: `Enemy` does not derive `PartialEq`, so the tests compare
    `enemy.pos` and `len()` only, not the whole `Enemy` (color is skipped).

### Group 6 — Dead-code removal (criteria 17–19)

#### Box 14 — Delete `Level::try_move`

- [x] done
- **Acceptance**: `client/src/level.rs` no longer contains `try_move` nor its
  `#[allow(dead_code)]` attribute; the crate compiles.
- **Test**: `grep -rn "try_move" client/src` returns no matches;
  `cargo test -p metroidvania-rust` still passes.
- **Explain**
  - What: Removed the `try_move` method and its `#[allow(dead_code)]` + TODO
    comment from `level.rs`.
  - Why: It was an unused, untested edge-snapping resolver; wiring it up would be
    a behavior change, so the spec chose deletion over activation.
  - Rust concepts: dead-code removal; keeping the remaining `TILE_SIZE`/
    `Colisionable` imports that are still used by other methods.
  - Watch out: `try_move` used `TILE_SIZE`, `Rect`, `Vec2` — none of those imports
    became fully unused (other methods still use them), so no import cleanup was
    needed in `level.rs`.

#### Box 15 — Delete `World::spawn_entities`

- [x] done
- **Acceptance**: `client/src/world/game.rs` no longer contains `spawn_entities`
  nor its `#[allow(dead_code)]` attribute; the crate compiles.
- **Test**: `grep -rn "spawn_entities" client/src` returns no matches;
  `cargo test -p metroidvania-rust` still passes.
- **Explain**
  - What: Removed the `spawn_entities` method and its `#[allow(dead_code)]` + TODO
    from `game.rs`, and dropped the now-unused `TILE_SIZE` and `EntityType`
    imports.
  - Why: It duplicated `load_current_level`'s spawn loop and was called nowhere;
    deleting it (per spec) removes zero reachable behavior.
  - Rust concepts: dead-code removal; pruning unused `use` items so `clippy
    -D warnings` stays clean.
  - Watch out: `spawn_entities` was the only consumer of `TILE_SIZE` and
    `EntityType` in `game.rs`; leaving those imports would have tripped
    `unused_imports`.

#### Box 16 — Dead-code grep verification

- [x] done
- **Acceptance**: `grep -rn "try_move\|spawn_entities" client/src` returns no
  matches; `grep -rn "allow(dead_code)" client/src` returns no matches for
  either function.
- **Test**: the two `grep` commands above.
- **Explain**
  - What: Verified by search that `try_move`, `spawn_entities`, and their
    `#[allow(dead_code)]` attributes no longer appear in `client/src`.
  - Why: Confirms the two dead functions (and their allow attributes) are fully
    removed, not just renamed.
  - Rust concepts: n/a (grep verification); the `#[allow(dead_code)]` attribute
    is gone because the code it suppressed no longer exists.
  - Watch out: A stray comment or string mentioning the names would be a false
    positive; a clean grep is the acceptance signal.

### Group 7 — Quality gates (criteria 20–22)

#### Box 17 — `cargo fmt` clean

- [x] done
- **Acceptance**: `cargo fmt --check --all` reports no diffs.
- **Test**: `cargo fmt --check --all`.
- **Explain**
  - What: Hand-formatted the edited files to match `rustfmt` style (import list
    reflow, 4-space indent, single blank line between items).
  - Why: Keeps the tree `fmt`-clean so CI passes.
  - Rust concepts: formatting only (no semantic change).
  - Watch out: Verified green by the orchestrator (`cargo fmt --check --all` exits 0).

#### Box 18 — `cargo clippy` clean

- [x] done
- **Acceptance**: `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
- **Test**: `cargo clippy --all-targets --workspace -- -D warnings`.
- **Explain**
  - What: Removed the now-unused `TILE_SIZE` and `EntityType` imports from
    `game.rs` so `unused_imports` would not trip `-D warnings`.
  - Why: The phase's quality gate runs clippy with warnings denied; dead imports
    left behind by `spawn_entities` removal would fail it.
  - Rust concepts: unused-import lint; `#[allow(dead_code)]` removal.
  - Watch out: Verified green by the orchestrator (`cargo clippy --all-targets --workspace -- -D warnings` exits 0).

#### Box 19 — `cargo test --workspace` green

- [x] done
- **Acceptance**: `cargo test --workspace` passes (all new tests green; no
  rendering/window code exercised — collision tests build `Rect`/`Vec2` only and
  never call `Level::draw` or a macroquad drawing function).
- **Test**: `cargo test --workspace`.
- **Explain**
  - What: All production changes are in place (PartialEq, postcard dev-dep,
    `enemies_from_data`, initial-load wiring, dead-code deletion) so the tests
    written by the tester should be green.
  - Why: The tests lock behavior and prove the spawn fix and cleanups compile and
    behave as intended.
  - Rust concepts: workspace test invocation; integration + inline `#[cfg(test)]`
    test targets.
  - Watch out: Verified green by the orchestrator (`cargo test --workspace` passes: 23 tests).

## Open questions

- None. The `colision` ±2-inset test uses a second hand-built fixture (single
  isolated wall cell) rather than the 3×4 ring; the tester should pin the exact
  `Rect` coordinates so all six sample points miss the corner overlap (the values
  above were verified against `solid_point`'s `(coord / TILE_SIZE) as usize`
  cast). If `Color` in macroquad does not implement `PartialEq`, the
  `enemies_from_data` tests assert `enemy.pos` and the count only.
