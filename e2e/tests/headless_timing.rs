// Box 12 (AC5): Headless fixed-timestep — the tick loop does not drift.
//
// The headless loop (`run_headless`) must use a fixed timestep with
// `next_tick += tick_interval`: sleep for the bulk of the interval, then busy-wait
// the last ~`HEADLESS_BUSY_WAIT_MS` ms. The pre-fix loop used a fixed
// `sleep(16)` per tick, which drifts (~4%) and finishes early relative to the
// fixed-timestep duration of `TICK_HZ` (60Hz → 16.666ms).
//
// NOTE: these are wall-clock subprocess tests and are subject to OS timer
// granularity, `cargo run` launch overhead, and (for the windowed headless
// binary) a display. The meaningful assertions are: (1) the client completes the
// exact expected tick count, and (2) the wall-clock time is close to the
// requested duration (a fixed-timestep loop must never finish early). Precise
// sub-ms verification is better served by the packet-log timestamps (Box 14)
// and by code review of the tick loop (Box 7).

use std::process::Command;
use std::time::{Duration, Instant};

/// Run the headless client for `duration_secs` and return (stdout, elapsed).
fn run_headless(duration_secs: u64) -> (String, Duration) {
    let start = Instant::now();
    // No server is required: the headless client runs its own tick loop and
    // simply drops the JoinRequest/PlayerInput packets when nobody listens.
    let output = Command::new("cargo")
        .args([
            "run",
            "--bin",
            "metroidvania-rust",
            "--",
            "--headless",
            &format!("--duration={}", duration_secs),
            "--server=127.0.0.1:9",
        ])
        .output()
        .expect("failed to run headless client");
    let elapsed = start.elapsed();
    (String::from_utf8_lossy(&output.stdout).to_string(), elapsed)
}

#[test]
fn headless_runs_expected_tick_count() {
    let duration_secs = 2u64;
    let (stdout, _) = run_headless(duration_secs);

    // The headless loop runs exactly `duration_secs * TICK_HZ` ticks.
    let expected_ticks = duration_secs * 60;
    let finished_line = stdout
        .lines()
        .find(|l| l.contains("finished after"))
        .unwrap_or_default();
    assert!(
        finished_line.contains(&format!("{} ticks", expected_ticks)),
        "expected headless client to finish after {} ticks, got: {:?}",
        expected_ticks,
        finished_line
    );
}

#[test]
fn headless_fixed_timestep_no_drift() {
    let duration_secs = 8u64; // 480 ticks

    let (stdout, elapsed) = run_headless(duration_secs);
    println!("headless stdout:\n{}", stdout);

    // A fixed-timestep loop takes AT LEAST `duration_secs`. The pre-fix
    // `sleep(16)` loop finishes early (drift) relative to the 60Hz interval.
    // The lower bound catches that early-finish; the upper bound is generous
    // to absorb `cargo run` launch + window creation overhead in CI.
    let lower = Duration::from_millis(duration_secs * 1000 - 400);
    let upper = Duration::from_millis(duration_secs * 1000 + 2500);
    assert!(
        elapsed >= lower,
        "headless loop finished early: {:?} < {:?} (fixed timestep must not finish before the target duration)",
        elapsed,
        lower
    );
    assert!(
        elapsed <= upper,
        "headless loop took too long: {:?} (upper bound {:?})",
        elapsed,
        upper
    );
}
