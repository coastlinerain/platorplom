// Group B — Room Transition Tests (9 tests)

use e2e::harness::TestHarness;

#[test]
fn b1_transition_right_changes_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Place at x=766 so player's right edge (796) exceeds the 795 px
    // threshold. colision's 2px horizontal inset keeps the sample
    // points at (768..796) inside the 800px level at col 19 (Empty),
    // so the player is not stuck. Both server AND client must be at
    // the edge so that client-side check_level_transitions fires.
    h.set_server_player_pos(p0, 766.0, 320.0);
    h.set_client_player_pos(p0, 766.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);

    let coords = h.client_player_coords(p0);
    assert_eq!(
        coords,
        (1, 0),
        "expected coords (1,0) after right transition, got {coords:?}"
    );
}

#[test]
fn b2_transition_left_changes_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // First transition into room (1, 0).
    h.set_server_player_pos(p0, 790.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    // Now move left to go back.
    h.set_server_player_pos(p0, 10.0, 320.0);
    h.hold_left(p0, 10);
    h.tick_n(10);

    let coords = h.client_player_coords(p0);
    assert_eq!(
        coords,
        (0, 0),
        "expected coords (0,0) after left transition, got {coords:?}"
    );
}

#[test]
fn b3_transition_down_changes_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.set_server_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);

    let (_cx, cy) = h.client_player_coords(p0);
    assert_eq!(cy, 1, "expected row 1 after down transition, got {cy}");
}

#[test]
fn b4_transition_up_changes_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Go down first.
    h.set_server_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);
    assert_eq!(h.client_player_coords(p0).1, 1);

    // Now jump up.
    h.set_server_player_pos(p0, 300.0, 10.0);
    h.set_server_player_grounded(p0, true);
    h.press_jump(p0);
    h.tick_n(5);

    let (_cx, cy) = h.client_player_coords(p0);
    assert_eq!(cy, 0, "expected row 0 after up transition, got {cy}");
}

#[test]
fn b5_not_stuck_after_right_transition() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Transition right into room (1, 0).
    h.set_server_player_pos(p0, 766.0, 320.0);
    h.set_client_player_pos(p0, 766.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    // After transition, reposition server player inside the new room.
    // The old position (766,320) is a wall in room (1,0) — the server
    // player is stuck there until we move it.
    h.set_server_player_pos(p0, 100.0, 320.0);

    let (start_x, _) = h.server_player_pos(p0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    let (end_x, _) = h.server_player_pos(p0);

    assert!(
        end_x > start_x,
        "player should be able to move right after right transition"
    );
}

#[test]
fn b6_not_stuck_after_left_transition() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Go right into room (1, 0).
    h.set_server_player_pos(p0, 766.0, 320.0);
    h.set_client_player_pos(p0, 766.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    // Reposition in room (1, 0) at a safe spot, then go left back.
    h.set_server_player_pos(p0, 100.0, 320.0);
    h.set_client_player_pos(p0, 100.0, 320.0);

    // Transition left: move to left edge of room (1,0) and cross.
    h.set_server_player_pos(p0, 5.0, 320.0);
    h.set_client_player_pos(p0, 5.0, 320.0);
    h.hold_left(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (0, 0));

    // Reposition server player in room (0,0) after transition.
    h.set_server_player_pos(p0, 300.0, 320.0);

    let (start_x, _) = h.server_player_pos(p0);
    h.hold_left(p0, 10);
    h.tick_n(10);
    let (end_x, _) = h.server_player_pos(p0);

    assert!(
        end_x < start_x,
        "player should be able to move left after left transition"
    );
}

#[test]
fn b7_not_stuck_after_down_transition() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Transition down into room (0, 1).
    h.set_server_player_pos(p0, 300.0, 470.0);
    h.set_client_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);
    assert_eq!(h.client_player_coords(p0).1, 1);

    // Reposition server player inside room (0,1) — the old position
    // (300,470) is past the bottom boundary of the new room's grid.
    h.set_server_player_pos(p0, 300.0, 100.0);

    let (start_x, _) = h.server_player_pos(p0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    let (end_x, _) = h.server_player_pos(p0);

    assert!(
        end_x > start_x,
        "player should be able to move right after down transition"
    );
}

#[test]
fn b8_not_stuck_after_up_transition() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Go down into room (0, 1).
    h.set_server_player_pos(p0, 300.0, 470.0);
    h.set_client_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);
    assert_eq!(h.client_player_coords(p0).1, 1);

    // Reposition in room (0,1) then jump up back to (0,0).
    h.set_server_player_pos(p0, 300.0, 100.0);
    h.set_client_player_pos(p0, 300.0, 100.0);

    // Jump up to transition back to room (0,0).
    h.set_server_player_pos(p0, 300.0, 10.0);
    h.set_client_player_pos(p0, 300.0, 10.0);
    h.set_server_player_grounded(p0, true);
    h.press_jump(p0);
    h.tick_n(5);
    assert_eq!(h.client_player_coords(p0).1, 0);

    // Reposition server player in room (0,0) after transition.
    h.set_server_player_pos(p0, 300.0, 320.0);

    let (start_x, _) = h.server_player_pos(p0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    let (end_x, _) = h.server_player_pos(p0);

    assert!(
        end_x > start_x,
        "player should be able to move right after up transition"
    );
}

#[test]
fn b9_stay_in_same_room_at_map_edge() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_left(p0, 200);
    h.tick_n(200);

    let coords = h.client_player_coords(p0);
    assert_eq!(
        coords,
        (0, 0),
        "player should stay in room (0,0) at map edge, got {coords:?}"
    );
}
