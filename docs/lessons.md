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

## Phase 2b (2026-11-24) — server-authoritative movement

- **`Instant`-based fixed-step game loop** (`server/src/main.rs`) — instead of
  blocking on `recv()` and processing one event at a time, the server drains
  `try_recv()` (non-blocking) and runs simulation ticks at a fixed interval via
  `Instant::now() >= next_tick`. Each tick advances `next_tick += tick_interval`,
  and a catch-up loop handles overruns. This decouples network I/O cadence from
  simulation cadence — the server ticks at exactly `TICK_HZ` regardless of how
  many packets arrive between ticks.
  - **Gotcha**: `Instant::duration_since` panics if `now < next_tick` (time
    monotonicity), so use `next_tick.checked_duration_since(Instant::now())`
    instead when computing the idle sleep.

- **`Duration::from_secs_f64`** (`server/src/main.rs`) — converts a fractional
  second (`1.0 / TICK_HZ as f64`) into a `Duration`. The `_f64` variant accepts
  sub-millisecond precision; `from_secs_f32` exists too. Use this to derive tick
  intervals from a const Hz value without hardcoding milliseconds.

- **`HashMap::entry().or_default()`** (`server/src/world.rs`) — a lazy-init
  pattern: `map.entry(key).or_default()` returns a `&mut V` to either the
  existing value or a freshly-inserted `Default::default()`. More concise than
  `if !map.contains_key(&k) { map.insert(k, Default::default()); } map.get_mut(&k)`.
  Used in `ServerWorld::apply_input` to initialize per-player accumulators on
  first sight.
  - **Gotcha**: `or_default()` requires `V: Default`. `PlayerInput` derives
    `Default` (all fields `false`), so it works — but if `V` doesn't implement
    `Default`, use `or_insert_with(|| ...)` instead.

- **`.get().and_then()` for safe nested `Vec` lookup with fallback**
  (`server/src/world.rs`) — `levels.get(y).and_then(|row| row.get(x))` returns
  `Option<&Level>`, safely handling out-of-bounds indices for BOTH dimensions in
  one chain. Combined with `.unwrap_or(&levels[0][0])`, it provides a graceful
  fallback for client-trusted coordinates that might be bogus (e.g. `(255,255)`).
  - **Gotcha**: the chained `Option` only short-circuits on `None` from `.get()`.
    If `row.get(x)` returns `Some`, the `and_then` closure fires. This pattern beats
    nested `if y < levels.len() && x < levels[y].len()` bounds checks.

- **Match-arm flow inversion** (`client/src/network/world.rs`) — the old
  `handle_packet` tested `if Some(id) != world.id` (remote player branch first,
  own player branch absent/silent). The new code tests `if Some(id) == world.id`
  (own player branch first: apply authoritative pos/dir from server), with the
  else branch handling remote `Nemesis` mirroring unchanged. A small but critical
  inversion: the own player now *receives* its state from the network instead of
  *sending* it.
  - **Insight**: this is the code-level manifestation of the architectural shift
    from "client broadcasts position" to "server broadcasts position, client
    applies it."

- **`Vec<T>` field relocation between structs** (`client/src/world/game.rs`) —
  moving `bullets: Vec<Bullet>` from `Player` into `World` is a mechanical
  refactor: every `self.player.bullets.push(...)`/`.retain(...)`/`.iter()` becomes
  `self.bullets.push(...)`/etc. The field's ownership moves up one level in the
  struct hierarchy because the code that populates and drains it (bullet
  spawn/update/collide) moved from `Player::update` into `World::update`.
  - **Gotcha**: any other code that referenced `player.bullets` must be updated
    too — in this case `load_current_level` (which cleared on death/room change)
    and `draw` (which rendered). A `grep` for the old path catches leftovers.

- **`|=` boolean OR-assignment for edge accumulation**
  (`server/src/world.rs`) — `pending.jump |= input.jump` accumulates edge flags
  across multiple input packets between ticks. If the client sends `jump: true`
  twice, the accumulator stays `true` (OR is idempotent). This is the complement
  of the held-flag pattern (`pending.move_left = input.move_left`, which
  overwrites). Together they implement "latest held wins, edges latch."
  - **Insight**: `|=` on a `bool` is just a logical OR — `a |= b` is `a = a || b`.
    Rust's `|=` is defined for `bool` via `std::ops::BitOrAssign`.

- **`#[derive(Default)]` on `PlayerInput`** — the `Default` derive generates
  `PlayerInput { move_left: false, move_right: false, jump: false, shoot: false }`,
  which is exactly the "no input" state. Used in test fixtures
  (`PlayerInput { move_right: true, ..Default::default() }`) and when inserting
  a fresh accumulator entry via `or_default()`.