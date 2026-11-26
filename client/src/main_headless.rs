use game::config::{HEADLESS_BUSY_WAIT_MS, TICK_HZ};
use game::state_machine::{AppEvent, AppState};
use metroidvania_rust::app::App;
use metroidvania_rust::make_transport_factory;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse --duration=N (seconds)
    let duration_secs = args
        .iter()
        .find(|a| a.starts_with("--duration="))
        .and_then(|a| a.split('=').nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0); // 0 = run until quit/SIGINT

    // Parse --server=ADDR
    let server_addr = args
        .iter()
        .find(|a| a.starts_with("--server="))
        .and_then(|a| a.split('=').nth(1))
        .unwrap_or("127.0.0.1:12345")
        .parse::<SocketAddr>()
        .unwrap();

    let factory = make_transport_factory(server_addr, true);
    let mut app = App::new(factory, true);

    run_headless(&mut app, duration_secs);
}

/// Headless driver: reuse the same `App` + handshake as the GUI, never mapping
/// pause (so it never sits in `Paused`), and quit on `Disconnected` or when the
/// `--duration` elapses. Preserves the hybrid sleep + busy-wait pacing.
fn run_headless(app: &mut App, duration_secs: u64) {
    let tick_interval = Duration::from_secs_f64(1.0 / TICK_HZ as f64);
    let max_ticks = if duration_secs > 0 {
        (duration_secs as f32 * TICK_HZ as f32) as u64
    } else {
        u64::MAX
    };

    println!(
        "Headless client running for {} seconds ({} ticks)",
        duration_secs, max_ticks
    );

    // Fixed timestep with no drift: advance `next_tick` by a constant interval
    // each iteration, then sleep for the bulk of the interval and busy-wait the
    // last ~HEADLESS_BUSY_WAIT_MS for sub-millisecond precision (spec D2).
    let mut next_tick = Instant::now() + tick_interval;
    let mut tick = 0u64;

    loop {
        let now = Instant::now();
        app.update(now);

        if app.is_quitting() {
            break;
        }
        if app.state() == AppState::Disconnected {
            app.dispatch(AppEvent::Quit, now);
            break;
        }

        tick += 1;
        if duration_secs > 0 && tick >= max_ticks {
            break;
        }

        let now = Instant::now();
        if now < next_tick {
            let remaining = next_tick - now;
            // Sleep for most of the interval, then busy-wait the remainder.
            if remaining > Duration::from_millis(HEADLESS_BUSY_WAIT_MS) {
                std::thread::sleep(remaining - Duration::from_millis(HEADLESS_BUSY_WAIT_MS));
            }
            while Instant::now() < next_tick {
                std::hint::spin_loop();
            }
        }
        next_tick += tick_interval;
    }

    println!("Headless client finished after {} ticks", max_ticks);
}
