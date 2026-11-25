# Issue: Tick Rate Robustness - Fixed 60Hz Approximations
- **Status**: open
- **Type**: tech-debt
- **Severity**: minor
- **Created**: 2026-11-25
- **Closed**: 

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
- [ ] Analyze: Determine if this affects gameplay (prediction, reconciliation, interpolation)
- [ ] Design: Choose approach — fixed timestep with accumulator on client? Minimum frame rate enforcement?
- [ ] Server: Verify catch-up loop is sufficient or needs improvement
- [ ] Client GUI: Implement fixed timestep with accumulator (separate sim dt from render dt)
- [ ] Client Headless: Fix sleep duration to use sub-millisecond precision (park or spin)
- [ ] Tests: Add timing tests to verify tick consistency
- [ ] Verification: `cargo test`, `cargo clippy`, `cargo fmt`

## Verification
- [ ] cargo test
- [ ] cargo clippy
- [ ] cargo fmt

## Commits


## Lessons Learned