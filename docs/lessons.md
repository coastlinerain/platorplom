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
- **Traits** (`Entity`, `Colisionable`) — shared behavior across player/enemy/bullet.
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
