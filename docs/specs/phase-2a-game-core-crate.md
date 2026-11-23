# Phase 2a — Extract the game core crate

- Status: accepted
- Date: 2026-11-23
- Branch: `refactor/phase-2a-game-core-crate`

## Summary

ADR-001 (Accepted) chose **server-authoritative movement**: the client sends
inputs, the server simulates movement/collision at a fixed tick, and broadcasts
authoritative state. For the server to simulate with *the same* code the client
uses, that simulation code must live in a crate both can depend on.

This phase is that prerequisite. Extract a new library crate `game` that owns all
**pure simulation logic** — levels, types, maps, collision, entity state and
physics — and make both `client` and `server` depend on it. The `game` crate must
have **no macroquad dependency** (the server cannot link a graphics/input
library), so rendering (`draw_*`, `Color`) and input (`is_key_down`) stay in the
client.

This is a **pure refactor: no behavior change**. The game plays exactly as
before; only the code layout and the math types at the simulation boundary
change.

### Key decisions (read the code, decided here)

1. **Boundary.** Into `game`: `config`, `types`, `maps`, `level` (collision
   only), the `Colisionable` trait, and the entity data + physics
   (`Player`/`Enemy`/`Nemesis`/`Bullet`). Stays in `client`: macroquad rendering
   (`draw`), input (`is_key_down`/`is_key_pressed`), the `camera`, `network`, and
   the `World` orchestration that ties them together. The entities currently mix
   the two: **split each entity** into (a) a `game` struct with a concrete
   `update` physics method, and (b) a client-side `Drawable` impl that renders
   it. The `Entity` trait (which couples `update` + `draw`) is removed: its two
   halves get new homes (see Data model).
2. **Math dependency.** `game` uses **`glam` 0.27** for `Vec2` — already in
   `Cargo.lock` because macroquad re-exports `glam::Vec2`, so `macroquad::Vec2`
   and `game::Vec2` are **the same type** (no conversion at the boundary, no
   numeric drift). `Rect` is **not** in glam, so `game` defines a small local
   `Rect { x, y, w, h }` with `new()` and `overlaps()` whose semantics match
   macroquad's. `Color` stays client-only.
3. **Input.** The player's `update` becomes **input-driven**: it takes a
   `PlayerInput` struct (held move + edge-triggered jump/shoot) instead of
   reading `is_key_down`/`is_key_pressed`. No macroquad input lives inside
   `game`. The full input *sampling/queueing* refactor is step 2b; here the
   client just builds a `PlayerInput` from macroquad keys each frame and passes
   it in. Same for the enemy's randomness: `Enemy::update` takes an injected
   random draw instead of calling `macroquad::rand` (see Behavior / edge cases).
4. **Tick constant.** `game::config` gains `TICK_HZ: u32 = 20`, so the later
   20 → 60 Hz change is one line (owner's technical-debt requirement). Unused in
   2a; used by 2b.

## Goals / non-goals

### Goals

1. A new workspace member `game` (`game/Cargo.toml`, `edition = "2024"`,
   library-only), listed in the root `[workspace] members` alongside `server`,
   `client`, `protocol`.
2. `game` compiles with **no macroquad dependency**; its only dependency is
   `glam = "0.27"` (plus nothing else). No `postcard`, no `laminar`, no `serde`,
   no `Color`.
3. Move the pure simulation modules into `game`: `config` (constants + new
   `TICK_HZ`), `types` (`TileType`/`EntityType` + `From<u8>`), `maps` (`Map` +
   `LVL_*`/`DATA_*` grids), `level` (`Level` + collision, **without** `draw`),
   `traits` (`Colisionable`), and `entities/` (structs + physics, **without**
   `draw`).
4. Split rendering from simulation: the `draw` impls for `Level`, `Player`,
   `Enemy`, `Nemesis`, `Bullet` move to a client-side `render` module using a
   local `Drawable` trait; `Color`/`draw_*` stay in `client`.
5. Make the player physics input-driven (`PlayerInput`) and the enemy AI
   randomness injected (no `macroquad::rand` inside `game`).
6. Add `TICK_HZ: u32 = 20` to `game::config`.
7. `client` and `server` both add `game = { path = "../game" }`; `client`'s
   remaining code imports `game::*` instead of its now-deleted `config`/`types`/
   `maps`/`level`/`traits`/`entities` modules.
8. Move the existing inline tests for `types`, `maps`, `level`, and
   `enemies_from_data` into `game`, updating `macroquad::Rect`/`vec2` to
   `game::Rect`/`glam::Vec2`. The protocol round-trip tests stay in `protocol`.

### Non-goals (explicitly out of scope)

- **No behavior change.** This is a pure refactor; the game plays identically.
- **No server simulation loop / tick** — that is step 2b. The server only gains
  the `game` dependency (no logic change) in this phase.
- **No client prediction / reconciliation** — step 2c.
- **No full input refactor** (input queue, tick-sampled inputs, buffering) —
  2b. 2a only makes the player physics *take* explicit inputs.
- **No renaming of `colision`** (misspelling), and **no fixing** the existing
  `solid_in_rect` vs `solid_point` out-of-bounds asymmetry — both are documented
  and locked by tests in Phase 1 and remain unchanged.
- **No serialization of game state** — entities are not `Serialize`/`Deserialize`
  in this phase.
- **No change to `GamePacket`** — the wire protocol is untouched.

## Behavior

No player-visible change. Observable behavior is for developers and CI:

- `game` builds as a macroquad-free library; `cargo build --workspace` succeeds.
- The client still runs, renders, and accepts input exactly as before.
- `cargo test --workspace` passes with the moved tests green.
- `cargo clippy --all-targets --workspace -- -D warnings` is clean.

### Rendering/input split

Each entity's `draw` moves to a client `render` module:

```
client/src/render.rs
  pub trait Drawable { fn draw(&self); }
  impl Drawable for game::level::Level         // draws GRAY wall tiles
  impl Drawable for game::entities::player::Player   // RED rect
  impl Drawable for game::entities::enemy::Enemy     // BLUE or GREEN by kind
  impl Drawable for game::entities::nemesis::Nemesis // ORANGE + eye + "P: id" text
  impl Drawable for game::entities::bullet::Bullet   // YELLOW circle
```

`World::draw` then calls `level.draw()` / `player.draw()` / etc. through the
`Drawable` trait (imported locally), which is valid because `Drawable` is a
local trait implemented for the foreign `game` types (allowed by the orphan
rule).

### Edge cases (assert current behavior; do not change it)

- **`Vec2` identity.** `game::Vec2` is `glam::Vec2`, the same type macroquad
  re-exports. `pos`/`vel` arithmetic is bit-identical; no conversion or drift is
  introduced. `vec2(x, y)` (macroquad) becomes `Vec2::new(x, y)` (glam) wherever
  game state is constructed.
- **`Rect` semantics.** The local `game::Rect { x, y, w, h }` must expose
  `new(x, y, w, h)` in the same argument order as macroquad's `Rect::new`, and
  `overlaps` must match macroquad's exactly: `self.x < other.x + other.w &&
  other.x < self.x + self.w && self.y < other.y + other.h && other.y < self.y +
  self.h` (strict `<`), so **edge-only touching is not an overlap** (this is what
  `detect_collision` relies on). `Rect` is used only inside game logic; the
  client never renders a `Rect` object, so it never crosses the macroquad
  boundary.
- **`Color` removed from `Enemy`.** `Enemy.color: Color` is deleted; the enemy's
  blue-vs-green distinction is carried by a new `EnemyKind { Enemy, Potion }`
  field. The spawn mapping is preserved: `EntityType::EnemySpawn → EnemyKind::Enemy`
  (rendered BLUE), `EntityType::PotionSpawn → EnemyKind::Potion` (rendered
  GREEN). No visible change. **Note (owner clarification):** `Potion` is a
  *placeholder* — `PotionSpawn` is intended to spawn a collectible `Potion`, not
  an enemy. The green `Enemy` is pre-existing placeholder behavior preserved by
  this refactor; a future phase will introduce a real `Potion` collectible entity
  and retire `EnemyKind::Potion`.
- **Player input mapping.** `PlayerInput` must distinguish held vs edge-triggered
  exactly as today: `move_left`/`move_right` are **held** (map
  `is_key_down(Left/Right)`), `jump` and `shoot` are **pressed** (map
  `is_key_pressed(Space)` / `is_key_pressed(Z)`). The order of operations inside
  `Player::update` is preserved: zero `vel.x` → apply held move → edge jump →
  update `dir` → edge shoot (spawn bullet using the just-updated `dir`) → update
  bullets → gravity → X-axis move+collide → Y-axis move+collide → recompute
  `grounded` → retain active bullets.
- **Enemy randomness.** `Enemy::update` currently calls
  `macroquad::rand::gen_range(0, 4)` every 0.1 s to pick
  `Idle/WalkingLeft/WalkingRight/Jumping`. To keep `game` macroquad-free, the
  random draw is **injected**: `Enemy::update(&mut self, dt, nivel, random: u32)`
  (or equivalent), where `random` is a fresh draw supplied by the caller; the
  mapping `random % 4` → `0→Idle, 1→WalkingLeft, 2→WalkingRight, _→Jumping`
  matches today. The client supplies the draw via `macroquad::rand` (unchanged
  distribution); the server will later supply its own RNG. No RNG lives inside
  `game`.
- **Collision edge cases unchanged.** `solid_in_rect` (out-of-bounds is *not*
  solid), `solid_point`/`colision` (out-of-bounds *is* solid; ±2 px horizontal
  inset; negative-coordinate cast saturates to 0), and `grounded` (1 px-down
  probe at full height) all keep their exact current math; only the `Rect` type
  changes.
- **`colision`/`solid_in_rect`/`grounded` method names and `TILE_SIZE`**
  (`f32 = 40.0`), `GRAVEDAD`, `SALTO_FUERZA`, `VELOCIDAD`, `BULLET_SPEED`,
  `MAP_WIDTH`, `MAP_HEIGHT` values are all unchanged.

## Data model

### New crate: `game`

```
game/
  Cargo.toml        # name = "game", edition = "2024", [dependencies] glam = "0.27"
  src/
    lib.rs          # pub mod config; pub mod types; pub mod maps; pub mod level;
                    # pub mod traits; pub mod rect; pub mod entities;
    config.rs
    rect.rs
    types.rs
    maps.rs
    level.rs
    traits.rs
    entities/
      mod.rs        # pub mod player; pub mod enemy; pub mod nemesis; pub mod bullet;
      player.rs
      enemy.rs
      nemesis.rs
      bullet.rs
```

`game/Cargo.toml` dependencies: `glam = "0.27"` and nothing else. `[lints]
workspace = true` (matching the other crates). No `macroquad`, no `postcard`, no
`laminar`, no `serde`.

### `game::config` (`config.rs`)

Unchanged constants plus one addition:

```rust
pub const TILE_SIZE: f32 = 40.0;
pub const GRAVEDAD: f32 = 20.0;
pub const SALTO_FUERZA: f32 = -1000.0;
pub const VELOCIDAD: f32 = 400.0;
pub const BULLET_SPEED: f32 = 400.0;
pub const MAP_WIDTH: usize = 20;
pub const MAP_HEIGHT: usize = 12;
pub const TICK_HZ: u32 = 20;      // NEW — used by 2b, not 2a
```

### `game::rect` (`rect.rs`)

```rust
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self;
    pub fn overlaps(&self, other: &Rect) -> bool; // macroquad-compatible (strict <)
}
```

### `game::types` (`types.rs`)

`TileType` and `EntityType` enums and their `From<u8>` impls move verbatim (the
`From<u8>` impls and their fallback-to-`Empty` behavior are unchanged). The
Spanish comment in `From<u8>` is replaced with an English comment (code
convention), no logic change.

### `game::maps` (`maps.rs`)

`Map { grids: Vec<Vec<Vec<Vec<TileType>>>>, data: Vec<Vec<Vec<Vec<EntityType>>>>> }`
and `Map::new()` + the `LVL_*_*`/`DATA_*_*` grids move verbatim (comments
translated to English).

### `game::level` (`level.rs`)

`Level` moves with its `PartialEq, Clone, Debug` derive and its collision
methods, **minus `draw`**:

```rust
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

The `use macroquad::prelude::*;` at the top is removed; `Rect` becomes
`game::Rect`.

### `game::traits` (`traits.rs`)

```rust
pub trait Colisionable { fn get_rect(&self) -> Rect; }

pub fn detect_collision<A: Colisionable, B: Colisionable>(obj_a: &A, obj_b: &B) -> bool {
    obj_a.get_rect().overlaps(&obj_b.get_rect())
}
```

The old `Entity` trait is **removed** (see entity split below).

### `game::entities::player` (`player.rs`)

```rust
pub struct PlayerInput {          // NEW
    pub move_left: bool,          // held
    pub move_right: bool,         // held
    pub jump: bool,               // pressed (edge)
    pub shoot: bool,              // pressed (edge)
}

pub struct Player {
    pub id: u8,
    pub pos: Vec2,                // glam
    pub vel: Vec2,                // glam
    pub w: f32, pub h: f32,
    pub grounded: bool,
    pub dir: f32,
    pub bullets: Vec<Bullet>,
}
impl Player {
    pub fn new(id: u8) -> Self;   // pos = Vec2::new(100.0, 100.0)
    pub fn update(&mut self, dt: f32, nivel: &Level, input: &PlayerInput);
}
impl Colisionable for Player { fn get_rect(&self) -> Rect; }
```

`update` is the current `Player::update` body with `is_key_down`/`is_key_pressed`
replaced by reads of `input`, and `vec2` → `Vec2::new`. `draw` is removed
(moved to client). `Colisionable` returns `game::Rect`.

### `game::entities::enemy` (`enemy.rs`)

```rust
pub enum EnemyState { Idle, WalkingLeft, WalkingRight, Jumping }

pub enum EnemyKind { Enemy, Potion }   // NEW — replaces Color

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
    fn set_rand_state(&mut self, random: u32);        // random % 4, no macroquad
    fn apply_physics(&mut self, dt: f32, nivel: &Level);
    pub fn update(&mut self, dt: f32, nivel: &Level, random: u32);
}
impl Colisionable for Enemy { fn get_rect(&self) -> Rect; }
```

`draw` is removed. The `macroquad::rand::gen_range` call is removed; `random` is
injected.

### `game::entities::nemesis` (`nemesis.rs`)

Pure data (no `Entity` impl today, unchanged):

```rust
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,
    pub w: f32, pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
}
impl Nemesis { pub fn new(id: u8, pos: Vec2) -> Self; }
```

`draw` is removed (moved to client). No `update` (none exists today).

### `game::entities::bullet` (`bullet.rs`)

```rust
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

`draw` is removed.

### Spawn helper (moves to `game`)

`enemies_from_data(data: &[Vec<EntityType>]) -> Vec<Enemy>` moves from
`client/src/world/lib.rs` into `game` (e.g. `game::entities::enemy::enemies_from_data`
or a `game::spawn` module). Positions become `Vec2::new(x as f32 * TILE_SIZE,
y as f32 * TILE_SIZE)`; colors become kinds
(`EnemySpawn → Enemy::new(pos, EnemyKind::Enemy)`, `PotionSpawn →
Enemy::new(pos, EnemyKind::Potion)`, everything else skipped). Its three inline
tests move with it. The server can later reuse this to spawn from map data.

### `client` changes (stays in client)

- Delete `client/src/{config.rs, types.rs, maps.rs, level.rs, traits.rs,
  entities/}` and the corresponding `mod` lines in `client/src/main.rs`.
- Add `client/src/render.rs` (the `Drawable` trait + the five `draw` impls and
  the `EnemyKind → Color` mapping).
- `client/src/main.rs` keeps `mod camera; mod network; mod render; mod world;` and
  imports `game::maps::Map`.
- `client/src/world/game.rs` (`World`) imports the entities/level/map from
  `game::*` and uses `crate::render::Drawable` for drawing; its `update` builds a
  `PlayerInput` from macroquad keys and an enemy random draw, then calls the
  entities' concrete `update` methods.
- `client/src/world/lib.rs` keeps `check_level_transitions`,
  `check_player_death`, `handle_death`, `load_current_level`,
  `update_enemy_bullets_position` (they touch `World`/`NetworkClient`/`println!`);
  it calls the `game` spawn helper instead of its own. `handle_death` resets
  `world.player.pos = Vec2::new(100.0, 100.0)`.
- `client/src/camera.rs` changes its `Level` import (`crate::level::Level` →
  `game::level::Level`); its `Vec2` is already the same `glam::Vec2`.
- `client/src/network/world.rs` changes its entity imports
  (`crate::entities::bullet::Bullet`, `crate::entities::nemesis::Nemesis` →
  `game::entities::{bullet::Bullet, nemesis::Nemesis}`); other `network/*` files
  are unchanged.
- `client/Cargo.toml` adds `game = { path = "../game" }`.

Note on naming: the new external crate is referenced as `game::` (extern
prelude); the client's `world::game` module remains `crate::world::game` and is
unaffected.

### `server` changes

- `server/Cargo.toml` adds `game = { path = "../game" }` only. No server logic
  changes in 2a (the server does not `use` `game` yet; that begins in 2b). An
  unused dependency is not a clippy error.

### Workspace

Root `Cargo.toml` members become `["server", "client", "protocol", "game"]`.

## Network (if any)

No wire change. `GamePacket` (in `protocol`) is untouched: no variants added,
removed, reordered, or re-typed. `PlayerPos` still carries a client position
(client-authoritative) until 2b changes the flow. The only workspace-level
change is adding the `game` crate as a dependency of `client` and `server`.

## Acceptance criteria

Each item is a checkable statement of "done". Not an implementation plan.

### Workspace & crate

1. `game/Cargo.toml` exists with `name = "game"`, `edition = "2024"`, a single
   `[dependencies]` entry `glam = "0.27"`, and `[lints] workspace = true`.
2. The root `Cargo.toml` `[workspace] members` is exactly `["server", "client",
   "protocol", "game"]`.
3. `game/Cargo.toml` declares **no** `macroquad`, `postcard`, `laminar`, or
   `serde` dependency (a direct dependency check).
4. `grep -rn "macroquad" game/` returns no matches; `game/src` contains no
   `draw_*`, `is_key_down`, `is_key_pressed`, or `Color` usage.
5. `client/Cargo.toml` and `server/Cargo.toml` each declare
   `game = { path = "../game" }`.

### Simulation moved into `game`

6. `game/src/lib.rs` declares `pub mod config; pub mod types; pub mod maps; pub
   mod level; pub mod traits; pub mod rect; pub mod entities;` and compiles.
7. `game/src/config.rs` defines all seven existing constants (unchanged values)
   **and** `TICK_HZ: u32 = 20`.
8. `game/src/rect.rs` defines `Rect { x, y, w, h }` with `new(x, y, w, h)` and
   `overlaps` using strict `<` (edge-touching returns `false`).
9. `game/src/types.rs` defines `TileType`/`EntityType` and their `From<u8>`
   impls with unchanged mappings (including fallback-to-`Empty`).
10. `game/src/maps.rs` defines `Map` + `Map::new` with the same grids/data and
    dimensions as today.
11. `game/src/level.rs` defines `Level` with `new`, `pixel_width`,
    `pixel_height`, `map_colision`, `solid_point`, `colision`, `grounded`,
    `solid_in_rect`, and **no** `draw`.
12. `game/src/traits.rs` defines `Colisionable { get_rect -> Rect }` and
    `detect_collision`; it does **not** define an `Entity` trait with `draw`.
13. `game/src/entities/{player,enemy,nemesis,bullet}.rs` define the structs and
    physics methods from the Data model, with `Vec2 = glam::Vec2`, `Rect =
    game::Rect`, and no `Color`, no `draw`, no macroquad input/rand.

### Client/rendering split

14. `client/src/render.rs` (or equivalent) defines a local `Drawable` trait and
    implements `draw` for `game::Level`, `game::Player`, `game::Enemy`,
    `game::Nemesis`, `game::Bullet`, reproducing today's visuals (wall tiles
    GRAY, player RED, enemy BLUE for `EnemyKind::Enemy` / GREEN for
    `EnemyKind::Potion`, nemesis ORANGE + eye + `P: id` text, bullet YELLOW
    circle).
15. `client/src/{config.rs, types.rs, maps.rs, level.rs, traits.rs, entities/}`
    no longer exist, and `client/src/main.rs` no longer declares their `mod`
    lines.
16. `client/src/world/game.rs` imports the entities/level/map from `game::*` and
    renders via the `Drawable` trait; `World::update` builds a `PlayerInput`
    from macroquad keys and an enemy random draw and passes them into the
    entities' concrete `update` methods.
17. The `enemies_from_data` helper lives in `game` (not `client`), is used by
    `client/src/world/lib.rs`, and maps `EnemySpawn → EnemyKind::Enemy` and
    `PotionSpawn → EnemyKind::Potion`.

### Input-driven physics

18. `game::entities::player::Player::update` takes `(dt, &Level, &PlayerInput)`
    and contains no `is_key_down`/`is_key_pressed`; `PlayerInput` has held
    `move_left`/`move_right` and edge `jump`/`shoot`.
19. `game::entities::enemy::Enemy::update` takes an injected random value (e.g.
    `u32`) and contains no `macroquad::rand` call; the `random % 4` →
    `EnemyState` mapping is unchanged.

### Tests

20. The inline tests from `client/src/types.rs`, `client/src/maps.rs`,
    `client/src/level.rs`, and the `enemies_from_data` tests from
    `client/src/world/lib.rs` exist in `game` (mirroring their inline
    `#[cfg(test)]` placement), updated to `game::Rect`/`glam::Vec2`, and pass.
    The `level.rs` tests' `macroquad::prelude::Rect` becomes `game::Rect`.
21. `protocol/tests/round_trip.rs` is unchanged and still passes.

### Quality gates

22. `cargo build --workspace` from the root succeeds (all four crates).
23. `cargo test --workspace` passes (moved `game` tests green; no rendering or
    window/GPU code is exercised by any test — collision/parse/spawn tests
    construct `Rect`/`Vec2` values only and never call `draw` or macroquad).
24. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
25. `cargo fmt --check --all` reports no diffs.
