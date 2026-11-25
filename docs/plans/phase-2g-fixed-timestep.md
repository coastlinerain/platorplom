# Phase 2g — Fixed Timestep & Accumulator Pattern — Implementation Plan

- Status: draft
- Spec: `docs/specs/phase-2g-fixed-timestep.md`
- Date: 2026-11-25

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
6. **Direction lerp boundary** — `interpolated_player_dir` handles -π/π crossing (simple lerp accepted for small per-tick changes; document limitation).

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

- [ ] done
- **Acceptance**: `cargo build` succeeds; `MAX_ACCUMULATED_FRAMES = 5`, `HEADLESS_BUSY_WAIT_MS = 1` exported.
- **Test**: Unit test compiles (AC1 in spec).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 2 — Add `prev_player_pos` and `prev_player_dir` to `World` (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: Fields added to `World` struct; initialized in `World::new()` to `player.pos` and `player.dir` (note: at this point `player.pos` is (0,0) — server assigns real position later via `PlayerPos`; first frame renders at origin briefly, which is acceptable).
- **Test**: Unit test compiles; `World::new()` initializes correctly.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 2a — Decouple `World::update` from macroquad input (`client/src/world/game.rs`, `client/src/main.rs`)

- [ ] done
- **Acceptance**:
  - `World::update` no longer calls any macroquad input function (`is_key_pressed`, `is_key_down`). It reads the injected programmatic fields `move_left`, `move_right`, `pressed_jump`, `pressed_shoot` (already present on `World`) and builds the local `PlayerInput` from them.
  - The F11 clock-reset debug block is removed from `update` (it also calls `is_key_pressed`); moved to the GUI loop or dropped.
  - In `client/src/main.rs`'s GUI loop, immediately before `World::update`, sample `is_key_down(KeyCode::Left/Right)` and `is_key_pressed(KeyCode::Space/Z)` once per render frame and write them into the corresponding `World` fields (plus F11 if kept).
  - `cargo build` succeeds; `World::update` runs under `#[cfg(test)]` with no macroquad window (no panic).
- **Test**: Existing RED unit tests Boxes 3, 8, 9, 11 (`update_stores_previous_player_state`, `world_update_receives_fixed_tick_dt`, `accumulator_clamp_large_frame_dt`, `client_tick_increments_per_sim_tick`) and the e2e `gui_accumulator` test (Box 13) stop panicking and produce meaningful assertions.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 3 — Store previous position before player update in `World::update` (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: In `World::update`, before `self.player.update(...)`, store `self.prev_player_pos = self.player.pos` and `self.prev_player_dir = self.player.dir`.
- **Test**: Unit test — after one `update` call, `prev_player_pos` equals old position.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 4 — Add interpolation helpers and modify `World::draw` to accept `alpha` (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: 
  - Added pure methods `interpolated_player_pos(alpha: f32) -> Vec2` and `interpolated_player_dir(alpha: f32) -> f32` that return `lerp(prev, current, alpha)` with alpha clamped to [0,1].
  - `draw(&self, alpha: f32)` calls these helpers for local player render position/direction.
  - **Atomic with Box 5**: `draw()` signature change and GUI call site update MUST be in the same commit or code won't compile between boxes.
- **Test**: Unit test (AC4) — `interpolated_player_pos(0.0)` = prev, `(0.5)` = midpoint, `(1.0)` = current; same for dir.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out: Direction lerp uses simple linear interpolation; may produce artifacts if `prev_player_dir` and `player.dir` cross -π/π boundary. Accepted for Phase 2g (small per-tick changes); can improve with angle-aware lerp later.

### Box 5 — Implement accumulator loop in GUI main (`client/src/main.rs`)

- [ ] done
- **Acceptance**: GUI loop uses accumulator pattern: accumulates `frame_dt`, runs 0-5 `update(TICK_DT)` steps per frame, computes `alpha = accumulator / TICK_DT`, calls `draw(alpha)`. `client_tick` increments once per sim tick (already happens in `World::update`). **Atomic with Box 4** — same commit.
- **Test**: Integration test (AC6, AC9) — simulate 30 FPS (2 steps/frame), 10 FPS (5 steps/frame clamped), 144 FPS (0 or 1 step/frame).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 6 — Implement hybrid sleep + busy-wait in headless loop (`client/src/main.rs`)

- [ ] done
- **Acceptance**: `run_headless` changed to **non-async fn** (removes `async`/`await` since it uses blocking `std::thread::sleep` + `spin_loop`). Uses `next_tick += tick_interval` with sleep for `remaining - HEADLESS_BUSY_WAIT_MS` then `spin_loop` for last ~1ms. No `sleep(16)`.
- **Test**: Integration test (AC5) — 1000 ticks in 16.666s ±0.1s; verify no drift over 10s.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 7 — Verify server catch-up loop and add comment (`server/src/main.rs`)

- [ ] done
- **Acceptance**: Comment added at server tick loop (line ~295) documenting verification per **spec decision D3**: `tick_interval` exact (`Duration::from_secs_f64(1.0/60.0)`), `while now >= next_tick` catch-up, sleep `min(5ms)`, `next_tick += tick_interval` no drift, no busy-spin.
- **Test**: Code review (AC7); unit test for server timing if feasible.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 8 — Add unit test for `World::update` fixed dt (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: Test simulates accumulator loop calling `World::update`; verifies every call receives exactly `TICK_DT` (1/60s).
- **Test**: Unit test (AC3).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 9 — Add unit test for accumulator clamp (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: Test calls `World::update` in a simulated accumulator loop with large `frame_dt` (1.0s); verifies exactly 5 steps executed, accumulator clamped to `5 * TICK_DT`.
- **Test**: Unit test (AC9 — spiral of death protection).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 10 — Add unit test for interpolation helpers (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: Test constructs `World`, sets `prev_player_pos`/`player.pos` and `prev_player_dir`/`player.dir` to known values, calls `interpolated_player_pos(0.0)`, `(0.5)`, `(1.0)` and `interpolated_player_dir` equivalents; verifies lerp correctness.
- **Test**: Unit test (AC4 — draw alpha interpolation).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 11 — Add unit test for `client_tick` increments per sim tick (`client/src/world/game.rs`)

- [ ] done
- **Acceptance**: Test simulates accumulator loop with multiple render frames; verifies `client_tick` increments once per `World::update` call (i.e., per sim tick), not per render frame.
- **Test**: Unit test (AC10).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 12 — Add integration test for headless timing (`e2e/tests/headless_timing.rs`)

- [ ] done
- **Acceptance**: Spawns headless client for 1000 ticks, measures wall time; asserts 16.566s < elapsed < 16.766s.
- **Test**: Integration test (AC5).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 13 — Add integration test for GUI accumulator at different render rates (`e2e/tests/gui_accumulator.rs`)

- [ ] done
- **Acceptance**: Simulates render loop at 30 FPS, 60 FPS, 144 FPS, 10 FPS; verifies sim steps/frame match expectations, `client_tick` advances correctly.
- **Test**: Integration test (AC6, AC9).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 14 — Add integration test for packet log input rate (`e2e/tests/packet_log_rate.rs`)

- [ ] done
- **Acceptance**: Runs client with `RUSTVANIA_PACKET_LOG=1`, captures log; verifies `PlayerInput` packets sent at ~16.666ms intervals (60Hz).
- **Test**: Integration test (AC8).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 15 — Add integration test for server catch-up under load (`e2e/tests/server_catchup.rs`)

- [ ] done
- **Acceptance**: Simulates event drain taking 10ms; verifies server catch-up runs 1 tick, sleeps remainder, no busy-spin.
- **Test**: Integration test (AC7).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 16 — Verify fixed-rate input send (verification, merged with integration tests)

- [ ] done
- **Acceptance**: Verified via Box 14 (packet log) and Box 13 (GUI accumulator) that `update_network` called once per `World::update` → once per sim tick → 60Hz `PlayerInput` send rate. No separate code change needed.
- **Test**: Covered by Box 13, Box 14.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 17 — Run full test suite and quality gates

- [ ] done
- **Acceptance**: `cargo test` passes (172+ tests), `cargo fmt --check`, `cargo clippy -- -D warnings` clean.
- **Test**: Regression (AC11, AC12 in spec).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

## Open questions (resolved)

**Q1: Headless busy-wait CPU cost** — Owner confirmed: **1ms busy-wait (Recommended)** — best accuracy, negligible CPU on modern CI runners.

**Q2: Nemesis alpha interpolation** — Owner confirmed: **Local player only (Recommended)** — nemesis tick-based interpolation is more accurate for networked entities.

---

## Notes for implementer

- **Order matters**: Do **Boxes 1-2 first** (config + World fields), then **Box 2a (input decoupling)** — it is a hard prerequisite, ordered before both the GUI accumulator loop (Box 5) and every unit test that calls `World::update` (Boxes 3, 8, 9, 11): without it those tests panic on macroquad input. Then **Box 3** (store prev position), then **Box 4 + Box 5 atomically** (draw signature + GUI loop; Box 5 builds the accumulator loop on top of Box 2a's injected-input sampling), then Box 6 (headless), then Box 7 (server comment), then Boxes 8-11 (unit tests), then Boxes 12-15 (integration tests), then Box 16 (verification), then Box 17 (final validation).
- **Input decoupling (Box 2a)**: `World::update` must never call `is_key_pressed`/`is_key_down` — macroquad input functions panic when there is no window, which breaks every unit test that calls `update` (Boxes 3, 8, 9, 11) and the e2e `gui_accumulator` test. The GUI loop in `main.rs` samples keyboard state once per render frame and writes it into `World`'s `move_left`/`move_right`/`pressed_jump`/`pressed_shoot` fields before calling `update`. Move the F11 debug reset to `main.rs` too (or drop it). Headless mode performs no sampling — its fields stay `false`.
- **`client_tick`**: Already incremented in `World::update` (Phase 2f). Accumulator loop calls `update` once per sim tick → `client_tick` naturally increments at 60Hz.
- **Input send**: `update_network` is called at end of `World::update` → automatically once per sim tick. No changes to `network/world.rs` needed.
- **First frame edge case**: In GUI accumulator, first `frame_dt` may be large; clamp protects. `prev_player_pos` initialized to (0,0) in `World::new()`; server assigns real position via `PlayerPos` later — first few frames render at origin, acceptable.
- **Alpha at 1.0**: When `accumulator == TICK_DT`, `alpha = 1.0` but loop runs another step immediately. Render with `alpha ≈ 1.0` shows position very close to next tick — acceptable. Clamp alpha to `< 1.0` if preferred.
- **Direction lerp**: `prev_player_dir` and `player.dir` are angles (radians). Simple `lerp` works for small angles but may cross -π/π boundary. For Phase 2g, accept simple lerp; document in Box 4.
- **Server loop**: No changes needed. Verification comment references spec decision D3 for traceability.
- **Test location**: Client is a binary crate; unit tests go in `client/src/world/game.rs` under `#[cfg(test)]`. Integration tests go in `e2e/tests/`.
- **Consistent naming**: Use "sim tick" throughout (not "simulation step").