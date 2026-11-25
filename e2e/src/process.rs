//! Shared helpers to spawn the real server + headless client binaries for the
//! e2e tests that exercise the actual `--headless` subprocess path.
//!
//! The headless client now genuinely connects (game-state-machine G8), so these
//! tests must run against a live server. The server binary hardcodes
//! `127.0.0.1:12345` (out of scope to change), so tests that spawn it are
//! serialized on a global lock and the server is killed on teardown via a
//! `Drop` guard.

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Serializes tests that bind the server's hardcoded `127.0.0.1:12345` port.
/// UDP has no TIME_WAIT, so killing the server releases the port immediately;
/// the lock only needs to prevent two live servers from overlapping.
static SERVER_PORT_LOCK: Mutex<()> = Mutex::new(());

/// How long to wait for the freshly spawned server to bind and start ticking.
const SERVER_READY_MS: u64 = 500;

/// A live server child process, killed and reaped when dropped.
pub struct ServerGuard {
    child: Child,
}

impl ServerGuard {
    /// Spawn the server binary and wait for it to become ready. Its stdout and
    /// stderr are discarded so CI stays quiet.
    pub fn spawn() -> ServerGuard {
        let child = Command::new("cargo")
            .args(["run", "-p", "metroidvania-server"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");
        std::thread::sleep(Duration::from_millis(SERVER_READY_MS));
        ServerGuard { child }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run the headless client for `duration_secs` against a live server.
///
/// Serializes on the shared server port, spawns the server, then spawns the
/// client (`--headless --duration=N --server=127.0.0.1:12345`). When
/// `log_packets` is true the client's `RUSTVANIA_PACKET_LOG` is set (the server
/// is never given it, keeping CI output bounded). Returns the client's stdout
/// and the wall-clock elapsed of the client run.
pub fn run_headless_client(duration_secs: u64, log_packets: bool) -> (String, Duration) {
    let _guard = SERVER_PORT_LOCK.lock().expect("server port lock poisoned");
    let _server = ServerGuard::spawn();

    let start = Instant::now();
    let mut cmd = Command::new("cargo");
    cmd.args([
        "run",
        "-p",
        "metroidvania-rust",
        "--",
        "--headless",
        &format!("--duration={}", duration_secs),
        "--server=127.0.0.1:12345",
    ])
    .current_dir(env!("CARGO_MANIFEST_DIR"));
    if log_packets {
        cmd.env("RUSTVANIA_PACKET_LOG", "1");
    }
    let output = cmd.output().expect("failed to run headless client");
    let elapsed = start.elapsed();

    // `_server` drops here (kill + reap) before `_guard` releases the lock.
    (String::from_utf8_lossy(&output.stdout).to_string(), elapsed)
}
