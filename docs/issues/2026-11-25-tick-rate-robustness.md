# Issue: Tick Rate Robustness - Fixed 60Hz Approximations
- **Status**: closed
- **Type**: tech-debt
- **Severity**: minor
- **Created**: 2026-11-25
- **Closed**: 2026-11-25 (resolved by Phase 2g — `docs/specs/phase-2g-fixed-timestep.md`)

## Description
Both the server and client use a fixed 60Hz tick rate (`TICK_HZ = 60` in `game/src/config.rs`), but the current implementation relies on approximations:

1. **Server** (`server/src/main.rs`): Uses `Instant::now()` + `thread::sleep` with a `while now >= next_tick` catch-up loop. The sleep resolution is ~1ms, and the catch-up loop handles drift but doesn't guarantee precise 60Hz.

2. **Client GUI** (`client/src/main.rs`): Runs at whatever frame rate `macroquad` provides via `get_frame_time()`. This is typically ~60Hz but varies with vsync, GPU load, and platform. The `dt` passed to `World::update()` is the actual frame time, not a fixed step.

3. **Client Headless** (`client/src/main.rs:120`): Uses `thread::sleep(Duration::from_millis(1000 / TICK_HZ as u64))` which at 60Hz = 16ms (integer division truncates 16.666... to 16). This runs at ~62.5Hz, not 60Hz.

The user wants: "run a loop of at least 30 iterations on a client always, and the same for every client" — suggesting a minimum frame rate guarantee and consistent timing across all clients.

## Root Cause
- Fixed timestep not properly implemented on client side (GUI uses variable dt)
- Headless client uses integer division for sleep duration (16ms instead of 16.67ms)
- No accumulator pattern to decouple simulation from render/network rates
- Clock sync (Phase 2f) assumes server ticks are regular but client ticks may not be

## Fix Plan
- [x] Analyze: Determine if this affects gameplay (prediction, reconciliation, interpolation)
- [x] Design: Choose approach — fixed timestep with accumulator on client (Phase 2g spec)
- [x] Server: Verify catch-up loop is sufficient (spec D3 — correct, documented)
- [x] Client GUI: Implement fixed timestep with accumulator (separate sim dt from render dt)
- [x] Client Headless: Fix sleep duration to use sub-millisecond precision (hybrid sleep + `spin_loop`)
- [x] Tests: Add timing tests to verify tick consistency (headless_timing, packet_log_rate, gui_accumulator)
- [x] Verification: `cargo test`, `cargo clippy`, `cargo fmt`

## Verification
- [x] cargo test
- [x] cargo clippy
- [x] cargo fmt

## Commits
`feat/phase-2g-fixed-timestep` (8 commits): RED tests → plan fixes → accumulator/camera/headless implementation → subprocess-test fixes → CI Xvfb → docs → teleport-lerp fix → skills note.

## Lessons Learned
- The accumulator's `MAX_ACCUMULATED_FRAMES` clamp is a spiral-of-death *safety* bound, not a render-floor guarantee (full 60Hz sim is sustained down to ~12 FPS render).
- The "headless" client still opens a macroquad window (it is the GUI binary behind `#[macroquad::main]`), so CI needs Xvfb — a future phase should make it truly window-less.
- See `docs/lessons.md` (Phase 2g section) and `docs/progress.md` for the full record.