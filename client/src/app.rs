//! Client application runner: owns the flat state machine, the socket, the
//! `World`, per-frame input, and per-state drawing.
//!
//! This module is the "brain" between the pure `game::state_machine::transition`
//! core and the macroquad binary. `dispatch`/`update`/`set_game_input`/
//! `set_viewport` are all macroquad-free (only `draw` touches the renderer), so
//! the lifecycle, handshake and watchdog logic is unit- and e2e-testable without
//! a window.

use crate::network::handshake::{HandshakeResult, poll_join_response};
use crate::network::packet_io::PacketIO;
use crate::network::world::{recv_and_process, send_join_req};
use crate::world::game::World;
use game::config::{CONNECT_TIMEOUT_MS, DISCONNECT_TIMEOUT_MS, MAX_ACCUMULATED_FRAMES, TICK_HZ};
use game::fixed_timestep::FixedTimestep;
use game::maps::Map;
use game::state_machine::{AppEvent, AppState, transition};
use macroquad::prelude::*;
use protocol::GamePacket;
use std::time::{Duration, Instant};

/// Produces a fresh `Box<dyn PacketIO>` for one join attempt. Production binds a
/// laminar socket on `0.0.0.0:0` and spawns the poller; tests return a loopback
/// transport. Called once per `StartJoin`/`Retry` so each join uses a new socket.
pub type TransportFactory = Box<dyn Fn() -> Box<dyn PacketIO>>;

/// The client application runner.
pub struct App {
    state: AppState,
    /// Held standalone only in `Connecting`; moved into `World::new` on `Joined`.
    io: Option<Box<dyn PacketIO>>,
    /// Held only in `Playing`/`Paused`; owns the socket via `World.network`.
    world: Option<World>,
    /// The server-assigned player id (set on `Joined`, cleared on leave/drop).
    id: Option<u8>,
    /// Fixed-timestep accumulator, advanced only in `Playing`.
    timestep: FixedTimestep,
    /// `now + CONNECT_TIMEOUT_MS`, set on entering `Connecting`.
    connect_deadline: Option<Instant>,
    /// `now + DISCONNECT_TIMEOUT_MS`, set/reset by the disconnect watchdog.
    last_player_pos_deadline: Option<Instant>,
    factory: TransportFactory,
    /// Wall-clock of the previous frame, for `frame_dt` in the accumulator.
    last_time: Instant,
}

impl App {
    /// `factory` creates a socket per join; `headless` skips the menu and
    /// auto-dispatches `StartJoin` so the caller never sits in `MainMenu`.
    pub fn new(factory: TransportFactory, headless: bool) -> Self {
        let mut app = Self {
            state: AppState::MainMenu,
            io: None,
            world: None,
            id: None,
            timestep: FixedTimestep::new(TICK_HZ, MAX_ACCUMULATED_FRAMES),
            connect_deadline: None,
            last_player_pos_deadline: None,
            factory,
            last_time: Instant::now(),
        };
        if headless {
            app.dispatch(AppEvent::StartJoin, Instant::now());
        }
        app
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn is_quitting(&self) -> bool {
        self.state == AppState::Quitting
    }

    /// Apply an event and run its enter/exit side effects (bind, `JoinRequest`,
    /// `Leave`, `World` drop, deadline init). `now` is needed to set timeouts.
    pub fn dispatch(&mut self, event: AppEvent, now: Instant) {
        let next = transition(self.state, event);
        if next == self.state {
            // No-op: no side effects, state unchanged.
            return;
        }

        match (self.state, event) {
            (AppState::MainMenu, AppEvent::StartJoin) => self.begin_join(now),
            (AppState::Disconnected, AppEvent::Retry) => self.begin_join(now),
            (AppState::Connecting, AppEvent::Joined) => self.enter_playing_from_join(now),
            (AppState::Connecting, AppEvent::Cancel) => self.drop_without_leave(),
            (AppState::Connecting, AppEvent::ConnectFailed) => self.drop_without_leave(),
            (AppState::Playing, AppEvent::PauseRequested) => self.enter_paused(now),
            (AppState::Playing, AppEvent::QuitToMenu) => self.send_leave_and_drop(),
            (AppState::Playing, AppEvent::Quit) => self.send_leave_and_drop(),
            (AppState::Playing, AppEvent::ConnectionLost) => self.drop_without_leave(),
            (AppState::Paused, AppEvent::ResumeRequested) => self.resume_playing(now),
            (AppState::Paused, AppEvent::QuitToMenu) => self.send_leave_and_drop(),
            (AppState::Paused, AppEvent::Quit) => self.send_leave_and_drop(),
            (AppState::Paused, AppEvent::ConnectionLost) => self.drop_without_leave(),
            // MainMenu→Quit, Disconnected→BackToMenu/Quit: no side effects.
            _ => {}
        }

        self.state = next;
    }

    /// Inject per-frame gameplay input (sampled by the binary main; forwarded to
    /// the live `World` so `World::update` stays macroquad-free).
    pub fn set_game_input(&mut self, left: bool, right: bool, jump: bool, shoot: bool) {
        if let Some(world) = &mut self.world {
            world.move_left = left;
            world.move_right = right;
            world.pressed_jump = jump;
            world.pressed_shoot = shoot;
        }
    }

    /// Forward the render viewport to the live `World`'s camera. No-op unless in
    /// `Playing`/`Paused` (no `World` yet in menu/connecting/disconnected).
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if let Some(world) = &mut self.world {
            world.camera.set_viewport(width, height);
        }
    }

    /// Per-frame update: handshake polling, watchdog, accumulator (Playing only).
    pub fn update(&mut self, now: Instant) {
        match self.state {
            AppState::Connecting => self.update_connecting(now),
            AppState::Playing => self.update_playing(now),
            AppState::Paused => self.update_paused(now),
            // MainMenu, Disconnected, Quitting: nothing to advance.
            _ => {}
        }
    }

    /// Per-state draw (menu / connecting / world / paused overlay / disconnected).
    pub fn draw(&self) {
        match self.state {
            AppState::MainMenu => {
                clear_background(BLACK);
                draw_text("Rustvania", 100.0, 120.0, 60.0, WHITE);
                draw_text("Press ENTER to start", 100.0, 220.0, 30.0, WHITE);
                draw_text("Press Q to quit", 100.0, 260.0, 30.0, WHITE);
            }
            AppState::Connecting => {
                clear_background(BLACK);
                draw_text("Connecting...", 100.0, 120.0, 40.0, WHITE);
            }
            AppState::Playing => {
                clear_background(BLACK);
                if let Some(world) = &self.world {
                    world.draw(self.timestep.alpha());
                }
            }
            AppState::Paused => {
                clear_background(BLACK);
                if let Some(world) = &self.world {
                    world.draw(self.timestep.alpha());
                }
                draw_text("PAUSED", 100.0, 120.0, 40.0, YELLOW);
                draw_text("Press P to resume", 100.0, 170.0, 25.0, WHITE);
            }
            AppState::Disconnected => {
                clear_background(BLACK);
                draw_text("Disconnected", 100.0, 120.0, 40.0, RED);
                draw_text("Press ENTER to retry", 100.0, 180.0, 25.0, WHITE);
                draw_text("Press ESC for menu", 100.0, 215.0, 25.0, WHITE);
                draw_text("Press Q to quit", 100.0, 250.0, 25.0, WHITE);
            }
            AppState::Quitting => {}
        }
    }

    // -- Connecting state: poll the handshake and apply the connect timeout. --

    fn update_connecting(&mut self, now: Instant) {
        let result = match &mut self.io {
            Some(io) => poll_join_response(io.as_mut()),
            None => HandshakeResult::Pending,
        };

        match result {
            HandshakeResult::Joined { assigned_id } => {
                // Stash the id; `dispatch(Joined)` builds the `World` exactly once.
                self.id = Some(assigned_id);
                self.dispatch(AppEvent::Joined, now);
            }
            HandshakeResult::Pending => {
                let timed_out = self.connect_deadline.is_some_and(|d| now >= d);
                if timed_out {
                    self.dispatch(AppEvent::ConnectFailed, now);
                }
            }
        }
    }

    // -- Playing state: fixed-timestep accumulator + disconnect watchdog. --

    fn update_playing(&mut self, now: Instant) {
        let frame_dt = now.saturating_duration_since(self.last_time).as_secs_f64();
        self.last_time = now;

        let steps = self.timestep.advance(frame_dt);
        let tick_dt = 1.0 / TICK_HZ as f32;
        for _ in 0..steps {
            if let Some(world) = &mut self.world {
                world.update(tick_dt);
            }
        }

        self.run_watchdog(now);
    }

    // -- Paused state: receive/process only (no stepping, no send) + watchdog. --

    fn update_paused(&mut self, now: Instant) {
        if let Some(world) = &mut self.world {
            recv_and_process(world);
        }
        self.run_watchdog(now);
    }

    /// Read/clear the liveness flag, reset the deadline on any `PlayerPos`, and
    /// fire `ConnectionLost` when the deadline passes with no traffic.
    fn run_watchdog(&mut self, now: Instant) {
        let received = match &mut self.world {
            Some(world) => {
                let r = world.received_player_pos;
                world.received_player_pos = false;
                r
            }
            None => false,
        };

        if received {
            self.last_player_pos_deadline =
                Some(now + Duration::from_millis(DISCONNECT_TIMEOUT_MS));
        }

        let timed_out = self.last_player_pos_deadline.is_some_and(|d| now >= d);
        if timed_out {
            self.dispatch(AppEvent::ConnectionLost, now);
        }
    }

    // -- Lifecycle side effects (enter/exit handlers). --

    /// Bind a fresh socket, send `JoinRequest`, and arm the connect timeout.
    fn begin_join(&mut self, now: Instant) {
        let mut io = (self.factory)();
        send_join_req(&mut *io);
        self.io = Some(io);
        self.connect_deadline = Some(now + Duration::from_millis(CONNECT_TIMEOUT_MS));
    }

    /// `Joined`: build the `World` once from the standalone socket, stamp the
    /// assigned id, and start the disconnect-watchdog grace period.
    fn enter_playing_from_join(&mut self, now: Instant) {
        let assigned_id = self
            .id
            .expect("assigned id must be stashed before dispatching Joined");
        let io = self
            .io
            .take()
            .expect("a standalone socket must exist while Connecting");
        let mut world = World::new(Map::new(), io);
        world.id = Some(assigned_id);
        world.player.id = assigned_id;
        println!("Connected! My ID is {}", assigned_id);
        self.world = Some(world);
        self.last_player_pos_deadline = Some(now + Duration::from_millis(DISCONNECT_TIMEOUT_MS));
        self.last_time = now;
    }

    /// `ResumeRequested`: restart the accumulator clock and watchdog grace period
    /// (the `World` already exists).
    fn resume_playing(&mut self, now: Instant) {
        self.last_player_pos_deadline = Some(now + Duration::from_millis(DISCONNECT_TIMEOUT_MS));
        self.last_time = now;
    }

    /// `PauseRequested`: freeze the sim; restart the watchdog grace period.
    fn enter_paused(&mut self, now: Instant) {
        self.last_player_pos_deadline = Some(now + Duration::from_millis(DISCONNECT_TIMEOUT_MS));
        self.last_time = now;
    }

    /// `Quit`/`QuitToMenu` from `Playing`/`Paused`: send one `Leave { id }` (iff
    /// an id is assigned), then drop the `World` (and its socket).
    fn send_leave_and_drop(&mut self) {
        if let Some(world) = &mut self.world
            && let Some(id) = self.id
        {
            world.network.send(GamePacket::Leave { id });
        }
        self.world = None;
        self.io = None;
        self.id = None;
        self.last_player_pos_deadline = None;
    }

    /// `Cancel`/`ConnectFailed`/`ConnectionLost`: drop the socket and `World`
    /// without sending `Leave` (the link is already gone or no id was assigned).
    fn drop_without_leave(&mut self) {
        self.world = None;
        self.io = None;
        self.id = None;
        self.last_player_pos_deadline = None;
    }
}
