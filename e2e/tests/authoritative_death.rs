// Spec 2 — Authoritative player death E2E tests.
//
// The server is the sole authority on player death: a shot raycast hits the
// closest other player in the shooter's room, the server respawns the victim
// and broadcasts `PlayerDied`, and no client ever self-reports a death.
//
// Geometry: room (0,0), row 5 (`y ≈ 201`, `TILE_SIZE = 40`) is fully open
// between the border walls. The shooter's bullet origin is `(115, 220)`; the
// victim's body is `x ∈ [400, 430]`, `y ≈ [201, 239]` → vertical overlap, no
// wall, hit at `hit_x = 400`.

use e2e::harness::{ClientHandle, TestHarness};
use metroidvania_server::PlayerHitInfo;

/// Connect a shooter and a victim, put both in room (0,0) on row 5, mirror the
/// positions on the clients, and settle for 3 ticks.
fn setup_pvp(h: &mut TestHarness) -> (ClientHandle, ClientHandle) {
    let shooter = h.connect_client();
    let victim = h.connect_client();

    h.set_server_player_coords(shooter, 0, 0);
    h.set_server_player_coords(victim, 0, 0);

    h.set_server_player_pos(shooter, 100.0, 201.0);
    h.set_server_player_pos(victim, 400.0, 201.0);
    h.set_server_player_grounded(shooter, true);
    h.set_server_player_grounded(victim, true);

    h.set_client_player_pos(shooter, 100.0, 201.0);
    h.set_client_player_pos(victim, 400.0, 201.0);

    h.tick_n(3);

    (shooter, victim)
}

/// E1 — one shot in the same room kills exactly the victim: one `PlayerDied`
/// fan-out, the victim's client respawns at (100, 100) in room (0,0), and the
/// shooter stays where it was.
///
/// `last_deaths` is refreshed every tick, so the loop must break out on the
/// same tick the death appears (never read it after a later tick).
#[test]
fn e2e_shoot_kills_player_in_same_room() {
    let mut h = TestHarness::new();
    let (shooter, victim) = setup_pvp(&mut h);

    let shooter_id = h.client(shooter).player_id;
    let victim_id = h.client(victim).player_id;
    let (sx, sy) = h.client_player_pos(shooter);

    h.press_shoot(shooter);
    for _ in 0..5 {
        h.tick();
        if !h.last_deaths.is_empty() {
            break;
        }
    }

    assert_eq!(
        h.last_deaths.len(),
        1,
        "expected exactly one death, got {:?}",
        h.last_deaths
    );
    assert_eq!(
        h.last_deaths[0],
        PlayerHitInfo {
            shooter_id,
            victim_id
        }
    );

    // The victim's client applied the authoritative respawn.
    let (vx, vy) = h.client_player_pos(victim);
    assert!(
        (vx - 100.0).abs() <= 2.0 && (vy - 100.0).abs() <= 2.0,
        "victim should respawn near (100, 100), got ({vx}, {vy})"
    );
    assert_eq!(h.client_player_coords(victim), (0, 0));

    // The shooter did not respawn.
    let (x, y) = h.client_player_pos(shooter);
    assert!(
        (x - sx).abs() <= 5.0 && (y - sy).abs() <= 5.0,
        "shooter should stay near ({sx}, {sy}), got ({x}, {y})"
    );
}

/// E2 — the victim's client never self-reports: the death is server-originated,
/// so the harness observes at least one death and zero inbound `PlayerDied`
/// packets from any client.
#[test]
fn e2e_victim_does_not_self_report() {
    let mut h = TestHarness::new();
    let (shooter, _victim) = setup_pvp(&mut h);

    h.press_shoot(shooter);
    for _ in 0..5 {
        h.tick();
        if !h.last_deaths.is_empty() {
            break;
        }
    }

    // Guard against a vacuous pass: the death must have happened.
    assert!(
        !h.last_deaths.is_empty(),
        "expected a server-detected death before checking self-reports"
    );
    assert_eq!(
        h.client_death_reports, 0,
        "no client may send PlayerDied (server-authoritative death)"
    );
}
