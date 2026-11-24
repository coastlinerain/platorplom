// Group A — Basic Movement Tests (7 tests)

use e2e::harness::TestHarness;

#[test]
fn a1_move_right_changes_position() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_right(p0, 30);
    h.tick_n(30);

    let (sx, _sy) = h.server_player_pos(p0);
    let (cx, _cy) = h.client_player_pos(p0);

    assert!(sx > 150.0, "server x should be > 150, got {sx}");
    assert!(cx > 150.0, "client x should be > 150, got {cx}");
}

#[test]
fn a2_move_left_changes_position() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_left(p0, 30);
    h.tick_n(30);

    let (sx, _sy) = h.server_player_pos(p0);
    let (cx, _cy) = h.client_player_pos(p0);

    assert!(sx < 100.0, "server x should be < 100, got {sx}");
    assert!(cx < 100.0, "client x should be < 100, got {cx}");
}

#[test]
fn a3_jump_when_grounded_moves_up() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.set_server_player_grounded(p0, true);
    h.press_jump(p0);
    h.tick_n(2);

    let (_sx, sy) = h.server_player_pos(p0);
    assert!(sy < 100.0, "player y should be < 100 after jump, got {sy}");
}

#[test]
fn a4_jump_ignored_when_airborne() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    let (_sx, start_y) = h.server_player_pos(p0);

    h.press_jump(p0);
    h.tick_n(10);

    let (_sx, end_y) = h.server_player_pos(p0);

    assert!(
        end_y > start_y,
        "airborne jump should be ignored; y should increase due to gravity"
    );
}

#[test]
fn a5_gravity_pulls_player_down() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.tick_n(60);

    let (_sx, sy) = h.server_player_pos(p0);
    assert!(sy > 100.0, "gravity should pull player down; got {sy}");
}

#[test]
fn a6_simultaneous_right_and_jump_both_apply() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.set_server_player_grounded(p0, true);

    h.hold_right(p0, 5);
    h.press_jump(p0);
    h.tick_n(5);

    let (sx, sy) = h.server_player_pos(p0);

    assert!(sx > 100.0, "player should move right, got x={sx}");
    assert!(sy < 100.0, "player should jump up, got y={sy}");
}

#[test]
fn a7_prediction_tracks_server_after_reconciliation() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_right(p0, 60);
    h.tick_n(60);

    let (sx, _sy) = h.server_player_pos(p0);
    let (cx, _cy) = h.client_player_pos(p0);

    let error = (cx - sx).abs();
    assert!(
        error < 1.0,
        "prediction error should be < 1.0 after 60 ticks, got {error} (client={cx}, server={sx})"
    );
}
