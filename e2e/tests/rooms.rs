// Group B — Room Transition Tests (9 tests)

use e2e::harness::TestHarness;

#[test]
fn b1_transition_right_changes_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.set_server_player_pos(p0, 790.0, 320.0);
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

    h.set_server_player_pos(p0, 790.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

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

    // Go right then left.
    h.set_server_player_pos(p0, 790.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    h.set_server_player_pos(p0, 10.0, 320.0);
    h.hold_left(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (0, 0));

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

    h.set_server_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);
    assert_eq!(h.client_player_coords(p0).1, 1);

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

    // Go down then up.
    h.set_server_player_pos(p0, 300.0, 470.0);
    h.tick_n(15);
    assert_eq!(h.client_player_coords(p0).1, 1);

    h.set_server_player_pos(p0, 300.0, 10.0);
    h.set_server_player_grounded(p0, true);
    h.press_jump(p0);
    h.tick_n(5);
    assert_eq!(h.client_player_coords(p0).1, 0);

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
