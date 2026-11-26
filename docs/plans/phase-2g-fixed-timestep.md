# Phase 2g — Fixed Timestep & Accumulator Pattern — Implementation Plan

- Status: implemented
- Spec: `docs/specs/phase-2g-fixed-timestep.md`
- Date: 2026-11-25
- Implemented: 2026-11-25

## Overview

Phase 2f added clock synchronization, jitter buffer, frame interpolation, packet logging, and a headless client. However, the client simulation loop is not yet deterministic at 60Hz. This phase implements:
1. **Accumulator pattern on Client GUI** — Separate simulation dt (fixed 1/60s) from render dt (variable).
2. **Sub-millisecond precision sleep on Headless** — Hybrid sleep + busy-wait for true 60Hz.
3. **Verify server catch-up loop** — Confirm existing loop is correct, add documentation comment.
4. **Fixed-rate input send** — Natural consequence of accumulator (one `PlayerInput` per sim tick).
5. **Render interpolation alpha** — Local player lerp between previous and current tick positions.

## Public API (targets for the tester)

```rust
// game/src/config.rs
pub const MAX_ACCUMULATED_FRAMES: u32 = 5;
pub const HEADLESS_BUSY_WAIT_MS: u64 = 1;

// client/src/world/game.rs
pub struct World {
    // ... existing fields ...
    pub prev_player_pos: glam::Vec2,
    pub prev_player_dir: f32,
}

impl World {
    // CHANGED: draw now takes interpolation alpha
    pub fn draw(&self, alpha: f32);

    // NEW: pure helper for testable interpolation (Box 4)
    pub fn interpolated_player_pos(&self, alpha: f32) -> glam::Vec2;
    pub fn interpolated_player_dir(&self, alpha: f32) -> f32;

    // update(dt) signature unchanged, but now ALWAYS called with TICK_DT
    pub fn update(&mut self, dt: f32);
}

// client/src/main.rs
// No public API changes — internal loop restructuring only
// NOTE: run_headless will become a non-async fn (Box 6)

 // server/src/main.rs
// No public API changes — verification comment only
```

## Test cases (tester writes these first)

### Unit Tests
1. **`World::update` fixed dt (AC3)** — In accumulator loop, `World::update` receives exactly `TICK_DT` (1/60s) every call.
2. **`World::interpolated_player_pos/dir(alpha)` (AC4)** — At alpha=0: returns `prev_player_pos/dir`; alpha=0.5: returns midpoint; alpha=1: returns `player.pos/dir`.
3. **Headless sleep timing (AC5)** — `run_headless` loop: 1000 ticks complete in 16.666s ±0.1s (hybrid sleep + busy-wait).
4. **Accumulator clamp (AC9)** — Large `frame_dt` (e.g., 1.0s) results in exactly `MAX_ACCUMULATED_FRAMES` (5) steps, accumulator clamped to `5 * TICK_DT`.
5. **`client_tick` increments per sim tick (AC10)** — Not per render frame; verify in accumulator loop.
6. **Direction lerp boundary** — `interpolated_player_dir` handles -π/π crossing (simple lerp accepted for small per-tick changes; document limitation). *Owned by Box 4 (Watch out) + Box 10.*

### Integration Tests
7. **GUI at 30 FPS simulation (AC6)** — Simulate 30 FPS render: runs 2 sim steps/frame, `client_tick` advances 2/frame, render smooth.
8. **GUI at 10 FPS simulation (AC9)** — Simulate 10 FPS render: runs 5 sim steps/frame (clamped), `client_tick` advances 5/frame; sim rate drops below 60Hz but the clamp prevents runaway.
9. **Headless 10s run (AC5)** — Completes ~600 ticks, average tick time 16.666ms ±0.1ms.
10. **Packet log rate (AC8)** — `RUSTVANIA_PACKET_LOG=1`: `PlayerInput` sent at 60Hz intervals (verify timestamps in log).
11. **Server catch-up under load (AC7)** — Event drain takes 10ms → catch-up runs 1 tick, sleeps remainder, no busy-spin.
12. **Spiral of death protection (AC9)** — At pathological frame times, sim catch-up is bounded to 5 steps/frame; the clamp prevents runaway.

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the `Explain` fields.

### Box 1 — Add config constants (`game/src/config.rs`)

- [x] done
- **Acceptance**: `cargo build` succeeds; `MAX_ACCUMULATED_FRAMES = 5`, `HEADLESS_BUSY_WAIT_MS = 1` exported.
- **Test**: Unit test compiles (AC1 in spec).
- **Explain**
  - What: Added `pub const MAX_ACCUMULATED_FRAMES: u32 = 5` and `pub const HEADLESS_BUSY_WAIT_MS: u64 = 1` to `game/src/config.rs`.
  - Why: The accumulator loop needs a clamp bound (spiral-of-death protection) and the headless loop needs a busy-wait threshold for sub-millisecond sleep precision.
  - Rust concepts: `pub const` items (compile-time constants); explicit integer literal suffixes (`u32`/`u64`) to pin the types used by `Duration::from_millis` and the loop bound.
  - Watch out: `HEADLESS_BUSY_WAIT_MS` feeds `Duration::from_millis`, so it must be `u64`; keeping it at 1ms limits busy-wait CPU to ~6% at 60Hz (owner-approved tradeoff).

### Box 2 — Add `prev_player_pos` and `prev_player_dir` to `World` (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: Fields added to `World` struct; initialized in `World::new()` to `player.pos` and `player.dir` (note: `Player::new(0)` already sets `pos` to the spawn position `(100.0, 100.0)` and `dir = 1.0` — initialize `prev_player_*` to those same values so the first frame has no visual jump).
- **Test**: Unit test compiles; `World::new()` initializes correctly.
- **Explain**
  - What: Added `pub prev_player_pos: glam::Vec2` and `pub prev_player_dir: f32` to `World`, and initialized them in `World::new()` to `player.pos`/`player.dir`.
  - Why: `draw(alpha)` needs the previous tick's state to lerp between; seeding it with the spawn state avoids a first-frame visual jump (alpha starts near 0).
  - Rust concepts: struct field addition; `glam::Vec2` and `f32` are `Copy`, so `player.pos`/`player.dir` can be copied into locals before `player` is moved into the struct literal.
  - Watch out: the copy must happen *before* `player` is moved into the `Self { .. }` literal; struct-literal field order matters for move semantics.

### Box 2a — Decouple `World::update` from macroquad (input + camera viewport) (`client/src/world/game.rs`, `client/src/camera.rs`, `client/src/main.rs`)

- [x] done
- **Acceptance**:
  - `World::update` no longer calls any macroquad input function (`is_key_pressed`, `is_key_down`). It reads the injected programmatic fields `move_left`, `move_right`, `pressed_jump`, `pressed_shoot` (already present on `World`) and builds the local `PlayerInput` from them.
  - The F11 clock-reset debug block is removed from `update` (it also calls `is_key_pressed`); moved to the GUI loop or dropped.
  - In `client/src/main.rs`'s GUI loop, immediately before `World::update`, sample `is_key_down(KeyCode::Left/Right)` and `is_key_pressed(KeyCode::Space/Z)` once per render frame and write them into the corresponding `World` fields (plus F11 if kept).
  - `GameCamera::update` no longer calls `screen_width()`/`screen_height()`; it uses injected `view_width`/`view_height` fields set via `GameCamera::set_viewport`, which the GUI loop calls once per frame before the accumulator loop.
  - `cargo build` succeeds; `World::update` runs under `#[cfg(test)]` with no macroquad window (no panic).
- **Test**: Existing RED unit tests Boxes 3, 8, 9, 11 (`update_stores_previous_player_state`, `world_update_receives_fixed_tick_dt`, `accumulator_clamp_large_frame_dt`, `client_tick_increments_per_sim_tick`) and the e2e `gui_accumulator` test (Box 13) stop panicking and produce meaningful assertions.
- **Explain**
  - What: `World::update` now reads the injected `move_left`/`move_right`/`pressed_jump`/`pressed_shoot` fields instead of calling `is_key_pressed`/`is_key_down`; the F11 clock-reset block moved to the GUI loop. Additionally, `GameCamera::update` no longer calls `screen_width()`/`screen_height()` — it reads new `view_width`/`view_height` fields (defaults 1280×720) injected via `set_viewport`, which the GUI loop calls once per frame before the accumulator loop.
  - Why: macroquad input *and* `screen_width`/`screen_height` both panic when no window exists (`screen_width` → `get_context` → `thread_assert`), which would break every unit/e2e test that calls `update` (the camera update path). Decoupling sampling and viewport (I/O) from simulation makes the sim deterministic and headless-testable.
  - Rust concepts: dependency injection (state via struct fields rather than global input/context calls); separation of concerns (sampling vs simulation); `#[cfg(test)]` testability; `Default` flows the new fields through `GameCamera::new()`.
  - Watch out: `pressed_jump`/`pressed_shoot` remain "edge" signals (consumed by `advance_sticky` in `update_network`); the local `player.update` keeps `shoot: false` because bullet spawn is server-authoritative (Phase 2e). Headless mode performs no sampling and never calls `set_viewport` — its `view_width`/`view_height` stay at the 1280×720 defaults, which is fine because headless never renders.

### Box 3 — Store previous position before player update in `World::update` (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: In `World::update`, before `self.player.update(...)`, store `self.prev_player_pos = self.player.pos` and `self.prev_player_dir = self.player.dir`.
- **Test**: Unit test — after one `update` call, `prev_player_pos` equals old position.
- **Explain**
  - What: `update` stores `self.prev_player_pos = self.player.pos` and `self.prev_player_dir = self.player.dir` immediately before `self.player.update(...)`.
  - Why: `interpolated_player_pos/dir` lerp prev → current, so the previous state must be captured before the player mutates.
  - Rust concepts: disjoint field borrows — writing `self.prev_player_pos` while reading `self.player.pos` and holding a `&self.levels_matrix` borrow is allowed because the fields are independent.
  - Watch out: must be stored *before* `player.update`, not after; otherwise prev == current and interpolation is a no-op.

### Box 4 — Add interpolation helpers and modify `World::draw` to accept `alpha` (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: 
  - Added pure methods `interpolated_player_pos(alpha: f32) -> Vec2` and `interpolated_player_dir(alpha: f32) -> f32` that return `lerp(prev, current, alpha)` with alpha clamped to [0,1].
  - `draw(&self, alpha: f32)` calls these helpers for local player render position/direction.
  - **Atomic with Box 5**: `draw()` signature change and GUI call site update MUST be in the same commit or code won't compile between boxes.
- **Test**: Unit test (AC4) — `interpolated_player_pos(0.0)` = prev, `(0.5)` = midpoint, `(1.0)` = current; same for dir.
- **Explain**
  - What: Added `interpolated_player_pos(alpha)` (returns `self.prev_player_pos.lerp(self.player.pos, a)`) and `interpolated_player_dir(alpha)` (linear lerp of dir); changed `draw(&self)` to `draw(&self, alpha: f32)` and rendered the local player at the interpolated position.
  - Why: smooth local-player motion between fixed sim ticks at any render rate (G5/AC4).
  - Rust concepts: `glam::Vec2::lerp`; `f32::clamp` to bound alpha to [0,1]; `pub` method signatures (public API, no `dead_code`).
  - Watch out: direction uses simple linear lerp, which can misbehave when `prev_player_dir` and `player.dir` cross the -π/π boundary — accepted for small per-tick changes (spec D5). `draw` signature change is atomic with Box 5. The player rectangle is direction-agnostic, so only the position affects rendering today; `interpolated_player_dir` is kept for API parity and a future facing indicator.

### Box 5 — Implement accumulator loop in GUI main (`client/src/main.rs`)

- [x] done
- **Acceptance**: GUI loop uses accumulator pattern: accumulates `frame_dt`, runs 0-5 `update(TICK_DT)` steps per frame, computes `alpha = accumulator / TICK_DT`, calls `draw(alpha)`. `client_tick` increments once per sim tick (already happens in `World::update`). **Atomic with Box 4** — same commit.
- **Test**: Integration test (AC6, AC9) — simulate 30 FPS (2 steps/frame), 10 FPS (5 steps/frame clamped), 144 FPS (0 or 1 step/frame).
- **Explain**
  - What: GUI loop now uses a fixed-timestep accumulator: sample keys once per render frame, accumulate `frame_dt` (from `get_time()`), clamp to `MAX_ACCUMULATED_FRAMES * tick_dt`, run 0..=5 `update(tick_dt)` steps, compute `alpha = accumulator / tick_dt`, and call `draw(alpha)`.
  - Why: deterministic 60Hz simulation independent of render rate (G1); the clamp bounds catch-up (spiral-of-death protection, AC9).
  - Rust concepts: `get_time()` (returns `f64`) timing; a bounded `while` loop; `f64`↔`f32` casts for `update`/`draw`.
  - Watch out: input is sampled once per *render* frame but consumed across 0..=5 sim steps; at low FPS a single physical key press spans multiple ticks (mitigated by grounded-gated jump and server shoot cooldown) — do not assume per-tick edge fidelity.

### Box 6 — Implement hybrid sleep + busy-wait in headless loop (`client/src/main.rs`)

- [x] done
- **Acceptance**: `run_headless` changed to **non-async fn** (removes `async`/`await` since it uses blocking `std::thread::sleep` + `spin_loop`). Uses `next_tick += tick_interval` with sleep for `remaining - HEADLESS_BUSY_WAIT_MS` then `spin_loop` for last ~1ms. No `sleep(16)`.
- **Test**: Integration test (AC5) — 1000 ticks in 16.666s ±0.1s; verify no drift over 10s.
- **Explain**
  - What: `run_headless` is now a non-async `fn`; it computes `tick_interval = Duration::from_secs_f64(1.0 / TICK_HZ)`, advances `next_tick += tick_interval` each loop, sleeps for `remaining - 1ms`, then busy-waits the last ~1ms with `std::hint::spin_loop`.
  - Why: true 60Hz with no drift (G2); the previous `sleep(16)` ran at ~62.5Hz and drifted ~4%.
  - Rust concepts: `std::time::{Duration, Instant}` (monotonic clock); `Duration::from_secs_f64`; `std::hint::spin_loop`; blocking vs `async` (dropped the `async`/`await` since the body is purely blocking).
  - Watch out: preserved the `"Headless client finished after {} ticks"` stdout line (`e2e/tests/headless_timing.rs` parses it for the exact tick count); the busy-wait is only the last ~1ms (owner-approved CPU tradeoff).

### Box 7 — Verify server catch-up loop and add comment (`server/src/main.rs`)

- [x] done
- **Acceptance**: Comment added at server tick loop (line ~295) documenting verification per **spec decision D3**: `tick_interval` exact (`Duration::from_secs_f64(1.0/60.0)`), `while now >= next_tick` catch-up, sleep `min(5ms)`, `next_tick += tick_interval` no drift, no busy-spin.
- **Test**: Code review (AC7); unit test for server timing if feasible.
- **Explain**
  - What: Added a verification comment above the server catch-up loop documenting each D3 invariant (exact interval, catch-up, 5ms sleep, no drift, no busy-spin).
  - Why: the loop is already correct; the comment records the review decision for traceability (AC7) without touching behavior.
  - Rust concepts: none (documentation-only change).
  - Watch out: no functional change — the `next_tick += tick_interval` alignment (rather than resetting to `now + interval`) is intentional for deterministic tick alignment.

### Box 8 — Add unit test for `World::update` fixed dt (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: Test simulates accumulator loop calling `World::update`; verifies every call receives exactly `TICK_DT` (1/60s).
- **Test**: Unit test (AC3).
- **Explain**
  - What: (test already written) `world_update_receives_fixed_tick_dt` holds `move_right` and asserts a single `update(TICK_DT)` advances the player by exactly `VELOCIDAD * TICK_DT`.
  - Why: proves `update` receives the fixed dt rather than the variable render dt (AC3) via its observable effect.
  - Rust concepts: `#[test]`; `assert!` with an f32 tolerance; fixed-point float arithmetic.
  - Watch out: the 0.001 tolerance absorbs f32 rounding; the test relies on the spawn area (100,100) being collision-free so the player actually advances.

### Box 9 — Add unit test for accumulator clamp (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: Test calls `World::update` in a simulated accumulator loop with large `frame_dt` (1.0s); verifies exactly 5 steps executed, accumulator clamped to `5 * TICK_DT`.
- **Test**: Unit test (AC9 — spiral of death protection).
- **Explain**
  - What: (test already written) `accumulator_clamp_large_frame_dt` feeds a 1.0s `frame_dt`, clamps, then asserts exactly `MAX_ACCUMULATED_FRAMES` (5) steps ran and the accumulator drained to 0.
  - Why: verifies the spiral-of-death clamp (AC9).
  - Rust concepts: accumulator loop with clamp; `assert_eq!`; `f32::EPSILON` comparison.
  - Watch out: it tests the *logic* the main.rs loop uses — the loop itself lives in `main.rs` (a binary) and is exercised by the e2e `gui_accumulator` test instead.

### Box 10 — Add unit test for interpolation helpers (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: Test constructs `World`, sets `prev_player_pos`/`player.pos` and `prev_player_dir`/`player.dir` to known values, calls `interpolated_player_pos(0.0)`, `(0.5)`, `(1.0)` and `interpolated_player_dir` equivalents; verifies lerp correctness.
- **Test**: Unit test (AC4 — draw alpha interpolation).
- **Explain**
  - What: (tests already written) `interpolated_player_pos_and_dir_lerp` and `interpolated_player_pos_clamps_alpha` verify lerp at 0.0/0.5/1.0 and alpha clamping.
  - Why: validates the pure interpolation helpers (AC4) and that out-of-range alpha does not extrapolate.
  - Rust concepts: `glam::Vec2::lerp`; `f32::EPSILON` tolerance comparisons; `std::f32::consts::FRAC_PI_2`.
  - Watch out: uses exact lerp expressions; the clamp test guards against a large first `frame_dt` producing alpha > 1.

### Box 11 — Add unit test for `client_tick` increments per sim tick (`client/src/world/game.rs`)

- [x] done
- **Acceptance**: Test simulates accumulator loop with multiple render frames; verifies `client_tick` increments once per `World::update` call (i.e., per sim tick), not per render frame.
- **Test**: Unit test (AC10).
- **Explain**
  - What: (test already written) `client_tick_increments_per_sim_tick` simulates 3 render frames at 30/60/10 FPS and asserts `client_tick` advances by exactly the number of sim steps (not the render-frame count).
  - Why: confirms `client_tick` is a per-sim-tick counter (AC10).
  - Rust concepts: accumulator simulation; `assert_eq!` on `u64` tick counters.
  - Watch out: `client_tick` is already incremented inside `update` (Phase 2f); the accumulator calling `update` once per tick makes this automatic.

### Box 12 — Add integration test for headless timing (`e2e/tests/headless_timing.rs`)

- [x] done
- **Acceptance**: Spawns the headless client via `cargo run -- --headless --duration=N`. Two assertions: (1) at `N=2`, stdout's "finished after N ticks" line reports exactly `N * TICK_HZ` ticks; (2) at `N=8`, elapsed wall time is within `[N-0.4s, N+2.5s]` (fixed-timestep loop must never finish early; upper bound absorbs `cargo run` + window-open overhead).
- **Test**: Integration test (AC5).
- **Explain**
  - What: (test already written) subprocess-based wall-clock test of the headless fixed-timestep loop.
  - Why: verifies the fixed-timestep loop neither drifts nor finishes early (AC5).
  - Rust concepts: `std::process::Command`; `std::time::Instant`; string parsing of stdout.
  - Watch out: relies on the `"finished after {} ticks"` line — Box 6 preserved it. Timing is wall-clock and subject to OS/`cargo run` overhead, hence the generous upper bound.

### Box 13 — Add integration test for GUI accumulator at different render rates (`e2e/tests/gui_accumulator.rs`)

- [x] done
- **Acceptance**: Simulates render loop at 30 FPS, 60 FPS, 144 FPS, 10 FPS; verifies sim steps/frame match expectations, `client_tick` advances correctly.
- **Test**: Integration test (AC6, AC9).
- **Explain**
  - What: (test already written) drives `World::update` through a simulated accumulator at 30/60/144/10 FPS plus a 1s pathological frame, asserting step counts and `client_tick` alignment.
  - Why: exercises the accumulator logic deterministically in the e2e harness (AC6, AC9) without a renderer.
  - Rust concepts: `TestHarness` + `ClientHandle`; a `simulate_frame` helper mirroring the main.rs accumulator.
  - Watch out: depends on Box 2a — before decoupling, `World::update` called macroquad input and would panic here.

### Box 14 — Add integration test for packet log input rate (`e2e/tests/packet_log_rate.rs`)

- [x] done
- **Acceptance**: Runs client with `RUSTVANIA_PACKET_LOG=1`, captures log; verifies `PlayerInput` packets sent at ~16.666ms intervals (60Hz).
- **Test**: Integration test (AC8).
- **Explain**
  - What: (test already written) starts a server + headless client with packet logging and parses `ts_ms` of `PlayerInput` sends to check count and average interval.
  - Why: verifies the fixed 60Hz input send rate (AC8), a natural consequence of `update_network` running once per sim tick.
  - Rust concepts: `std::process::Command` (spawn/kill/wait); JSONL timestamp parsing; `std::env::set_var`.
  - Watch out: `ts_ms` is millisecond-quantized (16/17ms), so the test asserts count robustly and interval coarsely (±2ms).

### Box 15 — Add integration test for server catch-up under load (`e2e/tests/server_catchup.rs`)

- [x] done
- **Acceptance**: Tests the invariant the catch-up loop relies on (the `while now >= next_tick` loop itself is in the server binary, verified by code review in Box 7): `ServerWorld::update` advances `current_tick` exactly once per call; a burst of queued inputs is fully consumed across a catch-up (none dropped); `current_tick` is monotonic under rapid back-to-back ticks.
- **Test**: Integration test (AC7).
- **Explain**
  - What: (test already written) asserts `ServerWorld::update` advances `current_tick` by exactly 1 per call, that a backlog of inputs is fully applied, and that the tick counter is monotonic under rapid ticks.
  - Why: verifies the invariants the server catch-up loop relies on (AC7); the loop itself lives in the binary and is covered by Box 7's review.
  - Rust concepts: `TestHarness::tick_n`/`hold_right`; assertions on `ServerWorld::current_tick`.
  - Watch out: the actual `while now >= next_tick` loop is not directly testable from the library — this test covers its foundational invariant instead.

### Box 16 — Verify fixed-rate input send (verification, merged with integration tests)

- [x] done
- **Acceptance**: Verified via Box 14 (packet log) and Box 13 (GUI accumulator) that `update_network` called once per `World::update` → once per sim tick → 60Hz `PlayerInput` send rate. No separate code change needed.
- **Test**: Covered by Box 13, Box 14.
- **Explain**
  - What: No code change — confirmed `update_network` is called at the end of `World::update`, which the accumulator now calls once per sim tick.
  - Why: the fixed 60Hz input send rate (G4) is a natural consequence of the accumulator, not a separate mechanism.
  - Rust concepts: none (verification-only).
  - Watch out: headless and e2e already called `update` once per tick; the GUI accumulator makes the GUI path consistent.

### Box 17 — Run full test suite and quality gates

- [x] done
- **Acceptance**: `cargo test` passes (172+ tests), `cargo fmt --check`, `cargo clippy -- -D warnings` clean.
- **Test**: Regression (AC11, AC12 in spec).
- **Explain**
  - What: Intended to run `cargo test`, `cargo fmt --check`, and `cargo clippy -- -D warnings`.
  - Why: final regression gate (AC11/AC12).
  - Rust concepts: n/a.
  - Watch out: **BLOCKED — see handoff note.** The implementer's shell was denied in this environment, so `cargo test`/`fmt`/`clippy` could not be executed here. The code was reviewed by hand against the RED-test contract. The orchestrator must run the three gates and commit the work (see report).

## Open questions (resolved)

**Q1: Headless busy-wait CPU cost** — Owner confirmed: **1ms busy-wait (Recommended)** — best accuracy, negligible CPU on modern CI runners.

**Q2: Nemesis alpha interpolation** — Owner confirmed: **Local player only (Recommended)** — nemesis tick-based interpolation is more accurate for networked entities.

---

## Notes for implementer

- **Order matters**: Do **Boxes 1-2 first** (config + World fields), then **Box 2a (input decoupling)** — it is a hard prerequisite, ordered before both the GUI accumulator loop (Box 5) and every unit test that calls `World::update` (Boxes 3, 8, 9, 11): without it those tests panic on macroquad input. Then **Box 3** (store prev position), then **Box 4 + Box 5 atomically** (draw signature + GUI loop; Box 5 builds the accumulator loop on top of Box 2a's injected-input sampling), then Box 6 (headless), then Box 7 (server comment), then Boxes 8-11 (unit tests), then Boxes 12-15 (integration tests), then Box 16 (verification), then Box 17 (final validation).
- **Input decoupling (Box 2a)**: `World::update` must never call `is_key_pressed`/`is_key_down` — macroquad input functions panic when there is no window, which breaks every unit test that calls `update` (Boxes 3, 8, 9, 11) and the e2e `gui_accumulator` test. The GUI loop in `main.rs` samples keyboard state once per render frame and writes it into `World`'s `move_left`/`move_right`/`pressed_jump`/`pressed_shoot` fields before calling `update`. Move the F11 debug reset to `main.rs` too (or drop it). Headless mode performs no sampling — its fields stay `false`.
- **`client_tick`**: Already incremented in `World::update` (Phase 2f). Accumulator loop calls `update` once per sim tick → `client_tick` naturally increments at 60Hz.
- **Input send**: `update_network` is called at end of `World::update` → automatically once per sim tick. No changes to `network/world.rs` needed.
- **First frame edge case**: In GUI accumulator, first `frame_dt` may be large; clamp protects. `prev_player_pos` initialized to `player.pos` (spawn position `(100.0, 100.0)`) in `World::new()`, so the first rendered frame matches the current position with no jump.
- **Alpha at 1.0**: When `accumulator == TICK_DT`, `alpha = 1.0` but loop runs another step immediately. Render with `alpha ≈ 1.0` shows position very close to next tick — acceptable. Clamp alpha to `< 1.0` if preferred.
- **Direction lerp**: `prev_player_dir` and `player.dir` are angles (radians). Simple `lerp` works for small angles but may cross -π/π boundary. For Phase 2g, accept simple lerp; document in Box 4.
- **Server loop**: No changes needed. Verification comment references spec decision D3 for traceability.
- **Test location**: Client is a binary crate; unit tests go in `client/src/world/game.rs` under `#[cfg(test)]`. Integration tests go in `e2e/tests/`.
- **Consistent naming**: Use "sim tick" throughout (not "simulation step").

## Review follow-up fix (post-implementation, 2026-11-25)

- **What**: In `client/src/world/lib.rs`, after a room transition teleports the player (`check_level_transitions`, inside the `if changed` block after `load_current_level`) and after the death respawn teleport (`handle_death`, right after `player.pos = (100.0, 100.0)`), reset `world.prev_player_pos = world.player.pos` and `world.prev_player_dir = world.player.dir`.
- **Why**: `World::update` stores `prev_player_*` *before* simulating, then `check_level_transitions` teleports `player.pos` afterward. On the transition frame, `draw(alpha)` would lerp from the old-room position to the new-room entry position for one frame while the level is already rendered at the new coords — a one-frame visual glitch. Snapping the interpolation source to the post-teleport position makes `draw` render at the new spawn.
- **Rust concepts**: mutable-field assignment on `&mut World`; disjoint field writes (`world.prev_player_pos`/`prev_player_dir` vs `world.player.pos`/`dir`); the borrow of `p: &mut world.player` has already ended (NLL) before the `if changed` block, so re-reading `world.player.pos` is allowed.
- **Watch out**: The death path (`handle_death`) is called *before* the prev-state store in `update`, so it was already correct — the added reset there is purely defensive and preserves the invariant that `prev_player_*` never holds a stale pre-teleport position. No test was changed; existing interpolation tests are unaffected because they set `prev_player_*` explicitly.

## QA finding m2 refactor — extract shared `FixedTimestep` (2026-11-25)

- **What**: Extracted the accumulate → clamp → step → alpha accumulator into a shared
  pure helper, `game::fixed_timestep::FixedTimestep` (new module, re-exported from
  `game/src/lib.rs`). Both `client/src/main.rs`'s GUI loop and `e2e/tests/gui_accumulator.rs`
  now drive their simulation through it, removing the duplicated inline arithmetic.
  Added `FixedTimestep::with_max_accumulator` (decouples the clamp from the guard)
  and two unit tests, one of which proves the `steps < max_frames` guard actually
  binds.
- **Why**: QA m2 — the accumulator logic was copy-pasted between production and the
  e2e test, and the spiral-of-death guard (`steps < MAX_ACCUMULATED_FRAMES`) was
  provably dead: with `max_accumulator = max_frames * tick_dt`, the clamp already
  bounds the accumulator to `max_frames` ticks, so the guard could never be the
  binding constraint and no test would fail if it were removed.
- **Rust concepts**: struct-with-methods encapsulation of stateful loop logic;
  `&mut self` on `advance` (it mutates the accumulator) vs `&self` on `alpha`
  (read-only); `u32` step counting; private fields exposed only through public
  constructors (`new` derives `max_accumulator`, `with_max_accumulator` lets a test
  decouple it); `#[cfg(test)]` unit-test module with `use crate::config::TICK_HZ`.
- **Watch out**: The guard test *must* decouple `max_accumulator` from `max_frames`
  (via `with_max_accumulator`) or the clamp masks the guard and the test becomes
  tautological. `advance` returns a step *count*; callers still own the actual
  `World::update(TICK_DT)` call (production computes in f64, casts to f32 only at
  the `update` boundary). The client unit tests (Box 9/11 in `client/src/world/game.rs`)
  still contain inline accumulator arithmetic — left as-is to keep this refactor
  scoped (candidate for a follow-up).