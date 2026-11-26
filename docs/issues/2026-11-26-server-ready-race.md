# Issue: Fixed-sleep server-ready wait races with `cargo run` build in e2e tests
- **Status**: open
- **Type**: test
- **Severity**: minor
- **Created**: 2026-11-26
- **Closed**: —

## Description

`e2e/src/process.rs` spawns the live server for the subprocess e2e tests with a
fixed readiness delay:

```rust
pub fn spawn() -> ServerGuard {
    let child = Command::new("cargo")
        .args(["run", "-p", "metroidvania-server"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start server");
    std::thread::sleep(Duration::from_millis(SERVER_READY_MS)); // 500ms
    ServerGuard { child }
}
```

`run_headless_client` then starts the client immediately after this 500ms sleep.
On a cold CI checkout, the first `cargo run -p metroidvania-server` can trigger a
*build* of the server binary (the earlier `cargo clippy --all-features` step
produces metadata, not the linked executable; `cargo test` builds test targets,
not this binary), and that build can outlast the fixed 500ms. The client then
starts and sends its `JoinRequest` before the server is listening, so the join
handshake is delayed or retried for longer than the tests assume.

This is a latent race that can surface as flaky timing assertions in
`e2e/tests/headless_timing.rs` and `e2e/tests/packet_log_rate.rs` (which already
had their tolerances widened in `3b549c5` / `c0e67d2` to mask the symptom).

## Root Cause

- Readiness is inferred from a fixed wall-clock sleep, not an actual signal that
  the server has bound its socket and started ticking.
- The sleep (`SERVER_READY_MS = 500`) is shorter than a cold `cargo run` build.
- `cargo run` inside a test re-invokes the build system, so the first spawn in a
  fresh CI job pays a variable compile/link cost that the fixed delay does not
  absorb.

## Impact (if not fixed)

- Intermittent CI failures in the subprocess e2e tests (client joins a server
  that is not yet up), independent of the timing tolerances already widened.
- The tests remain timing-sensitive and can still flake under CI load.

## Fix Plan

- [ ] Pre-build the `metroidvania-server` and `rustvania-headless` binaries in CI
      (`cargo build -p metroidvania-server -p metroidvania-rust --bin rustvania-headless
      --features headless`) before `cargo test --workspace`, so `cargo run` inside
      the tests only spawns (no build).
- [ ] Replace the fixed `SERVER_READY_MS` sleep with a readiness poll: wait until
      the server reports it is listening (a "ready" log line on stdout, or a
      bounded retry loop that attempts a UDP probe) instead of a blind sleep.
- [ ] Alternatively, run the built binaries directly (`target/debug/<bin>`) rather
      than via `cargo run`, removing the build step from the test path entirely.
- [ ] Tests to add/update: none (behavioral); re-run the timing tests under load
      to confirm the race is gone.
- [ ] Verification: `cargo test --workspace`, `cargo clippy --all-targets
      --all-features --workspace -- -D warnings`, `cargo fmt --check --all`.

## Verification

- [ ] cargo test --workspace
- [ ] cargo clippy
- [ ] cargo fmt

## Commits

- (none yet)

## Lessons Learned

- A fixed `sleep(N)` is not a readiness signal; a spawned subprocess that needs a
  build cannot be assumed ready after a constant wall-clock delay.
- `cargo run` inside a test re-enters the build system and is inherently
  slow/variable on a cold checkout — prefer running a pre-built binary or probing
  for readiness.
