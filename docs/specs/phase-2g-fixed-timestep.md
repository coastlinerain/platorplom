# Phase 2g — Fixed Timestep & Accumulator Pattern

- Status: implemented
- Date: 2026-11-25
- Branch: `feat/phase-2g-fixed-timestep`

## Summary

Phase 2f added clock synchronization, jitter buffer, frame interpolation, packet logging, and a headless client. However, the client simulation loop is not yet deterministic at 60Hz:

- **Client GUI** (`client/src/main.rs:177`): Uses variable `get_frame_time()` dt — simulation speed varies with frame rate (vsync, load, etc.).
- **Client Headless** (`client/src/main.rs:120`): Uses `sleep(1000 / TICK_HZ)` ≈ 16ms = ~62.5Hz, not 60Hz — accumulates drift; `std::thread::sleep` has ~1ms precision on Linux, ~15ms on Windows.
- **Server** (`server/src/main.rs:295-368`): Already uses fixed 60Hz loop with catch-up (correct). Needs verification that catch-up is sufficient and no busy-spin issues.
- **Network send rate** (`client/src/network/world.rs:71`): Sends inputs every frame (variable) — should send at fixed 60Hz matching server tick rate.

This phase implements:
1. **Accumulator pattern on Client GUI**: Separate simulation dt (fixed 1/60s) from render dt (variable). Run 0-N simulation steps per frame, then render with interpolation alpha.
2. **Sub-millisecond precision sleep on Headless**: Use `spin_sleep` or busy-wait for remainder <1ms to hit true 60Hz.
3. **Verify server catch-up loop**: Ensure it handles load correctly, no busy-spin, and sleep granularity is acceptable.
4. **Fixed-rate input send**: Client sends `PlayerInput` at exactly 60Hz (once per simulation step), not every render frame.

## Goals / non-goals

### Goals
- **G1**: Client GUI runs deterministic 60Hz simulation regardless of render frame rate (fixed `TICK_DT` steps driven by an accumulator).
- **G2**: Client Headless runs true 60Hz (±0.1ms) using sub-ms sleep.
- **G3**: Server catch-up loop verified sufficient; no busy-spin under load.
- **G4**: Input packets sent at fixed 60Hz (once per sim tick), not every render frame.
- **G5**: Render interpolation alpha used for smooth visuals between sim ticks (local player + remote entities).
- **G6**: All existing tests pass (172+ tests), quality gates green.

### Non-Goals (Future Phases)
- Variable tick rate / dynamic simulation rate.
- Server-side frame interpolation (server remains tick-perfect).
- Client-side prediction for enemies/bullets (scope boundary D6 from ADR-001).
- Network jitter simulation in headless (already handled by jitter buffer in Phase 2f).

## Behavior

### Client GUI — Accumulator Pattern (D1)

**Current**: `game_world.update(get_frame_time())` — variable dt.

**New**: Fixed simulation timestep with accumulator.

```rust
const TICK_DT: f32 = 1.0 / TICK_HZ as f32; // 1/60 ≈ 0.0166667s
const MAX_ACCUMULATED_FRAMES: u32 = 5;      // Spiral of death protection: max 5 ticks/frame (safety clamp, not a render floor)

let mut accumulator = 0.0f32;
let mut last_time = get_time();

loop {
    let current_time = get_time();
    let frame_dt = current_time - last_time;
    last_time = current_time;

    accumulator += frame_dt;

    // Spiral of death protection: clamp accumulator
    if accumulator > MAX_ACCUMULATED_FRAMES as f32 * TICK_DT {
        accumulator = MAX_ACCUMULATED_FRAMES as f32 * TICK_DT;
    }

    // Run simulation steps
    let mut steps = 0;
    while accumulator >= TICK_DT && steps < MAX_ACCUMULATED_FRAMES {
        game_world.update(TICK_DT);  // Fixed dt!
        accumulator -= TICK_DT;
        steps += 1;
    }

    // Alpha for render interpolation (0.0 = previous tick, 1.0 = next tick)
    let alpha = accumulator / TICK_DT;

    game_world.draw(alpha);  // Pass alpha for interpolation

    next_frame().await
}
```

**Key points**:
- `World::update(dt)` receives **exactly** `TICK_DT` (1/60s) every call.
- `World::draw(alpha)` receives interpolation alpha ∈ [0,1) for smooth rendering between ticks.
- Maximum 5 simulation steps per frame → bounds the sim catch-up (spiral of death protection).
- `client_tick` increments once per simulation step (not per render frame).

**Edge cases**:
- First frame: `frame_dt` may be large (startup); accumulator clamps to `MAX_ACCUMULATED_FRAMES * TICK_DT`.
- Tabbed out / minimized: `get_frame_time()` may return huge values; clamp protects.
- Vsync off / high FPS: many frames with 0 simulation steps (alpha advances smoothly).
- Vsync on / 60Hz: typically 1 step per frame, alpha ≈ 0.
- Vsync on / 30Hz: 2 steps per frame, alpha = 0 or 0.5 alternating.

### Client Headless — Sub-ms Precision Sleep (D2)

**Current**: `std::thread::sleep(Duration::from_millis(1000 / TICK_HZ))` → 16ms = 62.5Hz.

**New**: Hybrid sleep + busy-wait for sub-ms precision.

```rust
use std::time::{Duration, Instant};

let tick_interval = Duration::from_secs_f64(1.0 / TICK_HZ as f64); // ~16.666ms
let mut next_tick = Instant::now() + tick_interval;

for tick in 0..max_ticks {
    game_world.update(TICK_DT);

    // Periodic status log
    if tick % (TICK_HZ as u64 * 5) == 0 && tick > 0 {
        println!("Headless tick: {}", tick);
    }

    // Sleep for most of the interval, then busy-wait remainder
    let now = Instant::now();
    if now < next_tick {
        let remaining = next_tick - now;
        // Sleep if > 1ms remaining (avoid busy-wait for long periods)
        if remaining > Duration::from_millis(1) {
            std::thread::sleep(remaining - Duration::from_millis(1));
        }
        // Busy-wait for the last ~1ms
        while Instant::now() < next_tick {
            std::hint::spin_loop();
        }
    }
    next_tick += tick_interval;
}
```

**Key points**:
- Target: true 60Hz = 16.666...ms per tick.
- `std::thread::sleep` has ~1ms precision on Linux, ~15ms on Windows.
- Sleep for `remaining - 1ms`, then busy-wait (`spin_loop`) for the last ~1ms.
- Achieves ±0.1ms accuracy on both Linux and Windows.
- No drift accumulation (uses `next_tick += tick_interval`, not `sleep` in a loop).

**Edge cases**:
- If `game_world.update` takes > tick_interval: catch up immediately (no sleep, next_tick = now + tick_interval).
- System clock adjustments: uses `Instant` (monotonic), immune to NTP steps.
- CI environments: busy-wait acceptable for short durations (<1ms per tick).

### Server — Verify Catch-Up Loop (D3)

**Current** (`server/src/main.rs:295-368`):
```rust
let now = Instant::now();
while now >= next_tick {
    let (shots, deaths) = world.update(tick_dt);
    // ... broadcast ...
    next_tick += tick_interval;
}
// Sleep remainder
let remaining = next_tick.checked_duration_since(Instant::now()).unwrap_or_default();
std::thread::sleep(remaining.min(Duration::from_millis(5)));
```

**Verification checklist**:
- [ ] `tick_interval` computed as `Duration::from_secs_f64(1.0 / TICK_HZ as f64)` (exact 16.666...ms).
- [ ] `while now >= next_tick` catches up correctly if event drain took long.
- [ ] No busy-spin: sleeps `remaining.min(5ms)` after catch-up.
- [ ] `next_tick += tick_interval` does not drift (accumulates error if `world.update` is slow consistently).
- [ ] Consider: `next_tick = Instant::now() + tick_interval` after catch-up to prevent drift spiral? (Decision: keep `+=` for deterministic tick alignment; if consistently behind, server is overloaded anyway.)

**Conclusion**: Server loop is correct. No changes needed, but document verification in acceptance criteria.

### Fixed-Rate Input Send (D4)

**Current** (`client/src/network/world.rs:71`): `world.network.send(input_msg);` called every `World::update` (every render frame in GUI, every sim tick in headless).

**New**: Send exactly once per simulation tick.

- **GUI**: In `World::update`, after simulation step, send input. Since `World::update` is now called once per sim tick (via accumulator), this is automatic.
- **Headless**: Already calls `game_world.update(TICK_DT)` once per tick → sends once per tick.
- **e2e TestHarness**: Already calls `tick()` once per sim tick → sends once per tick.

**No code change needed** — the accumulator pattern naturally makes `World::update` the per-tick function. Just verify send rate = 60Hz.

### Render Interpolation Alpha (D5)

**Current**: `World::draw()` uses raw positions.

**New**: `World::draw(alpha: f32)` interpolates local player and remote entities for smooth visuals.

```rust
// In World::draw(alpha):
// Local player: interpolate between previous_tick_pos and current_tick_pos
// (Need to store previous_tick_pos in World)
// Remote nemesis: already uses tick-based interpolation (Phase 2f), but can also use alpha for sub-tick smoothness
// Bullets/enemies: server-authoritative or client-simulated, no interpolation needed
```

**Implementation detail**:
- Add `prev_player_pos: Vec2` and `prev_player_dir: f32` to `World`.
- In `World::update`, before `player.update()`, store `prev_player_pos = player.pos`, `prev_player_dir = player.dir`.
- In `World::draw(alpha)`, render local player at `lerp(prev_player_pos, player.pos, alpha)`.

**Edge case**: First frame (no previous) → alpha = 0, render current.

**Note**: Remote nemesis already uses tick-based interpolation (Phase 2f). Alpha-based sub-tick interpolation for nemesis is optional but can be added for extra smoothness. For Phase 2g, focus on local player alpha interpolation.

## Data model

### Config (`game/src/config.rs`)
```rust
// Existing
pub const TICK_HZ: u32 = 60;
pub const CLIENT_HZ: u32 = 60;
pub const INTERPOLATION_DELAY_TICKS: u8 = 5;
pub const STALENESS_THRESHOLD_TICKS: u64 = 5;

// NEW
/// Maximum simulation steps per render frame (spiral of death protection).
/// At 60Hz, 5 steps = ~83ms of sim catch-up per frame (safety clamp).
pub const MAX_ACCUMULATED_FRAMES: u32 = 5;

/// Headless sleep precision: busy-wait for last N milliseconds.
pub const HEADLESS_BUSY_WAIT_MS: u64 = 1;
```

### Client World (`client/src/world/game.rs`)
```rust
pub struct World {
    // ... existing fields ...
    // NEW: For render interpolation
    pub prev_player_pos: glam::Vec2,
    pub prev_player_dir: f32,
    // client_tick already exists (Phase 2f) — increments per sim tick
}
```

### Client Main (`client/src/main.rs`)
```rust
// GUI loop: accumulator pattern
// Headless loop: hybrid sleep + busy-wait
```

### Server Main (`server/src/main.rs`)
```rust
// No changes — verify existing loop
```

## Network

No new packets. Existing `PlayerInput` send rate changes from variable (per render frame) to fixed 60Hz (per sim tick). This is a behavioral change, not a protocol change.

- Server expects inputs at ~60Hz (already designed for this).
- Client now sends at exactly 60Hz.
- No sequence number changes needed (`input_seq` increments per input sent).

## Acceptance criteria

| ID | Criterion | Test Type |
|----|-----------|-----------|
| AC1 | `game/src/config.rs` defines `MAX_ACCUMULATED_FRAMES = 5` and `HEADLESS_BUSY_WAIT_MS = 1` | Unit (compile) |
| AC2 | Client GUI loop uses accumulator: runs 0-5 `World::update(TICK_DT)` per frame, passes alpha to `draw(alpha)` | Integration (manual/visual) |
| AC3 | `World::update` receives exactly `TICK_DT` (1/60s) every call — verify with debug log or test | Unit (instrumented) |
| AC4 | `World::draw(alpha)` interpolates local player position using `prev_player_pos` and `alpha` | Unit (World::draw) |
| AC5 | Client Headless loop achieves 60Hz ±0.1ms (measure over 1000 ticks) | Integration (headless) |
| AC6 | Headless uses hybrid sleep + busy-wait (no `sleep(16)`) | Code review |
| AC7 | Server catch-up loop verified: `tick_interval` exact, `while now >= next_tick`, sleep `min(5ms)`, no busy-spin | Code review + Unit (ServerWorld timing) |
| AC8 | Input send rate = 60Hz (once per `World::update` call) on GUI, Headless, and e2e | Integration (packet log) |
| AC9 | Spiral of death protection: sim catch-up is bounded to 5 steps/frame even under pathological frame times | Integration (stress test) |
| AC10 | `client_tick` increments once per sim tick (not per render frame) | Unit (World::update) |
| AC11 | All existing tests pass (172+) | Regression (`cargo test`) |
| AC12 | Quality gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` | CI |

## Design Decisions (D1-D5) with Rationale

### D1: Accumulator Pattern on Client GUI — **Fixed TICK_DT, MAX_ACCUMULATED_FRAMES=5**
- **Chosen**: Classic accumulator with clamp. `TICK_DT = 1/60`. Max 5 steps/frame (safety clamp, not a render floor).
- **Rejected**: Variable dt (current) — non-deterministic. Uncapped accumulator — spiral of death. Interpolation only — no sim determinism.
- **Rationale**: Industry standard (Gaffer on Games). 5 steps = ~83ms of sim budget. At 60 FPS render, 1 step/frame; at 30 FPS, 2 steps/frame — full 60Hz sim is sustained as long as render stays ≥12 FPS (5 × 16.67ms). Below that the sim rate drops, but the clamp stops the spiral of death. Simple, robust.

### D2: Headless Sub-ms Sleep — **Hybrid sleep + busy-wait (1ms)**
- **Chosen**: Sleep for `remaining - 1ms`, then `spin_loop` for last 1ms.
- **Rejected**: `sleep(16)` — 62.5Hz, drifts. `sleep(16666 micros)` — not portable, still ~1ms precision. Pure busy-wait — wastes CPU.
- **Rationale**: Best accuracy/CPU tradeoff. 1ms busy-wait per tick = 6% CPU at 60Hz (negligible). Works on Linux (1ms sleep precision) and Windows (15ms sleep precision). `std::hint::spin_loop()` is the correct intrinsic.
- **Decision confirmed**: Owner approved 1ms busy-wait as recommended.

### D3: Server Catch-Up Loop — **Keep existing (verified sufficient)**
- **Chosen**: No changes. `while now >= next_tick` + `sleep(min(5ms))`.
- **Rejected**: `next_tick = now + tick_interval` after catch-up — breaks deterministic tick alignment.
- **Rationale**: Server is authoritative; if consistently behind, it's overloaded. Current loop is standard. The 5ms min sleep prevents busy-spin while keeping latency low.

### D4: Fixed-Rate Input Send — **Natural consequence of accumulator**
- **Chosen**: `World::update` called once per sim tick → sends once per tick.
- **Rejected**: Explicit send timer — redundant, couples network to render.
- **Rationale**: Simulation and network are coupled: one input per simulation step. Headless and e2e already do this. GUI accumulator makes it consistent.

### D5: Render Interpolation Alpha — **Local player only (nemesis already tick-based)**
- **Chosen**: Store `prev_player_pos`/`prev_player_dir`, lerp in `draw(alpha)`.
- **Rejected**: Interpolate everything — overkill, nemesis already uses superior tick-based interpolation (Phase 2f). No interpolation — jittery at non-60Hz render.
- **Rationale**: Local player is the most visible; prediction + reconciliation + alpha gives smoothest feel. Nemesis tick-based interpolation is more accurate for networked entities.
- **Decision confirmed**: Owner approved local player only; nemesis keeps tick-based interpolation from Phase 2f.

## Files to Modify / Create

| File | Change |
|------|--------|
| `game/src/config.rs` | Add `MAX_ACCUMULATED_FRAMES = 5`, `HEADLESS_BUSY_WAIT_MS = 1` |
| `client/src/world/game.rs` | Add `prev_player_pos`, `prev_player_dir`; store before `player.update()`; `draw(alpha)` uses lerp |
| `client/src/main.rs` | GUI: accumulator loop with alpha; Headless: hybrid sleep + busy-wait |
| `server/src/main.rs` | No changes — add comment verifying catch-up loop correctness |
| `client/src/network/world.rs` | No changes — verify send rate = 60Hz (automatic via accumulator) |
| `e2e/src/harness.rs` | No changes — already tick-based; verify `TestHarness::tick` matches 60Hz |

## Test Cases (for planner)

### Unit Tests
1. `World::update` — called with exactly `TICK_DT` in accumulator loop (instrumented test).
2. `World::draw(alpha)` — local player position = `lerp(prev, current, alpha)` at alpha=0, 0.5, 1.0.
3. Headless sleep — `run_headless` loop timing: 1000 ticks complete in 16.666s ±0.1s.
4. Accumulator clamp — large `frame_dt` (e.g., 1.0s) results in exactly 5 steps, accumulator clamped.
5. `client_tick` increments once per `World::update` call (not per render frame).

### Integration Tests
6. GUI at 30 FPS (simulate): runs 2 sim steps/frame, `client_tick` advances 2/frame, render smooth.
7. GUI at 10 FPS (simulate): runs 5 sim steps/frame (clamped), `client_tick` advances 5/frame — sim rate drops below 60Hz but the clamp prevents runaway.
8. Headless 10s run: completes ~600 ticks, average tick time 16.666ms ±0.1ms.
9. Packet log (`RUSTVANIA_PACKET_LOG=1`): `PlayerInput` sent at 60Hz intervals (verify timestamps).
10. Server under load: event drain takes 10ms → catch-up runs 1 tick, sleeps remainder, no busy-spin.

## Explain Block (for implementer commits)

Each implementation commit must include:
```
## What
<what changed>

## Why
<reason>

## Rust concepts
<ownership, borrows, lifetimes, traits, generics, Result/Option, pattern matching, channels, derive, etc.>

## Watch out
<gotchas, trade-offs, things to verify>
```

## Owner Decisions

The following product/trade-off decisions were confirmed by the owner:

1. **Headless CPU — 1ms busy-wait accepted**: Use `std::hint::spin_loop()` for the last ~1ms of each tick interval. Achieves ±0.1ms accuracy; 6% CPU at 60Hz is negligible on modern CI runners.

2. **Nemesis Alpha — Local player only**: Remote nemesis entities keep their tick-based interpolation from Phase 2f. Alpha-based sub-tick lerp is not applied to nemesis, avoiding complexity and potential conflict with the jitter buffer.

## Follow-up (deviations from spec, recorded at implementation)

1. **"Headless" client still opens a window.** The spec's `--headless` mode reuses the GUI binary behind `#[macroquad::main]`, so it still needs a display (panics with `XOpenDisplay() failed!` without one). This was worked around in CI (`xvfb-run`), not fixed. A future phase should make the headless client truly window-less (a separate binary with a plain `fn main` and no macroquad), so it can run on bare CI runners without Xvfb.
2. **Box 2a scope grew to include the camera.** The plan's input-decoupling box originally covered only `is_key_pressed`/`is_key_down`. During implementation a *second* macroquad-context dependency surfaced — `GameCamera::update` calling `screen_width()`/`screen_height()` — which also panics without a window. The box was extended to inject the viewport via `GameCamera::set_viewport`, and the spec/plan wording was updated accordingly.