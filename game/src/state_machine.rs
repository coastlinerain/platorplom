//! Pure client game-state-machine core.
//!
//! This module holds the flat, total state machine that owns the client's
//! application lifecycle: `MainMenu → Connecting → Playing ⇄ Paused`, with
//! `Disconnected` for failure/loss and terminal `Quitting`.
//!
//! The core is deliberately side-effect free: `transition` maps a
//! `(state, event)` pair to the next state and nothing else. Socket binding,
//! `World` construction, `Leave` sending, and timeout bookkeeping all live in
//! the `client::app` runner, driven by the results of `transition`.

/// The six application states of the client lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    /// Idle: no socket, no `World`.
    MainMenu,
    /// Socket bound, `JoinRequest` sent, awaiting `JoinResponse`.
    Connecting,
    /// `World` live; full simulation + networking + drawing.
    Playing,
    /// `World` alive but frozen (no sim, no input send); still receiving.
    Paused,
    /// Connection failed or lost; socket dropped.
    Disconnected,
    /// Terminal: the application loop exits.
    Quitting,
}

/// The eleven events that drive state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppEvent {
    StartJoin,
    Joined,
    ConnectFailed,
    ConnectionLost,
    PauseRequested,
    ResumeRequested,
    QuitToMenu,
    Cancel,
    Retry,
    BackToMenu,
    Quit,
}

/// Total transition function.
///
/// Returns the next state for a valid `(state, event)` pair, or the input
/// `state` unchanged (a no-op) for any pair not in the transition table.
/// `Quitting` is terminal: no event leaves it. This function never panics and
/// never returns an `Option`/sentinel — every one of the 6 × 11 = 66 pairs has
/// a defined result (16 edges + 50 no-ops).
pub fn transition(state: AppState, event: AppEvent) -> AppState {
    use AppEvent::*;
    use AppState::*;

    match (state, event) {
        (MainMenu, StartJoin) => Connecting,
        (MainMenu, Quit) => Quitting,
        (Connecting, Joined) => Playing,
        (Connecting, ConnectFailed) => Disconnected,
        (Connecting, Cancel) => MainMenu,
        (Playing, PauseRequested) => Paused,
        (Playing, QuitToMenu) => MainMenu,
        (Playing, ConnectionLost) => Disconnected,
        (Playing, Quit) => Quitting,
        (Paused, ResumeRequested) => Playing,
        (Paused, QuitToMenu) => MainMenu,
        (Paused, ConnectionLost) => Disconnected,
        (Paused, Quit) => Quitting,
        (Disconnected, Retry) => Connecting,
        (Disconnected, BackToMenu) => MainMenu,
        (Disconnected, Quit) => Quitting,
        // Quitting is terminal: every event is a no-op.
        (Quitting, _) => Quitting,
        // Everything else is a no-op.
        _ => state,
    }
}
