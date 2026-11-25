// Box 13 (AC6, AC9): GUI accumulator at 30/60/144/10 FPS.
//
// Verifies the accumulator pattern: render `frame_dt` is accumulated, clamped to
// `MAX_ACCUMULATED_FRAMES` ticks, and one `World::update(TICK_DT)` runs per sim
// tick. `client_tick` advances once per sim tick (not per render frame).
//
// NOTE: `World::update` is macroquad-free since Box 2a (input is sampled once
// per render frame in main.rs and injected via the `move_left`/`move_right`/
// `pressed_*` fields), so these tests run headless.

use e2e::harness::TestHarness;
use game::config::{MAX_ACCUMULATED_FRAMES, TICK_HZ};
use game::fixed_timestep::FixedTimestep;
use metroidvania_rust::network::world::handle_packet;

const TICK_DT: f32 = 1.0 / TICK_HZ as f32;

/// Simulate one render frame of the accumulator loop. The accumulate → clamp →
/// step logic lives in `FixedTimestep` (the same implementation production's
/// `client/src/main.rs` uses), so `advance` returns how many fixed `update`
/// steps to run this frame.
fn simulate_frame(
    harness: &mut TestHarness,
    handle: e2e::ClientHandle,
    frame_dt: f64,
    timestep: &mut FixedTimestep,
) -> u32 {
    let steps = timestep.advance(frame_dt);
    for _ in 0..steps {
        harness.client_mut(handle).world.update(TICK_DT);
    }
    steps
}

/// Connect a client and flush the `JoinResponse` that `connect_client` queued
/// before the accumulator loop starts.
///
/// `connect_client` leaves the reply in the client's receive channel. If the
/// first `World::update` (inside `simulate_frame`) processed it, `client_tick`
/// would already have been incremented and the `JoinResponse` arm would reset
/// it to 0 (clock sync restarts on join), cancelling that first sim tick and
/// leaving `client_tick` one short of the number of `World::update` calls.
/// Draining the pending packets up front makes each sim tick count exactly once.
fn connect_joined_client(h: &mut TestHarness) -> e2e::ClientHandle {
    let handle = h.connect_client();
    // `connect_client` queues exactly one packet (the `JoinResponse`); drain
    // whatever is pending so the join is handled before the first sim tick.
    // Same shape as `TestHarness::tick` (step 7a).
    let world = &mut h.client_mut(handle).world;
    while let Some(packet) = world.network.try_recv() {
        handle_packet(world, packet);
    }
    assert!(
        h.client(handle).world.id.is_some(),
        "client must be assigned an id before simulating"
    );
    handle
}

#[test]
fn gui_accumulator_30_fps() {
    // 30 FPS → 2 sim steps per frame on average. 60 frames = 2.0s = ~120 steps.
    let mut h = TestHarness::new();
    let p = connect_joined_client(&mut h);
    let frame_dt = 1.0f64 / 30.0;
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);
    let mut total = 0u32;

    for _ in 0..60 {
        total += simulate_frame(&mut h, p, frame_dt, &mut timestep);
    }

    assert!(
        (total as i32 - 120).abs() <= 2,
        "30 FPS: expected ~120 sim steps over 2s, got {}",
        total
    );
    assert_eq!(h.client(p).world.client_tick, total as u64);
}

#[test]
fn gui_accumulator_60_fps() {
    // 60 FPS → exactly 1 sim step per frame (frame_dt == TICK_DT).
    let mut h = TestHarness::new();
    let p = connect_joined_client(&mut h);
    let frame_dt = 1.0f64 / 60.0;
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);
    let mut total = 0u32;

    for _ in 0..60 {
        total += simulate_frame(&mut h, p, frame_dt, &mut timestep);
    }

    assert_eq!(
        total, 60,
        "60 FPS should yield exactly 1 sim step per frame"
    );
    assert_eq!(h.client(p).world.client_tick, total as u64);
}

#[test]
fn gui_accumulator_144_fps() {
    // 144 FPS → ~0.417 sim steps per frame. 144 frames = 1.0s = ~60 steps.
    let mut h = TestHarness::new();
    let p = connect_joined_client(&mut h);
    let frame_dt = 1.0f64 / 144.0;
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);
    let mut total = 0u32;

    for _ in 0..144 {
        total += simulate_frame(&mut h, p, frame_dt, &mut timestep);
    }

    assert!(
        (total as i32 - 60).abs() <= 2,
        "144 FPS: expected ~60 sim steps over 1s, got {}",
        total
    );
    assert_eq!(h.client(p).world.client_tick, total as u64);
}

#[test]
fn gui_accumulator_10_fps_clamped() {
    // 10 FPS → 100ms per frame → clamped to 5 sim steps per frame.
    let mut h = TestHarness::new();
    let p = connect_joined_client(&mut h);
    let frame_dt = 1.0f64 / 10.0;
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);
    let mut total = 0u32;

    for _ in 0..10 {
        total += simulate_frame(&mut h, p, frame_dt, &mut timestep);
    }

    assert_eq!(
        total, 50,
        "10 FPS should yield 5 clamped sim steps per frame"
    );
    assert_eq!(h.client(p).world.client_tick, total as u64);
}

#[test]
fn gui_accumulator_spiral_of_death_protection() {
    // A pathological 1s frame must clamp to MAX_ACCUMULATED_FRAMES (5) steps,
    // not run 60.
    let mut h = TestHarness::new();
    let p = connect_joined_client(&mut h);
    let mut timestep = FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES);

    let steps = simulate_frame(&mut h, p, 1.0f64, &mut timestep);

    assert_eq!(steps, MAX_ACCUMULATED_FRAMES);
    assert_eq!(h.client(p).world.client_tick, MAX_ACCUMULATED_FRAMES as u64);
}
