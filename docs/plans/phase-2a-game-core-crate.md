# Phase 2a — Extract the game core crate — Implementation Plan

- Status: done
- Spec: `docs/specs/phase-2a-game-core-crate.md`
- Date: 2026-11-23
- Branch: `refactor/phase-2a-game-core-crate`

## Overview

Pure refactor: extract a new library crate `game` that owns all **pure
simulation logic** (config, types, maps, level collision, the `Colisionable`
trait, entity state + physics, and the `enemies_from_data` spawn helper) and
make `client` and `server` depend on it. `game` has **no macroquad dependency**;
its only dependency is `glam = "0.27"` (whose `Vec2` is the *same type*
macroquad re-exports, so there is no conversion at the boundary). Rendering
(`draw`, `Color`) and input (`is_key_down`/`is_key_pressed`) stay in the client.

This is a **behavior-preserving refactor**: the game plays identically. The
visible change is layout — the mixed entity modules are split into (a) a `game`
struct with a concrete `update` physics method, and (b) a client-side
`Drawable` impl. The old `Entity` trait (which coupled `update` + `draw`) is
removed.

**TDD / dependency ordering for this refactor** (differs from the documentation
grouping below — the moved tests cannot compile until the `game` modules they
live in exist):

1. **Skeleton** (Boxes 1–3): `game/Cargo.toml`, root `members`, client/server deps.
2. **Move pure modules** (Boxes 4–11): `config`/`rect`/`types`/`maps`/`level`/
   `traits`/`entities` into `game`; `cargo build -p game` goes green.
3. **Move + write tests** (Boxes 18–19): the moved inline tests relocate with
   their modules (type updates `macroquad::Rect`→`game::Rect`, `vec2`→
   `Vec2::new`, `Color`→`EnemyKind`); the tester writes the **new** tests
   (`Rect::overlaps`, `Player::update` input-driven, `Enemy::set_rand_state`
   mapping). `cargo test -p game` is the gate.
4. **Client render/input split** (Boxes 12–16): `render.rs`, delete old client
   modules, rewire `world/{game,lib}.rs` + `camera.rs` + `network/world.rs`,
   verify `game` is macroquad-free.
5. **Quality gates** (Boxes 21–24): build/test/clippy/fmt across the workspace.

The tester is **not** a no-op: they author the new tests (Box 19) against the
public API below and verify the type updates in the moved tests (Box 18). The
moved tests pass once the `game` crate compiles; the new tests are RED until the
implementer lands the corresponding module.

## Public API (targets for the tester)

Module layout (`game/src/lib.rs`):

```rust
pub mod config;
pub mod types;
pub mod maps;
pub mod level;
pub mod traits;
pub mod rect;
pub mod entities;   // mod.rs: pub mod player; pub mod enemy; pub mod nemesis; pub mod bullet;
```

### `game::config`

```rust
pub const TILE_SIZE: f32 = 40.0;
pub const GRAVEDAD: f32 = 20.0;
pub const SALTO_FUERZA: f32 = -1000.0;
pub const VELOCIDAD: f32 = 400.0;
pub const BULLET_SPEED: f32 = 400.0;
pub const MAP_WIDTH: usize = 20;
pub const MAP_HEIGHT: usize = 12;
pub const TICK_HZ: u32 = 20;   // NEW — used by 2b, unused in 2a
```

### `game::rect`

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self;
    // macroquad-compatible: strict `<` — edge-touching returns false
    pub fn overlaps(&self, other: &Rect) -> bool;
}
```

`overlaps` must be exactly
`self.x < other.x + other.w && other.x < self.x + self.w && self.y < other.y + other.h && other.y < self.y + self.h`.

### `game::types`

`TileType { Empty, Wall, EnemySpawn, BossSpawn }` and
`EntityType { Empty, EnemySpawn, BossSpawn, PotionSpawn }`, both
`#[derive(Clone, Copy, Debug, PartialEq)]`, with unchanged `From<u8>` impls
(fallback to `Empty`). Moves verbatim (Spanish comment → English).

### `game::maps`

```rust
pub struct Map {
    pub grids: Vec<Vec<Vec<Vec<TileType>>>>,
    pub data: Vec<Vec<Vec<Vec<EntityType>>>>,
}
impl Map { pub fn new() -> Self; }
```

Plus the private `LVL_*_*` / `DATA_*_*` grids, moved verbatim.

### `game::level`

```rust
#[derive(PartialEq, Clone, Debug)]
pub struct Level {
    pub grid: Vec<Vec<TileType>>,
    pub width: usize,
    pub height: usize,
    pub data: Vec<Vec<EntityType>>,
}
impl Level {
    pub fn new(grid: Vec<Vec<TileType>>, data: Vec<Vec<EntityType>>) -> Self;
    pub fn pixel_width(&self) -> f32;
    pub fn pixel_height(&self) -> f32;
    pub fn map_colision<T: Colisionable>(&self, objeto: &T) -> bool;
    fn solid_point(&self, x: f32, y: f32) -> bool;
    pub fn colision(&self, rect_x: f32, rect_y: f32, w: f32, h: f32) -> bool;
    pub fn grounded(&self, rect: Rect) -> bool;
    pub fn solid_in_rect(&self, r: Rect) -> bool;
    // pub fn draw(&self)  -> MOVED to client::render
}
```

`Rect` is `game::Rect`. No `macroquad::prelude::*` import.

### `game::traits`

```rust
pub trait Colisionable { fn get_rect(&self) -> Rect; }

pub fn detect_collision<A: Colisionable, B: Colisionable>(obj_a: &A, obj_b: &B) -> bool {
    obj_a.get_rect().overlaps(&obj_b.get_rect())
}
```

No `Entity` trait (removed — its `update`/`draw` halves get new homes).

### `game::entities::player`

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerInput {     // NEW
    pub move_left: bool,     // held
    pub move_right: bool,    // held
    pub jump: bool,          // pressed (edge)
    pub shoot: bool,         // pressed (edge)
}

#[derive(Debug)]
pub struct Player {
    pub id: u8,
    pub pos: Vec2,           // glam::Vec2
    pub vel: Vec2,           // glam::Vec2
    pub w: f32, pub h: f32,
    pub grounded: bool,
    pub dir: f32,
    pub bullets: Vec<Bullet>,
}
impl Player {
    pub fn new(id: u8) -> Self;                       // pos = Vec2::new(100.0, 100.0), dir = 1.0
    pub fn update(&mut self, dt: f32, nivel: &Level, input: &PlayerInput);
}
impl Colisionable for Player { fn get_rect(&self) -> Rect; }
```

`update` keeps the exact current order of operations (zero `vel.x` → held move →
edge jump → update `dir` → edge shoot → update bullets → gravity → X move+collide
→ Y move+collide → recompute `grounded` → retain bullets), with
`is_key_down`/`is_key_pressed` replaced by `input` reads and `vec2` → `Vec2::new`.

### `game::entities::enemy`

```rust
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum EnemyState { Idle, WalkingLeft, WalkingRight, Jumping }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyKind { Enemy, Potion }   // NEW — replaces Color

#[derive(Debug)]
pub struct Enemy {
    pub pos: Vec2,
    pub vel: Vec2,
    pub w: f32, pub h: f32,
    pub grounded: bool,
    pub state: EnemyState,
    pub timer: f32,
    pub alive: bool,
    pub kind: EnemyKind,               // NEW
}
impl Enemy {
    pub fn new(pos: Vec2, kind: EnemyKind) -> Self;   // was (pos, color: Color)
    pub fn die(&mut self);
    fn set_rand_state(&mut self, random: u32);        // random % 4 → state, no macroquad
    fn apply_physics(&mut self, dt: f32, nivel: &Level);
    pub fn update(&mut self, dt: f32, nivel: &Level, random: u32);
}
impl Colisionable for Enemy { fn get_rect(&self) -> Rect; }

/// Spawn helper moved from client/src/world/lib.rs (visibility pub(crate) → pub).
pub fn enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>;
```

`set_rand_state` mapping: `0 → Idle, 1 → WalkingLeft, 2 → WalkingRight, _ → Jumping`.
`enemies_from_data` maps `EnemySpawn → Enemy::new(Vec2::new(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE), EnemyKind::Enemy)`,
`PotionSpawn → ... EnemyKind::Potion`, everything else skipped.

### `game::entities::nemesis`

```rust
#[derive(Debug)]
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,
    pub w: f32, pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
}
impl Nemesis { pub fn new(id: u8, pos: Vec2) -> Self; }
```

### `game::entities::bullet`

```rust
#[derive(Debug)]
pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub active: bool,
    pub owner_id: u8,
}
impl Bullet {
    pub fn new(pos: Vec2, dir_x: f32, id: u8) -> Self;   // vel = Vec2::new(dir_x * BULLET_SPEED, 0.0)
    pub fn check_collision<T: Colisionable>(&self, target: &T) -> bool;
    pub fn update(&mut self, dt: f32, nivel: &Level);
}
impl Colisionable for Bullet { fn get_rect(&self) -> Rect; }  // Rect::new(pos.x, pos.y, 5.0, 5.0)
```

### Client-only (targets for the tester, but lives in `client`)

```rust
// client/src/render.rs
pub trait Drawable { fn draw(&self); }
impl Drawable for game::level::Level;                  // GRAY wall tiles
impl Drawable for game::entities::player::Player;      // RED rect
impl Drawable for game::entities::enemy::Enemy;        // BLUE (Enemy) / GREEN (Potion)
impl Drawable for game::entities::nemesis::Nemesis;    // ORANGE + eye + "P: id"
impl Drawable for game::entities::bullet::Bullet;      // YELLOW circle
```

`World::update` builds the `PlayerInput` from macroquad keys
(`is_key_down(Left/Right)`, `is_key_pressed(Space/Z)`) and an enemy random draw
(`macroquad::rand::gen_range(0, 4)`) and passes them into the entities' concrete
`update` methods.

## Test cases (tester writes these first)

Legend: **[MOVED]** = relocated with its module; passes once `game` compiles
(implementer moves it; tester verifies the type updates). **[NEW]** = genuinely
new, RED until the implementer lands the noted module.

### Moved tests (17 total)

| From | Test | To | Type update |
| --- | --- | --- | --- |
| `client/src/types.rs` | `tile_type_from_u8` | `game/src/types.rs` | none |
| `client/src/types.rs` | `entity_type_from_u8` | `game/src/types.rs` | none |
| `client/src/maps.rs` | `map_new_shape` | `game/src/maps.rs` | none (`crate::` paths unchanged) |
| `client/src/maps.rs` | `map_new_conversion_spot_checks` | `game/src/maps.rs` | none |
| `client/src/level.rs` | 10 `solid_in_rect_*` / `grounded_*` / `colision_*` | `game/src/level.rs` | `use macroquad::prelude::Rect` → `use crate::rect::Rect` |
| `client/src/world/lib.rs` | 3 `enemies_from_data_*` | `game/src/entities/enemy.rs` | `vec2(..)` → `Vec2::new(..)`; `BLUE`/`GREEN` → `EnemyKind::Enemy`/`EnemyKind::Potion` |

Notes for the moved `enemies_from_data` tests:

- `Enemy` does not derive `PartialEq`, so assert on `enemy.pos` (`glam::Vec2` is
  `PartialEq`) and `len()`, as before.
- **[recommended addition]** Now that `EnemyKind` (a `PartialEq` enum) replaces
  `Color` (which was never compared), the tester should strengthen
  `enemies_from_data_spawns_enemy_and_potion` to also assert
  `enemies[0].kind == EnemyKind::Enemy` and `enemies[1].kind == EnemyKind::Potion`.
  This closes the Phase-1 gap where the color assertion had to be skipped.

### New tests (10 total) — RED until implemented

**`game/src/rect.rs` (4) — RED until Box 4**

- `rect_new_sets_fields_in_order` — `Rect::new(1.0, 2.0, 3.0, 4.0)` has
  `x==1.0, y==2.0, w==3.0, h==4.0`.
- `rect_overlaps_true_when_overlapping` — `Rect::new(0.0,0.0,10.0,10.0)` overlaps
  `Rect::new(5.0,5.0,10.0,10.0)` → `true`.
- `rect_overlaps_false_when_apart` — `Rect::new(0.0,0.0,10.0,10.0)` vs
  `Rect::new(20.0,20.0,5.0,5.0)` → `false`.
- `rect_overlaps_false_when_edges_touch` — strict `<`: `Rect::new(0.0,0.0,10.0,10.0)`
  vs `Rect::new(10.0,0.0,10.0,10.0)` → `false`; and `Rect::new(0.0,0.0,10.0,10.0)`
  vs `Rect::new(0.0,10.0,10.0,10.0)` → `false`.

**`game/src/entities/player.rs` (5) — RED until Box 10**

Fixture: `fn empty_level() -> Level` = `Level::new(vec![vec![TileType::Empty; 12]; 12], vec![vec![EntityType::Empty; 12]; 12])`. (12×12 = 480×480 px, so the player's spawn at (100,100) with w=30/h=38 is fully in-bounds; a 3×3 grid is too small — the player spawns out-of-bounds and `colision` treats that as solid, permanently stuck.)
Use `let dt = 1.0 / 60.0;` and `Player::new(0)` (starts at `pos = (100.0, 100.0)`, `dir = 1.0`).
Build `PlayerInput` as a struct literal (or `PlayerInput { move_right: true, ..Default::default() }`).

- `player_input_move_right_moves_right` — `move_right: true` → `player.pos.x > 100.0`
  (and `dir` stays `1.0`).
- `player_input_move_left_moves_left` — `move_left: true` → `player.dir == -1.0`
  and `player.pos.x < 100.0`.
- `player_input_jump_when_grounded_moves_up` — set `player.grounded = true`, then
  `jump: true` → `player.pos.y < 100.0` (net `SALTO_FUERZA*dt + GRAVEDAD*dt` is
  negative; `grounded` recomputes to `false` on the empty grid).
- `player_input_jump_ignored_when_airborne` — `player.grounded = false`, `jump: true`
  → `player.pos.y > 100.0` (jump guarded by `&& self.grounded`; gravity wins).
- `player_input_shoot_spawns_bullet` — `shoot: true` (dir defaults `1.0`) →
  `player.bullets.len() == 1`, `bullets[0].owner_id == 0`,
  `bullets[0].vel == Vec2::new(BULLET_SPEED, 0.0)`.

**`game/src/entities/enemy.rs` (1, recommended) — RED until Box 11**

- `enemy_set_rand_state_maps_random_mod_4` — `Enemy::new(Vec2::new(0.0,0.0), EnemyKind::Enemy)`;
  `set_rand_state(0)` → `Idle`, `(1)` → `WalkingLeft`, `(2)` → `WalkingRight`,
  `(3)` → `Jumping`, `(4)` → `Idle`, `(255)` → `Jumping` (assert via `EnemyState`,
  which derives `PartialEq`). Reaches the private `set_rand_state` because it is in
  the same module's `#[cfg(test)]` child.

### Unchanged

- `protocol/tests/round_trip.rs` (6 tests) — untouched, still passes (criterion 21).

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields. Box numbering follows the execution order (skeleton → modules →
tests → client split → gates); the group headings below are thematic.

### Group 1 — Workspace & crate (criteria 1–5)

#### Box 1 — Create `game/Cargo.toml`

- [ ] done
- **Acceptance**: `game/Cargo.toml` exists with `[package] name = "game"`,
  `version = "0.1.0"`, `edition = "2024"`, a single `[dependencies]` entry
  `glam = "0.27"`, and `[lints] workspace = true`. It declares **no** `macroquad`,
  `postcard`, `laminar`, or `serde`.
- **Test**: `grep -nE "macroquad|postcard|laminar|serde" game/Cargo.toml` returns
  no matches; `grep -n "glam" game/Cargo.toml` shows `glam = "0.27"`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 2 — Register `game` in root workspace members

- [ ] done
- **Acceptance**: root `Cargo.toml` `[workspace] members` is exactly
  `["server", "client", "protocol", "game"]`; `cargo metadata --no-deps` lists a
  `game` package.
- **Test**: `grep -n "members" Cargo.toml`; `cargo metadata --no-deps --format-version 1 | grep -o '"name":"game"'`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 3 — Add `game` dependency to `client` and `server`

- [ ] done
- **Acceptance**: both `client/Cargo.toml` and `server/Cargo.toml` declare
  `game = { path = "../game" }` under `[dependencies]`.
- **Test**: `grep -n 'game = { path = "../game" }' client/Cargo.toml server/Cargo.toml`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group 2 — Simulation moved into `game` (criteria 6–13, 17–18)

#### Box 4 — `game/src/lib.rs` + `config.rs` + `rect.rs`

- [ ] done
- **Acceptance**: `game/src/lib.rs` declares `pub mod config; pub mod types; pub
  mod maps; pub mod level; pub mod traits; pub mod rect; pub mod entities;`
  (stub/empty files for not-yet-moved modules are acceptable so `cargo build -p
  game` compiles); `config.rs` defines the seven constants (unchanged values)
  **and** `TICK_HZ: u32 = 20`; `rect.rs` defines `Rect { x, y, w, h }` with
  `new(x, y, w, h)` and strict-`<` `overlaps`.
- **Test**: `cargo build -p game`; `cargo test -p game rect_` (after Box 19's
  `rect_*` tests are written).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 5 — Move `types.rs`

- [ ] done
- **Acceptance**: `game/src/types.rs` defines `TileType`/`EntityType` and their
  `From<u8>` impls with unchanged mappings (fallback-to-`Empty`), English comment;
  the two `From<u8>` inline tests live there.
- **Test**: `cargo test -p game tile_type_from_u8 entity_type_from_u8`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 6 — Move `maps.rs`

- [ ] done
- **Acceptance**: `game/src/maps.rs` defines `Map` + `Map::new` with the same
  `LVL_*_*`/`DATA_*_*` grids and dimensions; its two inline tests live there.
- **Test**: `cargo test -p game map_new`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 7 — Move `level.rs` (collision only)

- [ ] done
- **Acceptance**: `game/src/level.rs` defines `Level` with `new`, `pixel_width`,
  `pixel_height`, `map_colision`, `solid_point`, `colision`, `grounded`,
  `solid_in_rect`, and **no** `draw`; `Rect` is `game::Rect`; its 10 inline tests
  use `use crate::rect::Rect`.
- **Test**: `cargo test -p game solid_in_rect` and `cargo test -p game grounded`
  and `cargo test -p game colision`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 8 — Move `traits.rs`

- [ ] done
- **Acceptance**: `game/src/traits.rs` defines `Colisionable { get_rect -> Rect }`
  and `detect_collision`; it does **not** define an `Entity` trait with `draw`.
- **Test**: `grep -rn "trait Entity" game/src` returns no matches; `cargo build -p game`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 9 — Move `entities/bullet.rs` + `entities/nemesis.rs`

- [ ] done
- **Acceptance**: `game/src/entities/bullet.rs` defines `Bullet` with `new`,
  `check_collision`, `update`, and `Colisionable` (`Rect::new(pos.x, pos.y, 5.0,
  5.0)`), no `draw`; `game/src/entities/nemesis.rs` defines `Nemesis` with `new`,
  no `draw`, no `update`; `game/src/entities/mod.rs` declares all four modules.
- **Test**: `cargo build -p game`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 10 — Move `entities/player.rs` (input-driven)

- [ ] done
- **Acceptance**: `game/src/entities/player.rs` defines `PlayerInput` (held
  `move_left`/`move_right`, edge `jump`/`shoot`), `Player`, `Player::new`, and
  `Player::update(&mut self, dt, &Level, &PlayerInput)`; `update` contains no
  `is_key_down`/`is_key_pressed` and preserves the current order of operations;
  `vec2` → `Vec2::new`.
- **Test**: `cargo test -p game player_input_`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 11 — Move `entities/enemy.rs` (+ `enemies_from_data`)

- [ ] done
- **Acceptance**: `game/src/entities/enemy.rs` defines `EnemyState`, `EnemyKind`,
  `Enemy` with `kind` (no `color`), `Enemy::new(pos, kind)`, `die`, private
  `set_rand_state(random: u32)`, private `apply_physics`, and
  `update(&mut self, dt, &Level, random: u32)` (no `macroquad::rand`); plus the
  `pub fn enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>` helper moved
  from `client/src/world/lib.rs` (`EnemySpawn → EnemyKind::Enemy`, `PotionSpawn →
  EnemyKind::Potion`).
- **Test**: `cargo test -p game enemies_from_data` and `cargo test -p game
  enemy_set_rand_state`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group 3 — Client/rendering split (criteria 14–17)

#### Box 12 — Add `client/src/render.rs`

- [x] done
- **Acceptance**: `client/src/render.rs` defines a local `Drawable` trait and
  implements `draw` for `game::Level`, `game::Player`, `game::Enemy`,
  `game::Nemesis`, `game::Bullet`, reproducing today's visuals (wall tiles GRAY,
  player RED, enemy BLUE for `EnemyKind::Enemy` / GREEN for `EnemyKind::Potion`,
  nemesis ORANGE + eye + `P: id` text, bullet YELLOW circle).
- **Test**: `grep -n "impl Drawable" client/src/render.rs` shows five impls;
  `cargo build -p metroidvania-rust` (once Boxes 13–16 land).
- **Explain**
  - What: Created `client/src/render.rs` with a local `pub trait Drawable { fn
    draw(&self); }` and five `impl Drawable for game::…` blocks (Level, Player,
    Enemy, Nemesis, Bullet), each reproducing the original `draw` bodies. The
    `EnemyKind → Color` mapping (Enemy→BLUE, Potion→GREEN) is a `match` inside the
    Enemy impl. Imports come from `game::*` for the entity/level/types and
    `macroquad::prelude::*` for `Color`/`draw_*`.
  - Why: The `draw` methods previously lived in `game`'s predecessor modules and
    pulled macroquad (`Color`, `draw_rectangle`, …) into the simulated entity
    code. Since `game` must stay macroquad-free, rendering is split out into the
    client, which is the only crate allowed to touch graphics.
  - Rust concepts: traits; the orphan rule — `Drawable` is defined locally so it
    may be implemented for the foreign `game` types; `match` for the enum-to-Color
    mapping; `impl` blocks for a trait on foreign types.
  - Watch out: The orphan rule is exactly why this works and importing a `game`
    trait would not. The Color mapping must stay inside this impl (not `game`) to
    keep `Color` out of the simulation crate. Five impls, one per entity plus the
    level.

#### Box 13 — Delete moved client modules + rewire `main.rs`

- [x] done
- **Acceptance**: `client/src/{config.rs, types.rs, maps.rs, level.rs, traits.rs,
  entities/}` no longer exist; `client/src/main.rs` declares `mod camera; mod
  network; mod render; mod world;` (no `config`/`types`/`maps`/`level`/`traits`/
  `entities` mod lines) and uses `game::maps::Map`.
- **Test**: `ls client/src` and `grep -n "^mod" client/src/main.rs`.
- **Explain**
  - What: Deleted `client/src/{config,types,maps,level,traits}.rs` and
    `client/src/entities/` (performed by the orchestrator, since the implementer's
    shell denies `rm`). Rewrote `client/src/main.rs`: dropped the six stale `mod`
    lines and `use crate::maps::Map;`, added `mod hud;`/`mod render;`, and import
    `Map` as `use game::maps::Map;`.
  - Why: Those modules now live in `game`; keeping client copies would fork the
    simulation and reintroduce the exact duplication Phase 1 removed. `main.rs`
    must track the new layout or the crate fails to compile (stale `mod` paths).
  - Rust concepts: module system (`mod` declarations and `use` paths); extern-crate
    paths (`game::maps::Map` vs the old `crate::maps::Map`).
  - Watch out: `hud.rs` was empty and previously un-declared; it is now declared
    (`mod hud;`) but contributes nothing — harmless, and ready for later use. The
    `use game::maps::Map` path change is the single functional import edit in this
    box.

#### Box 14 — Rewire `client/src/world/game.rs`

- [x] done
- **Acceptance**: `world/game.rs` imports `game::{entities::*, level::Level,
  maps::Map}` (instead of `crate::…`) and renders via `crate::render::Drawable`;
  `World::update` builds a `PlayerInput` from macroquad keys and an enemy random
  draw, then calls the entities' concrete `update` methods; the old
  `crate::traits::Entity` import is gone. The `crate::world::lib` import drops
  `enemies_from_data` (it now lives in `game`, deleted from `world::lib` in
  Box 15), and `World::new` calls `game::entities::enemy::enemies_from_data(&levels[0][0].data)`
  instead.
- **Test**: `grep -n "game::" client/src/world/game.rs`; `grep -n "enemies_from_data" client/src/world/game.rs` shows the `game::entities::enemy::enemies_from_data` call and no `world::lib` re-export of it; `cargo build -p
  metroidvania-rust`.
- **Explain**
  - What: Re-imported every entity/level/map type from `game::*` (bullet, enemy,
    enemy::enemies_from_data, nemesis, player::{Player,PlayerInput}, level::Level,
    maps::Map), dropped `crate::traits::Entity`, and added `use crate::render::Drawable;`.
    `World::new` now calls `game::entities::enemy::enemies_from_data(...)`.
    `World::update` builds a `PlayerInput` from `is_key_down(Left/Right)` /
    `is_key_pressed(Space/Z)` and passes it to `player.update(dt, level, &input)`;
    it draws a fresh `macroquad::rand::gen_range(0, 4)` per enemy and passes it to
    `enemy.update(dt, level, random)`.
  - Why: The old entity modules are gone (Box 13), and the updated `Player::update`
    / `Enemy::update` signatures in `game` are input-driven — the client is now
    responsible for sampling macroquad input/randomness each frame and handing it
    to the engine.
  - Rust concepts: struct-literal construction of `PlayerInput` (it derives
    `Default`, but we build it fully); `&` borrow of `level` from `self` inside a
    block to satisfy the borrow checker while also mutating `self.player`/`self.enemies`;
    method-call dispatch into the extern `game` crate.
  - Watch out: The `PlayerInput` is rebuilt every frame (macroquad's edge-detection
    lives in `is_key_pressed`, not in `game`). The enemy random draw is now taken
    every frame rather than only when `timer >= 0.1`; the consumed distribution is
    unchanged because the extra draws are simply ignored until the timer elapses
    (noted in Open questions).

#### Box 15 — Rewire `client/src/world/lib.rs` + `camera.rs`

- [x] done
- **Acceptance**: `world/lib.rs` keeps `update_enemy_bullets_position`,
  `check_player_death`, `handle_death`, `load_current_level`,
  `check_level_transitions`; it calls `game::entities::enemy::enemies_from_data`
  (its own copy deleted) and imports `game::{config::TILE_SIZE, entities::enemy::Enemy,
  types::EntityType}`; `handle_death` sets `player.pos = Vec2::new(100.0, 100.0)`;
  `camera.rs` imports `game::level::Level` instead of `crate::level::Level`
  (its `Vec2`/`vec2` stay macroquad, which is the same `glam::Vec2`).
- **Test**: `grep -rn "enemies_from_data" client/src` shows the import/use only;
  `grep -rn "use game::level::Level" client/src/camera.rs`; `cargo build -p metroidvania-rust`.
- **Explain**
  - What: In `world/lib.rs`, removed the local `enemies_from_data` function and its
    three inline tests, replaced the `crate::config`/`crate::types`/`crate::traits`/
    `crate::entities` imports with `use game::entities::enemy::enemies_from_data;`.
    `load_current_level` now calls the `game` helper. In `camera.rs`, changed
    `use crate::level::Level` → `use game::level::Level`.
  - Why: The spawn helper and the level type moved into `game`; keeping a local
    copy would duplicate the mapping logic. `camera.rs` only referenced `Level`'s
    `pixel_width`/`pixel_height`, so a one-line import switch is all it needs —
    its `Vec2`/`vec2` remain macroquad's re-exported `glam::Vec2`, the identical
    type.
  - Rust concepts: deleting a now-redundant `pub(crate)` helper; consolidating
    imports; recognizing type identity (`macroquad::Vec2 == glam::Vec2`) at the
    crate boundary.
  - Watch out: `handle_death` still sets `world.player.pos = vec2(100.0, 100.0)`
    (macroquad `vec2`), not `Vec2::new` — functionally identical, and the plan's
    acceptance text was illustrative of the value, not the literal constructor.
    The three `enemies_from_data` tests were deleted here because they relocated to
    `game/src/entities/enemy.rs` (Box 11/18).

#### Box 16 — Rewire `client/src/network/world.rs`

- [x] done
- **Acceptance**: the two entity imports at the top of `client/src/network/world.rs`
  become `use game::entities::bullet::Bullet;` and `use game::entities::nemesis::Nemesis;`
  (replacing `crate::entities::…`, which no longer exist after Box 13). No other
  change to this file: the calls `Nemesis::new(...)`, the `.pos`/`.last_dir`/`.coords`
  field accesses, and `Bullet::new(...)` keep working because `game::Bullet`/
  `game::Nemesis` expose the same fields/constructors, and the local `vec2(x, y)`
  calls return `macroquad`'s re-exported `glam::Vec2`, the identical type the `game`
  entities use.
- **Test**: `grep -n "use game::entities" client/src/network/world.rs` shows both
  `bullet::Bullet` and `nemesis::Nemesis`; `grep -n "crate::entities" client/src`
  returns no matches; `cargo build -p metroidvania-rust`.
- **Explain**
  - What: Swapped the two entity imports at the top of `network/world.rs` from
    `crate::entities::{bullet::Bullet, nemesis::Nemesis}` to the `game::entities::`
    equivalents. No other lines changed.
  - Why: The `crate::entities` modules were deleted (Box 13); the types now live in
    `game`. Since `game::Bullet`/`game::Nemesis` keep identical fields and
    constructors, the body needs no edits.
  - Rust concepts: path-based imports; type identity across crates (`glam::Vec2`
    re-exported by macroquad means `vec2(...)` and `game`'s `Vec2` are the same
    type at the boundary).
  - Watch out: `grep -rn "crate::entities"` now returns nothing anywhere in the
    client, confirming the old module tree is fully gone and nothing still
    references it.

### Group 4 — Input-driven physics verification (criteria 4, 18–19)

#### Box 17 — Verify `game` is macroquad-free

- [ ] done
- **Acceptance**: `grep -rn "macroquad" game/` returns no matches; `game/src`
  contains no `draw_*`, `is_key_down`, `is_key_pressed`, or `Color` usage;
  `grep -rn "macroquad::rand" game/` returns nothing (enemy randomness is the
  injected `random: u32` param).
- **Test**: the `grep` commands above.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group 5 — Tests (criteria 20–21)

#### Box 18 — Moved tests green in `game`

- [ ] done
- **Acceptance**: the 17 moved inline tests (types/maps/level/enemies_from_data)
  live in `game` with `macroquad::Rect`→`game::Rect`, `vec2`→`Vec2::new`,
  `Color`→`EnemyKind` updates, and pass.
- **Test**: `cargo test -p game` (the `types`/`maps`/`level`/`enemies_from_data`
  subset).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 19 — New tests green in `game`

- [ ] done
- **Acceptance**: the 10 new tests (`rect_*` strict-`<`, `player_input_*`
  input-driven, `enemy_set_rand_state_maps_random_mod_4`) live in `game` and pass;
  none of them calls `draw` or any macroquad function (they build `Rect`/`Vec2`
  only).
- **Test**: `cargo test -p game rect_`, `cargo test -p game player_input_`,
  `cargo test -p game enemy_set_rand_state`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

#### Box 20 — Protocol round-trip tests untouched

- [ ] done
- **Acceptance**: `protocol/tests/round_trip.rs` is unchanged and passes
  (`cargo test -p protocol`).
- **Test**: `cargo test -p protocol`.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Group 6 — Quality gates (criteria 22–25)

#### Box 21 — `cargo build --workspace` green

- [x] done
- **Acceptance**: `cargo build --workspace` succeeds from the root (all four
  crates: `server`, `client`, `protocol`, `game`).
- **Test**: `cargo build --workspace`.
- **Explain**
  - What: `cargo build --workspace` succeeds. The client now `use`s `game`
    (Boxes 13–16), so its `unused dependency: game` warning is gone. The server's
    `unused dependency: game` warning remains, as expected — the server does not
    `use game` until 2b (an unused dependency is a manifest warning, not a clippy
    error).
  - Why: Compiling all four members end-to-end is the first whole-workspace check
    that the refactor holds together (the tester only built `-p game`).
  - Rust concepts: workspace members; cargo manifest `unused_dependencies` lint
    (`warn` by default, distinct from `clippy`).
  - Watch out: The remaining `unused dependency: game` on `server` is intentional
    and will disappear in 2b when the server starts simulating with `game`. Do not
    remove the dependency to silence it.

#### Box 22 — `cargo test --workspace` green

- [x] done
- **Acceptance**: `cargo test --workspace` passes (moved + new `game` tests green;
  no rendering/window/GPU code exercised by any test).
- **Test**: `cargo test --workspace`.
- **Explain**
  - What: `cargo test --workspace` passes: 27 `game` unit tests + 6 `protocol`
    round-trip tests all green; the client/server bins report 0 tests (their old
    inline tests were deleted with the old modules).
  - Why: The moved (types/maps/level/enemies_from_data) and new (rect_*,
    player_input_*, enemy_set_rand_state) tests now live in `game`, and none
    touches macroquad/rendering, so they run headless in CI.
  - Rust concepts: `#[cfg(test)]` modules; unit tests vs integration tests (the
    `protocol/tests/round_trip.rs` integration suite).
  - Watch out: The total moved from "client's inline tests" to "game's tests" — no
    test was dropped, only relocated. The three `enemies_from_data` tests from
    `world/lib.rs` are the same three now in `game/src/entities/enemy.rs` (plus a
    strengthened `kind` assertion on the spawn test).

#### Box 23 — `cargo clippy` clean

- [x] done
- **Acceptance**: `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
- **Test**: `cargo clippy --all-targets --workspace -- -D warnings`.
- **Explain**
  - What: `cargo clippy --all-targets --workspace -- -D warnings` is clean. The one
    lint it surfaced was `clippy::new_without_default` on `game::Map` (`Map::new()`
    without a matching `Default` impl); I added `impl Default for Map` that calls
    `Self::new()` in `game/src/maps.rs`.
  - Why: `-D warnings` promotes every clippy warning to an error, matching the
    repo's CI gate; `Map::new` is a pure constructor so a `Default` impl is
    idiomatic and satisfies the lint without behavior change.
  - Rust concepts: clippy lints and `-D warnings`; the `Default` trait and its
    relationship to `new()` (`new_without_default`).
  - Watch out: The server's `unused dependency: game` manifest warning is a cargo
    lint, not a clippy lint, so it does not fail this gate (and would not even under
    `-D warnings`). Only `game` needed a change to reach zero clippy warnings.

#### Box 24 — `cargo fmt` clean

- [x] done
- **Acceptance**: `cargo fmt --check --all` reports no diffs.
- **Test**: `cargo fmt --check --all`.
- **Explain**
  - What: `cargo fmt --all` was run (the initial `--check` reported diffs: import
    reordering and trailing-newline padding across the tester-authored `game`
    files and my new `client` files), after which `cargo fmt --check --all` is
    clean.
  - Why: The repo requires `cargo fmt` cleanliness; the tester's files and the new
    client files hadn't been run through rustfmt yet.
  - Rust concepts: `rustfmt` — import ordering (`use` groups), trailing-newline
    normalization.
  - Watch out: I ran `cargo fmt` (not just `--check`) because the check failed; per
    the task this is the one sanctioned case. It only touched whitespace/import
    ordering — no semantic change.

## Open questions

- **`enemies_from_data` home**: the spec allows `game::entities::enemy::enemies_from_data`
  (chosen here) or a separate `game::spawn` module. Either is fine; pick one and
  keep the import in `client/src/world/lib.rs` consistent.
- **`PlayerInput` derives**: `#[derive(Debug, Clone, Copy, Default)]` is
  recommended so `World::update` can build it and tests can use
  `PlayerInput { ..Default::default() }`. If the implementer omits `Default`, the
  tests must build full struct literals.
- **Enemy random-draw cadence**: passing `macroquad::rand::gen_range(0, 4)` every
  frame (vs only when `timer >= 0.1`) produces a draw that is ignored until the
  timer elapses. This does not change the consumed distribution but is worth
  noting in the `Explain` block as a minor behavioral nuance (uniform `0..4`
  preserved).
- **`camera.rs` change**: the spec's list of `client` changes (now corrected)
  correctly notes that `camera.rs` DOES change — exactly one edit,
  `use crate::level::Level` → `use game::level::Level`; its `Vec2`/`vec2` remain
  macroquad (same `glam::Vec2`). Flagged in Box 15.
