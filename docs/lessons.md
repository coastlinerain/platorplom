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

## Phase 2c (2026-11-24) — client prediction + reconciliation

- **`u16::wrapping_sub` for sequence-number comparison** (`game/src/lib.rs`) —
  network sequence numbers wrap around (u16 goes 65535 → 0). Simple `a > b`
  breaks at the boundary. `u16::wrapping_sub(a, b)` computes the difference
  in modular arithmetic, and comparing that difference against a threshold
  (32767 = u16::MAX/2) determines which value is "newer" without runtime
  overflow. This is the standard technique for TCP-like sequence comparison.
  - **Gotcha**: the threshold must be ≤ u16::MAX/2, or two values half the
    space apart become ambiguous. 32767 is safe; any value ≤ 32767 works.

- **`impl Iterator` with closure capture** (`client/src/network/prediction.rs`) —
  `InputRingBuffer::replay_order()` returns `impl Iterator<Item = &InputEntry>`
  instead of a concrete type. This hides the internal `filter_map` + `move`
  closure from the public API. Callers get a standard `.collect()`/`.for_each()`
  interface without knowing the ring buffer's internal layout.
  - **Gotcha**: the `move` keyword on the closure captures `self.head` and
    `self.len` by value (they're `Copy: usize`), so the closure outlives the
    borrow. Without `move`, the closure would borrow `self` and the return
    type would need a lifetime tied to `&self`.

- **`std::array::from_fn(|_| None)` for fixed-size array init**
  (`client/src/network/prediction.rs`) — Rust doesn't have a `Default` impl
  for `[Option<T>; 64]` (arrays only derive traits up to 32 elements).
  `from_fn` calls a closure once per index to populate the array, bypassing
  the 32-element trait limitation.
  - **Insight**: `[None; 64]` *does* work because `Option<T>` is `Copy`
    when `T: Copy`. But for non-`Copy` types like `Option<InputEntry>`
    (which contains `u16` + `bool` — `Copy` in this case, but the pattern
    generalizes), `from_fn` is the escape hatch.

- **`Instant::checked_duration_since` + `is_some_and` for safe elapsed-time math**
  (`client/src/network/interpolation.rs`) — `checked_duration_since` returns
  `Option<Duration>` (handles the case where `now < timestamp`, unlike
  `duration_since` which panics). Chained with `is_some_and(|d| d.as_millis() > 200)`,
  it expresses "is this timestamp more than 200ms old?" in a single expression
  without unwrap/expect. This replaces the older `.map_or(false, |d| ...)` pattern —
  `is_some_and` is more readable and was stabilized in Rust 1.70.
  - **Gotcha**: `checked_duration_since` is the safe cousin of `duration_since`.
    In game timers, `now < timestamp` can happen (clock adjustments, first frame),
    so always prefer the checked variant.

- **`f32::clamp(0.0, 1.0)` for lerp factor bounding**
  (`client/src/network/interpolation.rs`) — the lerp factor `t = elapsed / interval`
  can exceed 1.0 when the client is behind (elapsed > interval). `clamp(0.0, 1.0)`
  prevents extrapolation (the player would "run ahead" of the latest snapshot).
  Clamping to 1.0 means the interpolated position snaps to the newest snapshot
  when the interval has fully elapsed — exactly the desired behavior for
  game state interpolation.
  - **Insight**: `clamp` is a method on `f32` (and all float types), stabilized
    in Rust 1.50. It's `(value).clamp(min, max)` — not a free function.

- **`#[allow(clippy::too_many_arguments)]` for targeted lint suppression**
  (`client/src/network/prediction.rs`) — `reconcile` takes 8 parameters
  (player, buffer, last_acked_seq, server_x, server_y, server_dir,
  server_last_seq, level). The clippy default is 7. Rather than refactoring
  into a struct (which would add ceremony for a single call site), the function
  suppresses the lint with `#[allow(clippy::too_many_arguments)]`. This is a
  deliberate trade-off: the flat parameter list is easier to read at the call
  site in `handle_packet` than constructing a `ReconcileParams` struct.
  - **Insight**: `#[allow]` silences the lint unconditionally. `#[expect]`
    (newer) warns if the lint *doesn't* fire, acting as a self-cleaning
    annotation. Use `#[allow]` when you're sure the lint will persist.

- **`#[expect(dead_code)]` vs `#[allow(dead_code)]` — semantic difference**
  (`client/src/network/prediction.rs`) — `#[expect(dead_code)]` on
  `InputEntry::from_game_input` fires a warning if the function becomes
  *used* (the expectation is unfulfilled), prompting removal of the attribute.
  `#[allow(dead_code)]` on `is_empty`/`len` stays silent regardless. The
  difference matters: `from_game_input` is genuinely unused in production
  (only tests), while `is_empty`/`len` are used by tests in the same target
  but not in the binary target — the lint fires in one target but not the
  other, making `#[expect]` unreliable (it would warn in the binary build).
  - **Gotcha**: `#[expect]` is per-target. If a function is used in `#[test]`
    but not in production, `#[expect(dead_code)]` will warn in the test build.
    Use `#[allow]` when the usage depends on the compilation target.

- **Split-data interpolation pattern** (`client/src/network/interpolation.rs`
  + `game/src/entities/nemesis.rs`) — `Nemesis` (in `game`) holds the raw
  position snapshots; `RemoteInterp` (in `client`) holds the arrival timestamps.
  Keeping timestamps out of `game` prevents the simulation crate from depending
  on `std::time`, and the split means `game` stays headless/pure. The two
  structures synchronize through the `push` call which writes to both.
  - **Gotcha**: the synchronization is implicit — `RemoteInterp::push` relies
    on `Nemesis::push_interp_snapshot`'s shift-on-third-push behavior. If
    `push_interp_snapshot` changed its eviction order, the timestamp ring
    would silently desync from the position ring.

## Phase 2d (2026-11-24) — e2e testing infrastructure

### Trait objects (`Box<dyn Trait>`) for swappable transport (2026-11-24)

- **Where**: `client/src/network/packet_io.rs`, `client/src/world/game.rs`, `e2e/src/transport.rs`.
- **What it is**: `Box<dyn PacketIO>` stores a heap-allocated value whose concrete
  type is erased behind a trait. At runtime, method calls go through a vtable
  (dynamic dispatch). The real binary wraps a `RealPacketIO` (laminar); the
  e2e crate wraps a `LoopbackTransport` (mpsc channels). The `Box` is needed
  because `dyn Trait` is unsized — it can't live on the stack.
- **Gotcha / insight**: You don't need the trait in scope (`use PacketIO`) to
  call trait methods on a `Box<dyn Trait>` — Rust resolves them through the
  vtable automatically. This is different from generic dispatch where the
  trait bounds must be in scope.

### Cargo lib/bin split in a single crate (2026-11-24)

- **Where**: `client/Cargo.toml`, `client/src/lib.rs`, `client/src/main.rs`.
- **What it is**: By adding `[lib]` with a `path` to `src/lib.rs`, the same
  `Cargo.toml` produces TWO targets — a library (`metroidvania_rust.rlib`) and
  a binary (`metroidvania-rust`). `main.rs` uses `use metroidvania_rust::...`
  (the lib name is the Cargo lib target name, not the package name with hyphens).
  Bin-only modules (`hud`) stay as `mod hud;` in `main.rs`; shared modules
  (`network`, `world`, `camera`, `render`) go in `lib.rs`.
- **Gotcha / insight**: The lib target name replaces hyphens with underscores.
  So `metroidvania-rust` (package) becomes `metroidvania_rust` (lib). The
  `[lib] name` field controls this; without it, Cargo derives the lib name from
  the package name by replacing `-` with `_`.

### `std::sync::mpsc` channels for in-process messaging (2026-11-24)

- **Where**: `e2e/src/transport.rs`, `e2e/src/harness.rs`.
- **What it is**: `mpsc::channel()` creates a `(Sender<T>, Receiver<T>)` pair.
  The sender can be cloned and shared; the receiver is single-consumer.
  `rx.try_recv()` returns `Ok(value)` if data is available, `Err(Empty)` if not,
  and `Err(Disconnected)` if all senders have been dropped. This is the
  building block for the `LoopbackTransport` — a crossed pair of channels
  (`(tx_a → rx_b, tx_b → rx_a)`) that simulates a network with zero latency.
- **Gotcha / insight**: `mpsc::Sender` implements `Clone`, so you can have
  multiple producers feeding a single consumer. `Receiver` is NOT `Clone` —
  it's move-only, enforcing the single-consumer guarantee.

### In-memory integration test harness pattern (2026-11-24)

- **Where**: `e2e/src/harness.rs`.
- **What it is**: The `TestHarness` replaces UDP with mpsc channels: each
  connected `TestClient` has a `LoopbackTransport` (PacketIO over channels),
  and the harness simulates the full tick loop — client sends PlayerInput →
  server processes → server broadcasts PlayerPos/Action → client receives →
  client runs prediction. All in-process, deterministic, with no timing races.
  The `tick()` method is a 8-step simulation that mirrors the real server's
  `main()` loop exactly.
- **Gotcha / insight**: The borrow checker requires careful scope management
  when both client physics (needs `&Level`) and room transitions (needs
  `&mut World`) run in the same tick. Solution: scope-block the level borrow
  so it drops before `check_level_transitions(&mut client.world)`.

### `#[derive(Default)]` and clippy's `new_without_default` (2026-11-24)

- **Where**: Multiple structs (`GameCamera`, `RemoteInterp`, `InputRingBuffer`,
  `ServerWorld`, `VirtualClock`, `TestHarness`).
- **What it is**: Clippy's `new_without_default` lint fires when a type has a
  `pub fn new() -> Self` but no `impl Default`. The fix is either `#[derive(Default)]`
  (when all fields have Default impls) or a manual `impl Default for T { fn default()
  -> Self { Self::new() } }`. Adding `Default` makes the type usable in
  `HashMap::entry().or_default()`, `Option::unwrap_or_default()`, and other
  standard library APIs.
- **Gotcha / insight**: Derive `Default` only works when every field type implements
  `Default`. `VirtualClock` can't derive it because `Instant` doesn't implement
  `Default` (it depends on the system clock), so a manual impl calling `Self::new()`
  is the fix.