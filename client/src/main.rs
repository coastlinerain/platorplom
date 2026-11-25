use game::config::{HEADLESS_BUSY_WAIT_MS, TICK_HZ};
use game::state_machine::{AppEvent, AppState};
use macroquad::prelude::*;
use metroidvania_rust::app::{App, TransportFactory};
use metroidvania_rust::network::packet_io::PacketIO;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// RealPacketIO wraps the laminar socket for the GUI path.
struct RealPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for RealPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<protocol::GamePacket>(p.payload()) {
                    metroidvania_rust::network::logging::log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
        metroidvania_rust::network::logging::log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            protocol::GamePacket::JoinRequest | protocol::GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

/// HeadlessPacketIO for running without rendering (CI/testing). Structurally
/// identical to `RealPacketIO`; kept separate so the headless factory closure
/// and the GUI factory closure each reference a concrete type.
struct HeadlessPacketIO {
    sender: crossbeam_channel::Sender<laminar::Packet>,
    receiver: crossbeam_channel::Receiver<laminar::SocketEvent>,
    server_addr: SocketAddr,
}

impl PacketIO for HeadlessPacketIO {
    fn try_recv(&mut self) -> Option<protocol::GamePacket> {
        match self.receiver.try_recv() {
            Ok(laminar::SocketEvent::Packet(p)) => {
                if let Ok(decoded) = postcard::from_bytes::<protocol::GamePacket>(p.payload()) {
                    metroidvania_rust::network::logging::log_packet("recv", &decoded);
                    Some(decoded)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn send(&mut self, packet: protocol::GamePacket) {
        metroidvania_rust::network::logging::log_packet("sent", &packet);
        let bytes = postcard::to_allocvec(&packet).unwrap();
        let pkt = match &packet {
            protocol::GamePacket::JoinRequest | protocol::GamePacket::Hit { .. } => {
                laminar::Packet::reliable_ordered(self.server_addr, bytes, None)
            }
            _ => laminar::Packet::unreliable(self.server_addr, bytes),
        };
        let _ = self.sender.send(pkt);
    }
}

/// Build a transport factory that binds a fresh ephemeral socket per join. The
/// ephemeral-port bind is effectively infallible, so `.unwrap()` is an accepted,
/// documented exception (see plan notes).
fn make_transport_factory(server_addr: SocketAddr, headless: bool) -> TransportFactory {
    Box::new(move || {
        let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
        let sender = socket.get_packet_sender();
        let receiver = socket.get_event_receiver();
        std::thread::spawn(move || socket.start_polling());

        if headless {
            Box::new(HeadlessPacketIO {
                sender,
                receiver,
                server_addr,
            }) as Box<dyn PacketIO>
        } else {
            Box::new(RealPacketIO {
                sender,
                receiver,
                server_addr,
            }) as Box<dyn PacketIO>
        }
    })
}

/// Map UI keys to `AppEvent`s based on the current state (menu navigation,
/// cancel, pause/resume, quit). Returns `None` when no UI event this frame.
fn sample_ui_event(state: AppState) -> Option<AppEvent> {
    match state {
        AppState::MainMenu => {
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
                Some(AppEvent::StartJoin)
            } else if is_key_pressed(KeyCode::Q) {
                Some(AppEvent::Quit)
            } else {
                None
            }
        }
        AppState::Connecting => {
            if is_key_pressed(KeyCode::Escape) {
                Some(AppEvent::Cancel)
            } else {
                None
            }
        }
        AppState::Playing => {
            if is_key_pressed(KeyCode::P) {
                Some(AppEvent::PauseRequested)
            } else if is_key_pressed(KeyCode::Escape) {
                Some(AppEvent::QuitToMenu)
            } else if is_key_pressed(KeyCode::Q) {
                Some(AppEvent::Quit)
            } else {
                None
            }
        }
        AppState::Paused => {
            if is_key_pressed(KeyCode::P) {
                Some(AppEvent::ResumeRequested)
            } else if is_key_pressed(KeyCode::Escape) {
                Some(AppEvent::QuitToMenu)
            } else if is_key_pressed(KeyCode::Q) {
                Some(AppEvent::Quit)
            } else {
                None
            }
        }
        AppState::Disconnected => {
            if is_key_pressed(KeyCode::Enter) {
                Some(AppEvent::Retry)
            } else if is_key_pressed(KeyCode::Escape) {
                Some(AppEvent::BackToMenu)
            } else if is_key_pressed(KeyCode::Q) {
                Some(AppEvent::Quit)
            } else {
                None
            }
        }
        AppState::Quitting => None,
    }
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

#[macroquad::main("Rustvania")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");

    // Parse --duration=N (seconds)
    let duration_secs = args
        .iter()
        .find(|a| a.starts_with("--duration="))
        .and_then(|a| a.split('=').nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0); // 0 = run until the app quits (headless: run until SIGINT)

    // Parse --server=ADDR
    let server_addr = args
        .iter()
        .find(|a| a.starts_with("--server="))
        .and_then(|a| a.split('=').nth(1))
        .unwrap_or("127.0.0.1:12345")
        .parse::<SocketAddr>()
        .unwrap();

    let factory = make_transport_factory(server_addr, headless);
    let mut app = App::new(factory, headless);

    if headless {
        run_headless(&mut app, duration_secs);
        return;
    }

    loop {
        let now = Instant::now();

        // UI keys → AppEvent (menu navigation, cancel, pause/resume, quit).
        if let Some(event) = sample_ui_event(app.state()) {
            app.dispatch(event, now);
        }

        // Gameplay input, sampled once per render frame and injected into the
        // live `World` (matches the Phase 2g input-boundary precedent).
        let left = is_key_down(KeyCode::Left);
        let right = is_key_down(KeyCode::Right);
        let jump = is_key_pressed(KeyCode::Space);
        let shoot = is_key_pressed(KeyCode::Z);
        app.set_game_input(left, right, jump, shoot);

        // Inject the viewport once per render frame BEFORE drawing.
        app.set_viewport(screen_width(), screen_height());

        app.update(now);
        app.draw();

        if app.is_quitting() {
            break;
        }

        next_frame().await;
    }
}
