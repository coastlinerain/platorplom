//! RED tests for the pure client game-state-machine core (spec TC1–TC3, AC5).
//!
//! These target `game::state_machine` and the two timeout constants in
//! `game::config`, none of which exist yet, so this test target fails to compile.
//! That is the intended RED state for plan Box 1.
//!
//! The expected values below are transcribed from the spec's transition table,
//! independently of the implementation (no test recomputes what the code does).

use game::config::{CONNECT_TIMEOUT_MS, DISCONNECT_TIMEOUT_MS};
use game::state_machine::{AppEvent, AppState, transition};

/// Every `AppState`, in declaration order, so the 6×11 grid can be iterated
/// programmatically instead of hand-writing 66 cases.
const ALL_STATES: [AppState; 6] = [
    AppState::MainMenu,
    AppState::Connecting,
    AppState::Playing,
    AppState::Paused,
    AppState::Disconnected,
    AppState::Quitting,
];

/// Every `AppEvent`, in declaration order (11 variants).
const ALL_EVENTS: [AppEvent; 11] = [
    AppEvent::StartJoin,
    AppEvent::Joined,
    AppEvent::ConnectFailed,
    AppEvent::ConnectionLost,
    AppEvent::PauseRequested,
    AppEvent::ResumeRequested,
    AppEvent::QuitToMenu,
    AppEvent::Cancel,
    AppEvent::Retry,
    AppEvent::BackToMenu,
    AppEvent::Quit,
];

/// The 16 valid edges of the spec's transition table: `(state, event, next)`.
/// This array is the single expected-value source the grid tests compare against.
const VALID_EDGES: [(AppState, AppEvent, AppState); 16] = [
    (
        AppState::MainMenu,
        AppEvent::StartJoin,
        AppState::Connecting,
    ),
    (AppState::MainMenu, AppEvent::Quit, AppState::Quitting),
    (AppState::Connecting, AppEvent::Joined, AppState::Playing),
    (
        AppState::Connecting,
        AppEvent::ConnectFailed,
        AppState::Disconnected,
    ),
    (AppState::Connecting, AppEvent::Cancel, AppState::MainMenu),
    (
        AppState::Playing,
        AppEvent::PauseRequested,
        AppState::Paused,
    ),
    (AppState::Playing, AppEvent::QuitToMenu, AppState::MainMenu),
    (
        AppState::Playing,
        AppEvent::ConnectionLost,
        AppState::Disconnected,
    ),
    (AppState::Playing, AppEvent::Quit, AppState::Quitting),
    (
        AppState::Paused,
        AppEvent::ResumeRequested,
        AppState::Playing,
    ),
    (AppState::Paused, AppEvent::QuitToMenu, AppState::MainMenu),
    (
        AppState::Paused,
        AppEvent::ConnectionLost,
        AppState::Disconnected,
    ),
    (AppState::Paused, AppEvent::Quit, AppState::Quitting),
    (
        AppState::Disconnected,
        AppEvent::Retry,
        AppState::Connecting,
    ),
    (
        AppState::Disconnected,
        AppEvent::BackToMenu,
        AppState::MainMenu,
    ),
    (AppState::Disconnected, AppEvent::Quit, AppState::Quitting),
];

/// The table target for a pair, or `None` when the pair must be a no-op.
fn table_target(state: AppState, event: AppEvent) -> Option<AppState> {
    VALID_EDGES
        .iter()
        .find(|(from, ev, _)| *from == state && *ev == event)
        .map(|(_, _, next)| *next)
}

/// TC1 (AC2): each of the 16 table rows transitions to its documented target.
#[test]
fn all_sixteen_valid_edges_transition_to_their_target() {
    for &(from, event, expected) in &VALID_EDGES {
        assert_eq!(
            transition(from, event),
            expected,
            "transition({from:?}, {event:?}) should be {expected:?}"
        );
    }
}

/// TC2 (AC3): the exhaustive 6×11 grid. The 16 table pairs hit their target and
/// the other 50 pairs return the input state unchanged.
#[test]
fn exhaustive_grid_is_a_no_op_outside_the_sixteen_edges() {
    let mut edges = 0usize;
    let mut no_ops = 0usize;

    for &state in &ALL_STATES {
        for &event in &ALL_EVENTS {
            let next = transition(state, event);
            match table_target(state, event) {
                Some(expected) => {
                    edges += 1;
                    assert_eq!(
                        next, expected,
                        "table pair ({state:?}, {event:?}) must transition to {expected:?}"
                    );
                    assert_ne!(
                        next, state,
                        "table pair ({state:?}, {event:?}) must change state, not be a no-op"
                    );
                }
                None => {
                    no_ops += 1;
                    assert_eq!(
                        next, state,
                        "non-table pair ({state:?}, {event:?}) must be a no-op"
                    );
                }
            }
        }
    }

    assert_eq!(edges, 16, "the table has exactly 16 valid edges");
    assert_eq!(no_ops, 50, "the 6×11 grid minus 16 edges leaves 50 no-ops");
}

/// The expected-value table itself is well-formed: 16 distinct `(state, event)`
/// keys, so the grid counts above are meaningful.
#[test]
fn valid_edge_table_has_sixteen_distinct_pairs() {
    assert_eq!(VALID_EDGES.len(), 16);

    for (i, &(from, event, _)) in VALID_EDGES.iter().enumerate() {
        for &(other_from, other_event, _) in &VALID_EDGES[i + 1..] {
            assert!(
                !(from == other_from && event == other_event),
                "duplicate table entry for ({from:?}, {event:?})"
            );
        }
    }
}

/// TC3 (AC4): `Quitting` is terminal — no event leaves it.
#[test]
fn quitting_is_terminal_for_every_event() {
    for &event in &ALL_EVENTS {
        assert_eq!(
            transition(AppState::Quitting, event),
            AppState::Quitting,
            "event {event:?} must not leave Quitting"
        );
    }
}

/// AC1: the types expose the derives the runner and the grid rely on
/// (`Copy` to iterate, `Eq`/`Hash` for state comparisons).
#[test]
fn app_state_and_app_event_derive_copy_eq_hash() {
    fn assert_traits<T: Copy + Eq + std::hash::Hash + std::fmt::Debug>() {}

    assert_traits::<AppState>();
    assert_traits::<AppEvent>();
}

/// AC5: the connect / disconnect timeouts have their spec-mandated values.
#[test]
fn timeout_constants_match_the_spec() {
    assert_eq!(CONNECT_TIMEOUT_MS, 5000);
    assert_eq!(DISCONNECT_TIMEOUT_MS, 1000);
}
