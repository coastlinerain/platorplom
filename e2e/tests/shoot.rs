// Group D — Shoot Tests (6 tests)

use e2e::harness::TestHarness;

#[test]
fn d1_single_shoot_produces_one_server_shot() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.press_shoot(p0);
    h.tick_n(5);

    let count = h.server_bullet_count(p0);
    assert_eq!(
        count, 1,
        "one press should produce one server bullet, got {count}"
    );
}

#[test]
fn d2_client_sees_own_bullet() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.press_shoot(p0);
    h.tick_n(5);

    let count = h.client_bullet_count(p0);
    assert!(count > 0, "client should see own bullet, got {count}");
}

#[test]
fn d3_three_presses_produce_three_bullets() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Each press triggers up to 3 frames of shoot=true via sticky edges.
    // With 5 ticks between presses, the sticky from one press may
    // already be active when the next arrives.  Three Z presses produce
    // between 5 and 9 server shots depending on frame alignment.
    // The test asserts only the lower bound (≥ 3).
    h.press_shoot(p0);
    h.tick_n(8);
    h.press_shoot(p0);
    h.tick_n(8);
    h.press_shoot(p0);
    h.tick_n(8);

    let count = h.server_bullet_count(p0);
    assert!(
        count >= 3,
        "three presses should spawn at least 3 bullets, got {count}"
    );
    assert!(
        count <= 6,
        "multi-fire regression: three presses produced {count} bullets (expected ≤ 6 with cooldown)"
    );
}

#[test]
fn d4_shoot_latch_clears_after_one_tick() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    h.press_shoot(p0);
    h.tick_n(10);

    let count = h.server_bullet_count(p0);
    assert_eq!(
        count, 1,
        "latch should clear after one tick; expected 1 bullet, got {count}"
    );
}

#[test]
fn d5_bullet_direction_matches_player_direction() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();

    // Default facing is right (dir = 1.0).
    // Tick one at a time until the server processes the shoot and
    // last_shots is populated (happens within 1-3 ticks).
    h.press_shoot(p0);
    for _ in 0..5 {
        h.tick();
        if !h.last_shots.is_empty() {
            break;
        }
    }

    let right_shots = h
        .last_shots
        .iter()
        .filter(|&&(pid, dir)| pid == h.client(p0).player_id && dir > 0.0)
        .count();
    assert!(
        right_shots > 0,
        "bullet should fire in player's facing direction (right)"
    );

    // Now face left and verify bullets go left.
    h.hold_left(p0, 5);
    h.tick_n(5);
    h.press_shoot(p0);
    for _ in 0..5 {
        h.tick();
        if !h.last_shots.is_empty() {
            break;
        }
    }

    let left_shots = h
        .last_shots
        .iter()
        .filter(|&&(pid, dir)| pid == h.client(p0).player_id && dir < 0.0)
        .count();
    assert!(
        left_shots > 0,
        "bullet should fire left when player faces left"
    );
}

#[test]
fn d6_remote_receives_action_for_bullet() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    let _p1 = h.connect_client();

    let p0_id = h.client(p0).player_id;

    // p0 shoots. Loop until the server processes the shot and
    // last_actions is populated (happens within 1-3 ticks).
    h.press_shoot(p0);
    for _ in 0..5 {
        h.tick();
        if !h.last_actions.is_empty() {
            break;
        }
    }

    // Check that p1 received an Action packet for p0's shoot.
    let actions_for_remote: Vec<_> = h
        .last_actions
        .iter()
        .filter(|(handle, kind, _dir)| *kind == "shoot" && h.client(*handle).player_id != p0_id)
        .collect();

    assert!(
        !actions_for_remote.is_empty(),
        "remote client should receive an Action packet when p0 shoots"
    );
}
