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

### `Option::and_then()` chaining for safe indexing (2026-11-24)

- **Where**: `client/src/network/world.rs` (handle_packet), `client/src/world/lib.rs`
  (check_level_transitions), `e2e/src/harness.rs` (tick step 7).
- **What it is**: `Vec::get(idx)` returns `Option<&T>`, avoiding panics on
  out-of-range access. `Option::and_then(|x| ...)` chains lookups so you can
  do `levels_matrix.get(cy).and_then(|row| row.get(cx))` to get
  `Option<&Level>` from a 2D `Vec<Vec<Level>>` without nested indices that
  could panic.
- **Gotcha / insight**: Direct `vec[row][col]` indexing panics at runtime with
  "index out of bounds". The `.get().and_then()` pattern returns `None` for any
  missing dimension, and `let Some(x) = opt else { return; }` provides a clean
  early-return for the fail-fast case.

### `colision` vs `solid_in_rect` — OOB asymmetry (2026-11-24)

- **Where**: `game/src/level.rs` (`colision` / `solid_in_rect`), used in
  reconciliation guard at `client/src/network/world.rs`.
- **What it is**: `colision` samples 6 points of a rect against `solid_point`,
  which returns `true` for any coordinate outside the grid — treating OOB as
  solid. `solid_in_rect` checks tile mappings with bounds guards and treats OOB
  as non-solid. This asymmetry means `colision` at a room boundary (where part
  of the player rect extends beyond the level) falsely reports a wall hit,
  blocking reconciliation. The fix in the `not_in_wall` guard uses
  `solid_in_rect` instead of `colision` so edge positions are reconciliable.
- **Gotcha / insight**: When two functions on the same type have opposite OOB
  semantics, always check which one the calling code expects. The original
  guard used `colision` and silently skipped reconciliation for all room-edge
  positions, making room-transition tests impossible until this was fixed.

### Cumulative vs per-tick collection in test harness (2026-11-24)

- **Where**: `e2e/src/harness.rs` (`last_shots`, `last_actions`).
- **What it is**: Originally `last_shots` was reassigned each tick (`self.last_shots
  = self.server.update(...)`), so assertions after a `tick_n(5)` only saw the
  last tick's results. The fix: store the per-tick return value in a local,
  then `.push()` into `last_shots` and `last_actions` so they accumulate across
  ticks.
- **Gotcha / insight**: Integration tests that call `tick_n()` and then inspect
  harness fields need those fields to be cumulative unless explicitly cleared.
  Per-tick snapshots (re-assign) are only useful for single-tick assertions.

### Assertion tolerance for deterministic-but-tick-aligned values (2026-11-24)

- **Where**: `e2e/tests/movement.rs` (A7), `e2e/tests/reconciliation.rs` (E6).
- **What it is**: When the test harness uses `reconcile → predict` order but
  production uses `predict → reconcile`, the client's predicted position leads
  the server by exactly one tick of movement (~6.67 px at VELOCIDAD/60).
  Assertions must account for this constant offset rather than expecting exact
  equality.
- **Gotcha / insight**: Frame-alignment differences between test and production
  are a systemic source of "flaky" assertions. The fix is to compute the
  expected offset analytically (1 frame × VELOCIDAD × dt) and set the
  tolerance just above it, rather than using a magic number.

## Phase 2f (2026-11-25) — Netcode Synchronization & Frame Alignment

### NTP-style clock synchronization with wall-clock time (2026-11-25)

- **Where**: `server/src/world.rs` (`wall_time_ms`), `client/src/network/world.rs` (`handle_packet`).
- **What it is**: Server broadcasts `server_tick` (monotonic tick counter) and `server_time_ms` (wall-clock `SystemTime::now()`) in every `PlayerPos`. Client computes `clock_offset_ms = server_time_ms - local_time_ms` and `server_tick_offset = server_tick - client_tick`. This enables the client to estimate the current server tick (`client_tick + server_tick_offset`) for interpolation targeting.
- **Gotcha / insight**: Must use `SystemTime` (wall-clock) on BOTH sides, not `Instant` (monotonic). `Instant` subtraction is meaningless across processes/machines. `SystemTime` allows meaningful `server_time_ms - local_time_ms` arithmetic. Server must run NTP for accuracy; no smoothing/filtering in this phase.

### Tick-based interpolation domain (avoid mixing time domains) (2026-11-25)

- **Where**: `client/src/network/interpolation.rs` (`interpolate_at_tick`), `client/src/world/game.rs` (`draw`).
- **What it is**: Instead of interpolating based on wall-clock `now` (which mixes `Instant` and `SystemTime` domains), the client computes a `target_tick = server_tick_estimate - INTERPOLATION_DELAY_TICKS` and converts it to an expected arrival `Instant` using the most recent packet's `(server_tick, arrival_time)` as anchor. Finds two snapshots bracketing that arrival time and lerps.
- **Gotcha / insight**: Never mix `SystemTime` (wall-clock) and `Instant` (monotonic) in arithmetic. They have different epochs and semantics. Tick-based interpolation keeps everything in the monotonic `Instant` domain for interpolation, using server ticks only as a conversion factor (`target_tick - last_known_server_tick`) × tick_duration.

### Ring buffer eviction for N=5 jitter buffer (2026-11-25)

- **Where**: `game/src/entities/nemesis.rs` (`push_interp_snapshot`), `client/src/network/interpolation.rs` (`RemoteInterp::push`).
- **What it is**: Both structures use a 5-entry ring buffer. On the 6th push, shift entries `[1]→[0]`, `[2]→[1]`, `[3]→[2]`, `[4]→[3]`, new at `[4]`. `interp_len` tracks valid count (0..5). `RemoteInterp::timestamps` mirrors the exact same eviction to stay in sync with `Nemesis::interp_snapshots`.
- **Gotcha / insight**: The two structures' eviction logic MUST stay perfectly in sync. If `push_interp_snapshot` changes its shift order, `RemoteInterp::push` must be updated identically. This implicit coupling is a maintenance risk; consider a shared buffer struct in future.

### `INTERPOLATION_DELAY_TICKS` vs `INTERPOLATION_DELAY_MS` (2026-11-25)

- **Where**: `game/src/config.rs`, `client/src/network/interpolation.rs`, `client/src/world/game.rs`.
- **What it is**: Replaced `INTERPOLATION_DELAY_MS` (milliseconds) with `INTERPOLATION_DELAY_TICKS` (server ticks). At 60Hz, 5 ticks = 83ms. The delay is now expressed in the server's tick domain, matching the interpolation target calculation (`target_tick = server_tick_estimate - INTERPOLATION_DELAY_TICKS`).
- **Gotcha / insight**: Constants should live in the domain where they're used. Milliseconds belong to wall-clock; ticks belong to the simulation timeline. Using ticks avoids conversion errors and makes the headroom calculation explicit (N=5 snapshots = 4 intervals = 67ms span, 5 tick delay = 83ms, ~17ms headroom).

### `STALENESS_THRESHOLD_TICKS` for force-reconcile heuristic (2026-11-25)

- **Where**: `game/src/config.rs`, `client/src/network/world.rs` (`update_network`).
- **What it is**: If `estimated_server_tick - last_acked_server_tick > STALENESS_THRESHOLD_TICKS` (5), the client detects that server updates have stalled and could trigger force-reconciliation (input re-send, position query). Currently logs the condition; full force-reconcile needs a server position to reconcile against.
- **Gotcha / insight**: Staleness detection supplements (not replaces) `last_seq` based reconciliation. `last_seq` is input-sequence precise; `server_tick` adds wall-clock context. The threshold must account for normal jitter (5 ticks = 83ms at 60Hz).

### Packet logging with JSONL and `RUSTVANIA_PACKET_LOG` (2026-11-25)

- **Where**: `client/src/network/logging.rs`, `server/src/main.rs`, `client/src/main.rs`.
- **What it is**: `log_packet(dir, packet)` emits one JSON object per line to stdout when `RUSTVANIA_PACKET_LOG=1` is set. Includes `ts_ms`, `dir` (sent/recv), `type`, `id`, `seq`, `server_tick`, `server_time_ms`, and `key_fields` (abbreviated summary per variant). Zero cost when disabled (early return on env var check).
- **Gotcha / insight**: JSONL streams are parseable with `jq`, work in CI logs, and don't require external dependencies. The env var gating is essential — PlayerPos at 60Hz per player generates massive output. Server logging is inlined in `main.rs` to avoid a new crate dependency; client uses the shared `logging` module.

### Single binary with `--headless` flag (2026-11-25)

- **Where**: `client/src/main.rs` (`run_headless`, `HeadlessPacketIO`).
- **What it is**: The same `main.rs` entry point checks `std::env::args().any(|a| a == "--headless")`. If true, runs `run_headless()` with a fixed tick loop (`game_world.update(tick_dt)` + `std::thread::sleep`), no macroquad window init, no `draw()` or `next_frame().await`. Supports `--duration=N` (seconds) and `--server=ADDR` flags.
- **Gotcha / insight**: `#[macroquad::main]` still creates the async runtime, but we return early before any window calls. Single binary avoids code duplication between GUI and headless modes. For pure in-memory CI testing, the e2e harness uses `LoopbackTransport` (not this headless mode).

### Clock sync reset triggers (Join, Hit, Room Transition, F11) (2026-11-25)

- **Where**: `client/src/network/world.rs` (JoinResponse, Hit), `client/src/world/lib.rs` (`load_current_level`), `client/src/world/game.rs` (`update` F11).
- **What it is**: Clock sync fields (`client_tick`, `server_tick_offset`, `clock_offset_ms`, `last_server_tick`, `last_server_time_ms`, `last_acked_server_tick`) are zeroed on: (1) `JoinResponse` (new session), (2) `Hit` for own player (death/respawn teleport), (3) `load_current_level` (room transition teleport), (4) `KeyCode::F11` (manual debug reset).
- **Gotcha / insight**: These are exactly the discontinuities where the server's time reference or player position teleports. Forgetting any one causes stale offsets that corrupt interpolation. `Hit` handling is now in both `handle_packet` (server-broadcast death) AND `handle_death` (client-initiated death) — keep both in sync.

### Phase 2f: Netcode Synchronization & Frame Alignment (2026-11-25)

#### SystemTime vs Instant — never mix wall-clock and monotonic
- **Where**: `server/src/world.rs` (`wall_time_ms`), `client/src/network/world.rs` (`handle_packet` clock sync), `client/src/network/interpolation.rs` (`interpolate_at_tick`).
- **What it is**: `SystemTime` is wall-clock (can jump via NTP/leap seconds), `Instant` is monotonic (never goes backward, only measures duration since arbitrary epoch). Clock sync uses `SystemTime` on both sides (NTP-style offset). Interpolation uses `Instant` for arrival timestamps and converts target ticks to expected arrival using `last_known_arrival` as anchor.
- **Gotcha / insight**: The original prototype sent server `Instant`-based uptime as `server_time_ms` but client used `SystemTime::now()` — subtracting them is meaningless (different epochs). Fix: server must send `SystemTime` wall-clock. Also, `interpolate_at_tick` converts server ticks to monotonic `Instant` using the anchor (`last_known_arrival`), never mixing domains.

#### Ring buffer eviction — keep latest N, shift on (N+1)th push
- **Where**: `game/src/entities/nemesis.rs` (`push_interp_snapshot`), `client/src/network/interpolation.rs` (`RemoteInterp::push`).
- **What it is**: 5-entry ring buffer. On 6th push: shift `[1]→[0]`, `[2]→[1]`, `[3]→[2]`, `[4]→[3]`, new at `[4]`. `RemoteInterp.timestamps` mirrors this exactly. Both use `interp_len` to track valid count (0..5).
- **Gotcha / insight**: Eviction order must be identical in both structures. If `Nemesis` shifts 4 entries but `RemoteInterp` shifts 3, timestamps desync and interpolation produces garbage. Test `interpolate_evicts_old_snapshots` verifies this.

#### Tick-based interpolation — convert server ticks to Instant using anchor
- **Where**: `client/src/network/interpolation.rs` (`interpolate_at_tick`).
- **What it is**: Target render tick = `server_tick_estimate - INTERPOLATION_DELAY_TICKS`. Convert to expected arrival: `target_arrival = last_known_arrival + (target_tick - last_known_server_tick) * TICK_MS`. Find two snapshots whose arrival times bracket `target_arrival`, lerp position, snap direction to newer.
- **Gotcha / insight**: This avoids `SystemTime` entirely for interpolation. The server tick is the authoritative timeline; client renders at a fixed delay behind it. Stale guard uses `target_arrival` vs newest timestamp (not wall-clock `now`).

#### Feature-gated binaries — headless without macroquad window
- **Where**: `client/Cargo.toml` (`[[bin]]` with `required-features`), `client/src/main.rs` (GUI), `client/src/main_headless.rs` (headless).
- **What it is**: Two binaries: `metroidvania-rust` (requires `gui` feature, uses `#[macroquad::main]`), `rustvania-headless` (requires `headless` feature, plain `fn main()`). CI runs `cargo run --bin rustvania-headless --features headless`.
- **Gotcha / insight**: `#[macroquad::main]` creates the window BEFORE calling your async function — you cannot check a flag and skip window creation. Separate binaries with feature gates are the clean solution.

#### Staleness threshold constant — STALENESS_THRESHOLD_TICKS in game::config
- **Where**: `game/src/config.rs` (`STALENESS_THRESHOLD_TICKS = 5`), `client/src/network/world.rs` (staleness check).
- **What it is**: If `server_tick_estimate - last_acked_server_tick > STALENESS_THRESHOLD_TICKS`, network may be stalled. Force reconciliation or log warning.
- **Gotcha / insight**: `server_tick_estimate` is `client_tick + server_tick_offset` (both can be negative early on). Use `saturating_sub` and check `> 0` before comparing to threshold.

## Phase 2g (2026-11-25) — Fixed Timestep & Accumulator Pattern

### Accumulator pattern (fixed timestep) — `get_time()` vs `get_frame_time()` (2026-11-25)

- **Where**: `client/src/main.rs` (GUI loop).
- **What it is**: Each render frame, accumulate the real elapsed `frame_dt = get_time() - last_time`, clamp it to `MAX_ACCUMULATED_FRAMES * tick_dt` (spiral-of-death protection), then run 0..=5 fixed `update(TICK_DT)` steps and render with `alpha = accumulator / tick_dt`. Simulation runs at a fixed 60Hz regardless of render rate; only the *render* is interpolated.
- **Gotcha / insight**: `get_time()` returns `f64` (seconds since start), while the old `get_frame_time()` returns `f32` (the last frame's delta). Mixing them in the same `accumulator += frame_dt` expression is a type error — keep the accumulator in `f64` and cast to `f32` only at the `update(tick_dt as f32)` / `draw(alpha as f32)` boundary. The clamp is a *safety* bound (prevents runaway catch-up), not a render-floor guarantee.

### `std::hint::spin_loop()` — hybrid sleep + busy-wait (2026-11-25)

- **Where**: `client/src/main.rs` (`run_headless`).
- **What it is**: To hit true 60Hz without drift, the loop advances `next_tick += tick_interval` (a monotonic `Instant`), sleeps for `remaining - HEADLESS_BUSY_WAIT_MS` (most of the interval), then busy-waits the last ~1ms with `while Instant::now() < next_tick { std::hint::spin_loop() }`. `spin_loop` is a hint to the CPU (like `PAUSE`) that keeps the thread alive without a full context switch, giving sub-millisecond precision.
- **Gotcha / insight**: `std::thread::sleep` has ~1ms precision on Linux and ~15ms on Windows, so a plain `sleep(16)` drifts (~62.5Hz). Sleeping for the bulk and busy-waiting only the final ~1ms costs ~6% CPU at 60Hz — the owner-approved tradeoff. Advance `next_tick += interval` (not `next_tick = now + interval`) to avoid accumulating drift.

### `glam::Vec2::lerp` for render interpolation (2026-11-25)

- **Where**: `client/src/world/game.rs` (`interpolated_player_pos`).
- **What it is**: `self.prev_player_pos.lerp(self.player.pos, a)` returns `prev + (curr - prev) * a`, the standard linear interpolation. Combined with `alpha.clamp(0.0, 1.0)` it interpolates the local player between the previous and current sim-tick positions for smooth motion.
- **Gotcha / insight**: `Vec2` is `Copy`, so `player.pos` can be copied into a local before `player` is moved into the struct literal. For the *direction* (`interpolated_player_dir`), simple linear lerp misbehaves across the -π/π boundary; it's accepted here because per-tick angle changes are small (documented in the plan's Box 4 Watch out).

### Decoupling input sampling from simulation (dependency injection for testability) (2026-11-25)

- **Where**: `client/src/world/game.rs` (`World::update`), `client/src/main.rs` (GUI loop).
- **What it is**: `World::update` stopped calling `is_key_pressed`/`is_key_down` and instead reads injected `move_left`/`move_right`/`pressed_jump`/`pressed_shoot` fields. The GUI loop samples the keyboard once per render frame and writes those fields before calling `update`. The simulation crate therefore has no macroquad *input* dependency and can run headlessly under `#[cfg(test)]` and in the e2e harness.
- **Gotcha / insight**: macroquad *input* functions (`is_key_pressed`, `is_key_down`) panic when no window exists, but `macroquad::rand::gen_range` is context-free (it's a thread-local PRNG, `quad_rand`). So only the input calls had to move out — the enemy RNG call could stay. This is the same idea as Phase 2a's `PlayerInput`, taken one step further: even *sampling* the input is now outside the simulation.

### Decoupling the camera from screen dimensions (a second macroquad dependency) (2026-11-25)

- **Where**: `client/src/camera.rs` (`GameCamera`), `client/src/world/game.rs` (`World::update`), `client/src/main.rs`.
- **What it is**: `GameCamera::update` originally called `screen_width()`/`screen_height()`, which — like the input functions — panic outside a macroquad window (`get_context` → `assert!(THREAD_ID.is_some())`). This was a *second* macroquad-context dependency that Box 2a's input-decoupling description did not call out, and it broke every unit/e2e test that ran `World::update` headlessly. The fix: `GameCamera` stores injected `view_width`/`view_height` fields (default 1280×720), updated each frame in the GUI loop via `set_viewport(screen_width(), screen_height())`.
- **Gotcha / insight**: `screen_width`/`screen_height` are *context* queries, not *input* — grepping for `is_key_pressed`/`is_key_down` missed them. When decoupling a simulation from a windowing library, audit for **all** context-requiring calls (`screen_width`, `get_time`, `get_frame_time`, `get_mouse_position`, …), not just input.

### Edition 2024 gotchas surfaced by the RED tests (2026-11-25)

- **Where**: `e2e/tests/packet_log_rate.rs`.
- **What it is**: Two edition-2024 changes tripped up tests that were written but never executed (the tester's shell was denied): (1) `std::env::set_var`/`remove_var` are now `unsafe` (concurrent reads of the same var while another thread calls `set_var` are UB), so it must be wrapped in an `unsafe` block with a SAFETY comment; (2) `clippy::collapsible_if` fires on a nested `if { if let Some(..) { .. } }`, which collapses into a *let-chain*: `if cond && let Some(x) = expr { .. }`.
- **Gotcha / insight**: let-chains (`&& let`) are stable in edition 2024. A RED test that was never compiled/run can hide both kinds of issue — the subagent shell-denial meant the orchestrator had to be the first to actually execute them.