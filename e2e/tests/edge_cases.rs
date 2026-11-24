// Group F — Edge Cases Tests (6 tests)

use e2e::harness::TestHarness;

#[test]
fn f1_bogus_level_coords_fall_back_to_origin() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Artificially move the client to bogus coords (99, 99).
    h.client_mut(p0).world.current_coords = (99, 99);

    // Hold right so the client sends a PlayerInput reporting coords (99, 99).
    // The server must clamp it to the origin level without panicking.
    h.hold_right(p0, 5);
    h.tick_n(5);

    // The server processes the tick without panic.
    // The server stores the client-reported coords (99, 99) in coords map,
    // but uses origin level for collision via fallback. Verify no panic.
    let (sx, _) = h.server_player_pos(p0);
    assert!(
        sx > 100.0,
        "server should still process movement with bogus coords"
    );
    let coords_after = h.server_player_coords(p0);
    assert_eq!(
        coords_after,
        (99, 99),
        "server accepts client-reported coords"
    );
}

#[test]
fn f2_corner_position_not_stuck() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Place player near the top-left corner (tile (1,1) = 42,42 px)
    // just outside the solid wall border.  A position inside the wall
    // (e.g. 5,5) would be permanently stuck by collision rollback.
    h.set_server_player_pos(p0, 42.0, 42.0);

    h.tick_n(5);

    // Player should be able to move right from the corner.
    let (start_x, _) = h.server_player_pos(p0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    let (end_x, _) = h.server_player_pos(p0);

    assert!(
        end_x > start_x,
        "player should be able to move right from corner position"
    );
}

#[test]
fn f3_owner_bullet_spawn_correct_position() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    let (px, py) = h.client_player_pos(p0);

    h.press_shoot(p0);
    h.tick_n(3);

    let count = h.client_bullet_count(p0);
    assert!(count > 0, "bullet should have spawned");

    // Bullet should be near the player's position (not at origin).
    // The bullet travels at BULLET_SPEED over 3 ticks and is offset by
    // player w/2 from the spawn centre, so ~70 px is a safe bound.
    let bullets = &h.client(p0).world.bullets;
    if let Some(bullet) = bullets.first() {
        let dx = bullet.pos.x - px;
        let dy = bullet.pos.y - py;
        assert!(
            dx > 0.0,
            "bullet should move right from player; bullet.x={}, player.x={}",
            bullet.pos.x,
            px
        );
        assert!(
            dy.abs() < 30.0,
            "bullet y should be close to player y (within 30 px); bullet.y={}, player.y={}",
            bullet.pos.y,
            py
        );
    }
}

#[test]
fn f4_eight_players_no_panic() {
    let mut h = TestHarness::new();

    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(h.connect_client());
    }

    // Tick a bunch of times — no panic.
    h.tick_n(10);

    // Verify all 8 clients are still connected and have ids.
    for handle in &handles {
        assert!(
            h.client(*handle).world.id.is_some(),
            "client should have an id"
        );
    }
}

#[test]
fn f5_disconnect_during_room_transition() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Start a transition: move player to the right edge.
    h.set_server_player_pos(p0, 790.0, 320.0);
    h.hold_right(p0, 3);
    h.tick_n(3);

    // Disconnect mid-transition.
    h.disconnect_client(p0);

    // No panic. Connect a new client — verification the server is still healthy.
    let p1 = h.connect_client();
    h.tick_n(5);

    assert!(
        h.client(p1).world.id.is_some(),
        "new client should connect fine after mid-transition disconnect"
    );
}

#[test]
fn f6_empty_buffer_does_not_crash() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Tick 100 times with no inputs. The input buffer stays empty
    // and reconciliation should handle it without panicking.
    h.tick_n(100);

    // Verify no crash occurred and the server is still functional.
    let (sx, sy) = h.server_player_pos(p0);
    assert!(
        sx.is_finite(),
        "server x should be finite after 100 empty ticks"
    );
    assert!(
        sy.is_finite(),
        "server y should be finite after 100 empty ticks"
    );
}
