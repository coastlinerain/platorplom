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

use std::process::Command;
use std::time::Duration;

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
    // SAFETY: `set_var` is `unsafe` in Rust 2024 (a concurrent read of the same
    // env var while another thread calls `set_var` is UB). This test is the only
    // writer in a single-threaded subprocess spawn; no other thread touches it.
    unsafe { std::env::set_var("RUSTVANIA_PACKET_LOG", "1") };

    // A server is required so the client receives a JoinResponse and gets an id;
    // `update_network` only sends PlayerInput once an id is assigned.
    let mut server = Command::new("cargo")
        .args(["run", "-p", "metroidvania-server"])
        .spawn()
        .expect("failed to start server");
    std::thread::sleep(Duration::from_millis(500));

    let duration_secs = 3u64;
    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "metroidvania-rust",
            "--",
            "--headless",
            &format!("--duration={}", duration_secs),
            "--server=127.0.0.1:12345",
        ])
        .output()
        .expect("failed to run headless client");

    let _ = server.kill();
    let _ = server.wait();

    let stdout = String::from_utf8_lossy(&output.stdout);
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
    let expected = duration_secs * 60;
    assert!(
        timestamps.len() >= (expected - 10) as usize
            && timestamps.len() <= (expected + 10) as usize,
        "expected ~{} PlayerInput sends over {}s, got {}",
        expected,
        duration_secs,
        timestamps.len()
    );

    if timestamps.len() >= 2 {
        let intervals: Vec<u64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
        let avg = intervals.iter().sum::<u64>() as f64 / intervals.len() as f64;
        // 60Hz → ~16.666ms. Allow ±2ms for ms-quantization and OS jitter.
        assert!(
            (avg - 16.666).abs() < 2.0,
            "average PlayerInput interval {}ms, expected ~16.666ms (60Hz)",
            avg
        );
    }
}
