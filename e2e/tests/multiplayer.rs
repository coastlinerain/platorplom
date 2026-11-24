// Group C — Multiplayer Visibility Tests (7 tests)

use e2e::harness::TestHarness;

#[test]
fn c1_two_clients_connect_and_receive_ids() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let p1 = h.connect_client();

    let id0 = h.client(p0).world.id;
    let id1 = h.client(p1).world.id;

    assert!(id0.is_some(), "p0 should have an assigned id");
    assert!(id1.is_some(), "p1 should have an assigned id");
    assert_ne!(id0, id1, "clients should have different ids");
}

#[test]
fn c2_remote_player_position_is_visible() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let p1 = h.connect_client();

    let p0_id = h.client(p0).world.id.unwrap();

    // Move p0 right for 30 ticks.
    h.hold_right(p0, 30);
    h.tick_n(30);

    // p1 should see p0 at its updated position.
    let p1_remote = h
        .client(p1)
        .world
        .other_players
        .get(&p0_id)
        .expect("p1 should see p0 in other_players");

    assert!(
        p1_remote.pos.x > 100.0,
        "p1 should see p0's updated position; got x={}",
        p1_remote.pos.x
    );
}

#[test]
fn c3_remote_interp_has_snapshots_after_playerpos() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let p1 = h.connect_client();

    let p0_id = h.client(p0).world.id.unwrap();

    // A few ticks produce PlayerPos broadcasts, which push interpolation snapshots.
    h.hold_right(p0, 10);
    h.tick_n(10);

    let interp = h
        .client(p1)
        .world
        .other_interp
        .get(&p0_id)
        .expect("p1 should have interp state for p0");

    let has_snapshots = interp.timestamps.iter().any(|t| t.is_some());
    assert!(
        has_snapshots,
        "p1 should have interpolation snapshots for p0"
    );
}

#[test]
fn c4_late_join_sees_existing_players() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    let p0_id = h.client(p0).world.id.unwrap();

    // Move p0 right for 30 ticks.
    h.hold_right(p0, 30);
    h.tick_n(30);

    // p1 joins late.
    let p1 = h.connect_client();

    // Tick once so p1 receives a PlayerPos broadcast for p0.
    h.tick_n(1);

    let p1_sees_p0 = h.client(p1).world.other_players.contains_key(&p0_id);
    assert!(p1_sees_p0, "p1 (late-join) should see existing player p0");
}

#[test]
fn c5_player_leave_removes_remote_entry() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let p1 = h.connect_client();

    let p0_id = h.client(p0).world.id.unwrap();

    // Tick so p1 knows about p0.
    h.tick_n(1);

    // Disconnect p0.
    h.disconnect_client(p0);

    // Tick once so p1 receives the Leave packet.
    h.tick_n(1);

    let p1_still_sees_p0 = h.client(p1).world.other_players.contains_key(&p0_id);
    assert!(
        !p1_still_sees_p0,
        "p1 should no longer see p0 after disconnect"
    );
}

#[test]
fn c6_remote_in_different_room_has_different_coords() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let p1 = h.connect_client();

    let p0_id = h.client(p0).world.id.unwrap();

    // Transition p0 to room (1, 0).  Place at x=770 so the player's right
    // edge (800) is exactly at the level boundary — one pixel further right
    // triggers the transition, and the 2px inset on colision points stays
    // inside the 800px level (all empty tile in row 8).
    h.set_server_player_pos(p0, 770.0, 320.0);
    h.set_client_player_pos(p0, 770.0, 320.0);
    h.hold_right(p0, 10);
    h.tick_n(10);
    assert_eq!(h.client_player_coords(p0), (1, 0));

    // Tick so p1 gets the updated PlayerPos for p0.
    h.tick_n(1);

    let p1_coords = (
        h.client(p1).world.current_coords.0,
        h.client(p1).world.current_coords.1,
    );
    let p0_coords_in_p1_view = h
        .client(p1)
        .world
        .other_players
        .get(&p0_id)
        .map(|n| (n.coords.0, n.coords.1));

    if let Some(remote_coords) = p0_coords_in_p1_view {
        assert_ne!(
            remote_coords, p1_coords,
            "p1 should see p0 in a different room"
        );
    }
}

#[test]
fn c7_full_broadcast_to_all_players() {
    let mut h = TestHarness::new();
    let _p0 = h.connect_client();
    let _p1 = h.connect_client();
    let _p2 = h.connect_client();

    h.tick_n(1);

    // 3 clients × 3 players = 9 broadcast entries.
    assert_eq!(
        h.last_broadcasts.len(),
        9,
        "broadcast should include 3 players × 3 clients = 9 entries, got {}",
        h.last_broadcasts.len()
    );
}
