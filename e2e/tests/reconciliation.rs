// Group E — Reconciliation Tests (6 tests)

use e2e::harness::TestHarness;

// Used in e3 to inject a stale PlayerPos directly.
use protocol::GamePacket;

#[test]
fn e1_reconciliation_corrects_position_toward_server() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Move left so the server has a position left of spawn.
    h.hold_left(p0, 30);
    h.tick_n(30);

    let (sx, sy) = h.server_player_pos(p0);

    // Set client position far to the right — a deliberate mismatch.
    h.set_client_player_pos(p0, 200.0, sy);

    let error_before = (200.0 - sx).abs();

    // Tick a few times. Reconciliation should bring client toward server.
    h.tick_n(5);

    let (cx, _cy) = h.client_player_pos(p0);
    let error_after = (cx - sx).abs();

    assert!(
        error_after < error_before,
        "reconciliation should reduce error; before={error_before}, after={error_after}"
    );
}

#[test]
fn e2_no_large_jumps_during_reconciliation() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    let mut prev_x = h.client_player_pos(p0).0;
    let mut max_delta = 0.0f32;

    h.hold_right(p0, 60);
    for _ in 0..60 {
        h.tick_n(1);
        let (cx, _) = h.client_player_pos(p0);
        let delta = (cx - prev_x).abs();
        if delta > max_delta {
            max_delta = delta;
        }
        prev_x = cx;
    }

    assert!(
        max_delta < 20.0,
        "max single-frame delta should be < 20 px, got {max_delta}"
    );
}

#[test]
fn e3_stale_playerpos_ignored() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.tick_n(5);

    let player_id = h.client(p0).player_id;
    h.client_mut(p0).world.last_acked_seq = 100;

    let stale = GamePacket::PlayerPos {
        id: player_id,
        x: 999.0,
        y: 999.0,
        dir: 1.0,
        level_x: 0,
        level_y: 0,
        last_seq: 50,
    };
    let bytes = postcard::to_allocvec(&stale).expect("serialize");
    let _ = h.to_clients[0].send(bytes);

    h.tick_n(1);

    let client_x_after = h.client_player_pos(p0).0;
    assert!(
        (client_x_after - 999.0).abs() > 100.0,
        "stale PlayerPos should be ignored; client x={client_x_after}"
    );
}

#[test]
fn e4_first_playerpos_sets_last_acked() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    assert_eq!(
        h.client(p0).world.last_acked_seq,
        u16::MAX,
        "last_acked_seq should start at u16::MAX"
    );

    h.tick_n(1);

    assert_ne!(
        h.client(p0).world.last_acked_seq,
        u16::MAX,
        "first PlayerPos should set last_acked_seq away from u16::MAX"
    );
}

#[test]
fn e5_replay_uses_correct_room_collision() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.set_server_player_pos(p0, 790.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    h.tick_n(5);

    let (start_x, _) = h.client_player_pos(p0);
    h.hold_right(p0, 5);
    h.tick_n(5);
    let (end_x, _) = h.client_player_pos(p0);

    assert!(
        end_x > start_x,
        "player should be able to move right after room transition"
    );
}

#[test]
fn e6_position_error_bounded_after_100_ticks() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.hold_right(p0, 100);
    for _ in 0..100 {
        h.tick_n(1);
        let (sx, _) = h.server_player_pos(p0);
        let (cx, _) = h.client_player_pos(p0);
        let error = (cx - sx).abs();
        assert!(
            error < 5.0,
            "prediction error should stay below 5.0; error={error}"
        );
    }
}
