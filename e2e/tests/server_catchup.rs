// Box 15 (AC7): Server catch-up under load.
//
// The server tick loop (server/src/main.rs) runs `while now >= next_tick { ...;
// next_tick += tick_interval; }` so that when the event drain takes too long, it
// catches up by running one authoritative tick per missed interval, then sleeps
// the remainder — no busy-spin, no drift. That loop lives in the binary, so it is
// verified by code review (Box 7).
//
// What we CAN test deterministically is the invariant the catch-up loop relies on:
// `ServerWorld::update` advances `current_tick` exactly once per call, and a burst
// of queued inputs is fully processed across a catch-up (no dropped inputs).

use e2e::harness::TestHarness;

#[test]
fn server_tick_counter_advances_once_per_update() {
    let mut h = TestHarness::new();
    let _p = h.connect_client();

    h.tick_n(10);

    // Each `tick()` runs exactly one `ServerWorld::update`, advancing current_tick by 1.
    assert_eq!(h.server.current_tick(), 10);
}

#[test]
fn server_catchup_processes_all_queued_inputs() {
    let mut h = TestHarness::new();
    let p = h.connect_client();

    // Queue 30 frames of "hold right" input, then run 30 ticks back-to-back to
    // simulate the server draining a backlog (catch-up). Every tick must consume
    // one frame of input; none may be dropped.
    h.hold_right(p, 30);
    h.tick_n(30);

    assert_eq!(h.server.current_tick(), 30);

    // The player advanced: the queued inputs were applied, not lost.
    let (x, _) = h.server_player_pos(p);
    assert!(x > 100.0, "server player should have advanced after catch-up, x={}", x);
}

#[test]
fn server_current_tick_is_monotonic_under_rapid_ticks() {
    let mut h = TestHarness::new();
    let _p = h.connect_client();

    let mut last = 0u64;
    for _ in 0..100 {
        h.tick();
        let now = h.server.current_tick();
        assert_eq!(now, last + 1, "current_tick must advance by exactly 1 per update");
        last = now;
    }
    assert_eq!(h.server.current_tick(), 100);
}
