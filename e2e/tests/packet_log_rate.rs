// Box 14 (AC8): Packet log rate — PlayerInput sent at ~60Hz (16.666ms).
//
// With `RUSTVANIA_PACKET_LOG=1`, the client logs every packet as JSONL with a
// `ts_ms` field. The fixed-rate input send is a consequence of the accumulator:
// `update_network` runs once per `World::update` → once per sim tick → 60Hz.
//
// NOTE: `ts_ms` is millisecond-quantized, so each interval is an integer number
// of ms (16 or 17 at 60Hz). We assert the send *count* (robust) and that the
// average interval is close to 16.666ms (coarse). Precise sub-ms verification is
// covered by the headless timing test (Box 12) and code review (Box 7).

use e2e::run_headless_client;

/// Extract the leading integer `ts_ms` value from a JSONL log line.
fn extract_ts_ms(line: &str) -> Option<u64> {
    let key = "\"ts_ms\":";
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

#[test]
fn packet_log_playerinput_rate() {
    // A server is required so the client receives a JoinResponse and gets an id;
    // `update_network` only sends PlayerInput once an id is assigned. The shared
    // helper spawns the server (which is killed on teardown) and scopes the log
    // flag to the client child only (avoids a process-wide `unsafe { set_var }`
    // and keeps the spawned server from flooding CI).
    let duration_secs = 3u64;
    let (stdout, _) = run_headless_client(duration_secs, /*log_packets=*/ true);

    let mut timestamps: Vec<u64> = Vec::new();
    for line in stdout.lines() {
        if line.contains("\"dir\":\"sent\"")
            && line.contains("\"type\":\"PlayerInput\"")
            && let Some(ts) = extract_ts_ms(line)
        {
            timestamps.push(ts);
        }
    }

    // ~duration_secs * TICK_HZ (60) sends expected, minus the join round-trip lag.
    // The lower bound is generous: the client only starts sending PlayerInput
    // once the server assigns an id, and a slow JoinResponse under CI CPU
    // contention can cost several ticks. The upper bound guards against
    // over-sending (more than one PlayerInput per tick), which never happens.
    let expected = duration_secs * 60;
    assert!(
        timestamps.len() >= (expected - 20) as usize
            && timestamps.len() <= (expected + 10) as usize,
        "expected ~{} PlayerInput sends over {}s, got {}",
        expected,
        duration_secs,
        timestamps.len()
    );

    if timestamps.len() >= 2 {
        let intervals: Vec<u64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
        let avg = intervals.iter().sum::<u64>() as f64 / intervals.len() as f64;
        // 60Hz → ~16.666ms. Allow ±5ms for ms-quantization, OS jitter, and CI
        // CPU contention (a stalled tick is compensated by catch-up, but a
        // backlog at loop exit can nudge the average). Still catches gross rate
        // regressions: 30Hz (~33ms) and 120Hz (~8.3ms) are both far outside ±5ms.
        assert!(
            (avg - 16.666).abs() < 5.0,
            "average PlayerInput interval {}ms, expected ~16.666ms (60Hz)",
            avg
        );
    }
}
