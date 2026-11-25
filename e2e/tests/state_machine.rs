//! e2e RED tests for the client game state machine (spec TC4–TC6 / AC7–AC10).
//!
//! They drive `metroidvania_rust::app::App` in-process through a
//! `TransportFactory` that hands out `LoopbackTransport`s, with a `VirtualClock`
//! injected as `now` so handshake, watchdog and quit timing are deterministic
//! (no real socket, no real server).
//!
//! `metroidvania_rust::app` does not exist yet, so this test target fails to
//! compile — the intended RED state for plan Box 3.

use e2e::{LoopbackTransport, VirtualClock};
use game::config::{CONNECT_TIMEOUT_MS, DISCONNECT_TIMEOUT_MS};
use game::state_machine::{AppEvent, AppState};
use metroidvania_rust::app::{App, TransportFactory};
use protocol::GamePacket;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

/// Server → client senders, one per join attempt (the factory is called once per
/// `StartJoin`/`Retry`). The test keeps the senders to inject packets, while the
/// app owns the matching receiver inside each transport.
type InboundSenders = Rc<RefCell<Vec<mpsc::Sender<Vec<u8>>>>>;

/// A client under test plus the channels that drive and observe its transport.
struct TestApp {
    app: App,
    clock: VirtualClock,
    /// Everything the client sent (JoinRequest, PlayerInput, Leave, …).
    outbound: mpsc::Receiver<Vec<u8>>,
    /// Server → client senders, indexed by join attempt.
    inbound: InboundSenders,
}

/// Build an `App` whose transport factory produces in-memory loopback pairs.
fn make_test_app() -> TestApp {
    let (outbound_tx, outbound) = mpsc::channel::<Vec<u8>>();
    let inbound: InboundSenders = Rc::new(RefCell::new(Vec::new()));
    let senders = Rc::clone(&inbound);

    let factory: TransportFactory = Box::new(move || {
        let (inbound_tx, inbound_rx) = mpsc::channel::<Vec<u8>>();
        senders.borrow_mut().push(inbound_tx);
        Box::new(LoopbackTransport {
            rx: inbound_rx,
            tx: outbound_tx.clone(),
        })
    });

    TestApp {
        app: App::new(factory, /*headless=*/ false),
        clock: VirtualClock::new(),
        outbound,
        inbound,
    }
}

impl TestApp {
    /// Decode every packet the client has sent since the last drain.
    fn drain_sent(&self) -> Vec<GamePacket> {
        let mut packets = Vec::new();
        while let Ok(bytes) = self.outbound.try_recv() {
            packets.push(postcard::from_bytes::<GamePacket>(&bytes).expect("decode packet"));
        }
        packets
    }

    /// Deliver a server-originated packet to the client on its newest transport.
    fn inject(&self, packet: GamePacket) {
        let bytes = postcard::to_allocvec(&packet).expect("encode packet");
        let sender = self
            .inbound
            .borrow()
            .last()
            .cloned()
            .expect("client has not opened a transport yet");
        sender.send(bytes).expect("client transport was dropped");
    }

    /// `StartJoin` → `Connecting`, inject `JoinResponse` → `Playing`.
    fn reach_playing(&mut self, assigned_id: u8) {
        self.app.dispatch(AppEvent::StartJoin, self.clock.now());
        assert_eq!(self.app.state(), AppState::Connecting);

        self.inject(GamePacket::JoinResponse { assigned_id });
        self.app.update(self.clock.now());
        assert_eq!(self.app.state(), AppState::Playing);
    }

    /// One render frame: advance the virtual clock, then let the app run it.
    fn frame(&mut self, elapsed_ms: u64) {
        self.clock.advance(elapsed_ms);
        self.app.update(self.clock.now());
    }
}

/// TC4 / AC7: `StartJoin` sends exactly one `JoinRequest`; a `JoinResponse`
/// drives the app to `Playing`, and the next tick's `PlayerInput` carries the
/// assigned id.
///
/// `App` has no `world()` accessor, so the emitted `PlayerInput { id, .. }` is
/// the spec's documented observable of `world.id == assigned_id` (plan "Notes
/// for implementer & tester").
#[test]
fn handshake_join_reaches_playing_with_assigned_id() {
    let mut t = make_test_app();
    assert_eq!(t.app.state(), AppState::MainMenu);

    t.app.dispatch(AppEvent::StartJoin, t.clock.now());
    assert_eq!(t.app.state(), AppState::Connecting);

    let sent = t.drain_sent();
    let join_requests = sent
        .iter()
        .filter(|packet| matches!(packet, GamePacket::JoinRequest))
        .count();
    assert_eq!(
        join_requests, 1,
        "StartJoin must send exactly one JoinRequest, got {sent:?}"
    );

    t.inject(GamePacket::JoinResponse { assigned_id: 7 });
    t.app.update(t.clock.now());
    assert_eq!(t.app.state(), AppState::Playing);

    // A few render frames so the fixed-timestep accumulator emits >= 1 tick.
    let mut inputs = Vec::new();
    for _ in 0..3 {
        t.frame(20);
        inputs.extend(t.drain_sent());
    }
    assert_eq!(t.app.state(), AppState::Playing);

    let input_ids: Vec<u8> = inputs
        .iter()
        .filter_map(|packet| match packet {
            GamePacket::PlayerInput { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(
        !input_ids.is_empty(),
        "a Playing tick must emit PlayerInput, got {inputs:?}"
    );
    assert!(
        input_ids.iter().all(|id| *id == 7),
        "PlayerInput must carry the assigned id 7, got {input_ids:?}"
    );
}

/// TC4 / AC8: no `JoinResponse` before `CONNECT_TIMEOUT_MS` fires
/// `ConnectFailed` → `Disconnected`.
#[test]
fn connect_timeout_without_join_response_enters_disconnected() {
    let mut t = make_test_app();
    t.app.dispatch(AppEvent::StartJoin, t.clock.now());
    assert_eq!(t.app.state(), AppState::Connecting);

    // Just before the deadline the app must keep waiting.
    t.frame(CONNECT_TIMEOUT_MS - 1);
    assert_eq!(
        t.app.state(),
        AppState::Connecting,
        "the connect timeout must not fire before {CONNECT_TIMEOUT_MS} ms"
    );

    // Past the deadline it gives up.
    t.frame(2);
    assert_eq!(t.app.state(), AppState::Disconnected);
}

/// TC5 / AC9: after `Playing`, no `PlayerPos` for `DISCONNECT_TIMEOUT_MS` fires
/// `ConnectionLost` → `Disconnected`.
#[test]
fn disconnect_watchdog_enters_disconnected_without_player_pos() {
    let mut t = make_test_app();
    t.reach_playing(7);

    // The grace period from the join must still hold.
    t.frame(DISCONNECT_TIMEOUT_MS - 1);
    assert_eq!(
        t.app.state(),
        AppState::Playing,
        "the watchdog must not fire before {DISCONNECT_TIMEOUT_MS} ms"
    );

    t.frame(2);
    assert_eq!(t.app.state(), AppState::Disconnected);
}

/// TC5 / AC9: a `PlayerPos` inside the window resets the watchdog deadline, so
/// the app stays `Playing` past the *original* deadline.
#[test]
fn player_pos_resets_the_disconnect_watchdog() {
    let mut t = make_test_app();
    t.reach_playing(7);

    // t = 800 ms: still alive, no packet yet.
    t.frame(800);
    assert_eq!(t.app.state(), AppState::Playing);

    // A PlayerPos arrives and is drained by the next sim tick (t = 820 ms), so
    // the deadline moves to t = 1820 ms.
    t.inject(GamePacket::PlayerPos {
        id: 7,
        x: 100.0,
        y: 100.0,
        dir: 1.0,
        level_x: 0,
        level_y: 0,
        last_seq: 0,
        server_tick: 1,
        server_time_ms: 820,
    });
    t.frame(20);
    assert_eq!(t.app.state(), AppState::Playing);

    // t = 1720 ms is past the original deadline (1000 ms) but inside the reset
    // one (1820 ms): only a reset watchdog keeps the app in Playing.
    t.frame(DISCONNECT_TIMEOUT_MS - 100);
    assert_eq!(
        t.app.state(),
        AppState::Playing,
        "a PlayerPos within the window must reset the watchdog deadline"
    );
}

/// TC6 / AC10: `Quit` from `Playing` sends exactly one `Leave` with the assigned
/// id, drops the socket, and terminates the app.
#[test]
fn quit_from_playing_sends_one_leave_with_assigned_id() {
    let mut t = make_test_app();
    t.reach_playing(7);

    // Discard handshake-phase traffic so the Leave count is unambiguous.
    let _ = t.drain_sent();

    t.app.dispatch(AppEvent::Quit, t.clock.now());
    assert_eq!(t.app.state(), AppState::Quitting);
    assert!(t.app.is_quitting());

    let leaves: Vec<u8> = t
        .drain_sent()
        .iter()
        .filter_map(|packet| match packet {
            GamePacket::Leave { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(
        leaves,
        vec![7],
        "Quit must send exactly one Leave carrying the assigned id"
    );

    // Dropping the World must drop the socket: the client receiver is gone.
    let bytes = postcard::to_allocvec(&GamePacket::JoinResponse { assigned_id: 7 })
        .expect("encode probe packet");
    let sender = t
        .inbound
        .borrow()
        .last()
        .cloned()
        .expect("a transport should exist");
    assert!(
        sender.send(bytes).is_err(),
        "Quit must drop the client transport (and its World)"
    );
}

/// TC6 / AC10: `QuitToMenu` from `Playing` sends exactly one `Leave` with the
/// assigned id and returns to `MainMenu`.
#[test]
fn quit_to_menu_from_playing_sends_one_leave() {
    let mut t = make_test_app();
    t.reach_playing(9);

    let _ = t.drain_sent();

    t.app.dispatch(AppEvent::QuitToMenu, t.clock.now());
    assert_eq!(t.app.state(), AppState::MainMenu);
    assert!(!t.app.is_quitting());

    let leaves: Vec<u8> = t
        .drain_sent()
        .iter()
        .filter_map(|packet| match packet {
            GamePacket::Leave { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(
        leaves,
        vec![9],
        "QuitToMenu must send exactly one Leave carrying the assigned id"
    );
}

/// TC6 / AC10: with no id assigned (still in `MainMenu`), `Quit` sends no
/// `Leave` — there is no socket and no player id.
#[test]
fn quit_without_assigned_id_sends_no_leave() {
    let mut t = make_test_app();
    assert_eq!(t.app.state(), AppState::MainMenu);

    t.app.dispatch(AppEvent::Quit, t.clock.now());
    assert_eq!(t.app.state(), AppState::Quitting);

    let sent = t.drain_sent();
    assert!(
        !sent
            .iter()
            .any(|packet| matches!(packet, GamePacket::Leave { .. })),
        "no Leave may be sent when no id is assigned, got {sent:?}"
    );
}
