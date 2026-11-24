# Rust lessons

Running journal of Rust concepts encountered while building Rustvania.
**Add an entry whenever a new concept appears or clicks.**

## Template

### <concept> (<date>)

- **Where**: file / feature.
- **What it is**: 2-3 sentences.
- **Gotcha / insight**:

## Concepts already in the codebase (to review)

- **crossbeam channels** — `Sender`/`Receiver` for cross-thread message passing
  (network thread → game loop). Used in both server and client.
- **laminar** — UDP socket with reliable/unreliable + ordered channels
  (`Packet::reliable_ordered` vs `Packet::unreliable`).
- **postcard** — `serde`-based binary serialization (`to_allocvec` / `from_bytes`).
- **Trait** (`Colisionable`) — shared collision shape across player/enemy/bullet.
  (The old `Entity` trait was removed in Phase 2a — its `update`/`draw` halves
  split into `game` physics and a client `Drawable`.)
- **`From<u8>` impls** — converting raw map bytes into typed enums
  (`TileType`, `EntityType`).
- **`HashMap`** keyed by `SocketAddr` (server) and player id (client).
- **`Option<u8>`** for a "not yet assigned" player id.
- **`std::thread::spawn`** for the network polling loop.
- **macroquad prelude** — `Vec2`, `Rect`, `draw_rectangle`, `is_key_down`, etc.

## Phase 1 (2026-11-23)

- **`PartialEq` vs `Eq`** — `GamePacket` derives `PartialEq` but not `Eq`, because
  `f32` implements `PartialEq` (NaN != NaN) but not `Eq`. `assert_eq!` needs
  `PartialEq`; `Eq` requires total equality.
- **`#[cfg(test)] mod tests` vs `tests/` integration tests** — inline `#[cfg(test)]`
  unit tests can reach private items; `tests/` integration tests use only the
  public library API. A binary crate (no `lib.rs`) can only use inline tests.
- **`[dev-dependencies]` + feature flags** — `postcard` (with the `alloc` feature)
  is a dev-dependency of `protocol`, so the normal build stays lean (only `serde`).
- **`pub(crate)` visibility** — lets a sibling module (`world::game`) call a
  function in `world::lib` without exposing it publicly.
- **borrow-before-move** — in `World::new`, compute `enemies_from_data(&levels[0][0].data)`
  into an owned `Vec<Enemy>` BEFORE moving `levels` into the struct literal, so the
  borrow ends before the move.

## Phase 2a (2026-11-23) — extract the `game` core crate

- **Orphan rule + local `Drawable` trait** — you can't implement a foreign trait
  (`macroquad`'s `draw`) for a foreign type (`game::Player`), but you *can* define
  a trait in your own crate (`client::render::Drawable`) and implement it for a
  foreign type. That's how rendering moved out of the `game` crate while the
  `game` types stayed rendering-free.
- **Input-driven physics (`PlayerInput`)** — instead of `Player::update` reading
  `is_key_down` directly, the client builds a `PlayerInput` struct (held
  `move_left`/`move_right`, edge-triggered `jump`/`shoot`) and passes it in. The
  simulation crate then has no idea where inputs come from — essential for 2b,
  where the *server* will feed it inputs from the network.
- **Injected randomness** — `Enemy::update(dt, level, random: u32)` takes the
  random draw as a parameter rather than calling `macroquad::rand` itself. Keeps
  the simulation deterministic and macroquad-free; the client supplies the draw.
- **Cross-crate type identity** — `macroquad::Vec2` *is* `glam::Vec2` (macroquad
  re-exports it). So a `game` struct field of type `glam::Vec2` interoperates with
  the client's `vec2(...)` calls with **no conversion** — no `into()`, no
  `Vec2::from`. Worth checking the re-export before writing shims.
- **Workspace crate extraction** — moving pure logic from a binary crate
  (`client`) into a library crate (`game`) makes it unit-testable headlessly and
  shareable with the server. The `Entity` trait's `update`/`draw` split across
  crates is the key structural move.
