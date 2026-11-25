use game::state_machine::{AppEvent, AppState};
use macroquad::prelude::*;
use metroidvania_rust::app::App;
use metroidvania_rust::make_transport_factory;
use std::net::SocketAddr;
use std::time::Instant;

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

#[macroquad::main("Rustvania")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse --server=ADDR
    let server_addr = args
        .iter()
        .find(|a| a.starts_with("--server="))
        .and_then(|a| a.split('=').nth(1))
        .unwrap_or("127.0.0.1:12345")
        .parse::<SocketAddr>()
        .unwrap();

    let factory = make_transport_factory(server_addr, false);
    let mut app = App::new(factory, false);

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
