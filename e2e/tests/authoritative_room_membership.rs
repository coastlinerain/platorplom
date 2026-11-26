// Spec 3 — authoritative room membership E2E tests.
//
// Room membership is server-authoritative: the client sends a direction-only
// `RequestRoomChange` when its *predicted* body reaches a door's raw geometric
// band, the server validates the request against its own hitbox (plus the
// server-only lead tolerance) and the door graph, and only then teleports the
// player to the canonical entry and acks the requester.
//
// The harness additions these tests compile against (`client_room_change_requests`,
// `client_room_change_rejects`, `room_change_responses`, and the
// `force_teleport`-backed `set_server_player_coords`) land in Box 7, so this
// file is expected to be RED (compile error) until then.

use e2e::harness::{ClientHandle, TestHarness};
use game::room_graph::Direction;
use metroidvania_server::PlayerHitInfo;
use protocol::GamePacket;

/// Stage both the server and the client for `p` at `pos` in `room`.
fn place(h: &mut TestHarness, p: ClientHandle, room: (u8, u8), pos: (f32, f32)) {
    h.set_server_player_coords(p, room.0, room.1);
    h.set_server_player_pos(p, pos.0, pos.1);
    h.set_client_player_pos(p, pos.0, pos.1);
    h.set_server_player_grounded(p, true);
}

/// Connect a shooter and a victim, put both in room (0,0) on row 5, mirror the
/// positions on the clients, and settle for 3 ticks (copied from
/// `authoritative_death.rs`).
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

/// E1 — a valid East door transition changes the room once, teleports to the
/// canonical entry, and does not bounce back while the client stands there.
///
/// A single `hold_right` frame is used (not ten): on the acceptance tick the
/// server walks ~6.7 px off the entry if movement is still held, which would
/// defeat the ±2 px entry assertion (plan deviation 12). The client starts
/// already inside the 40 px geometric band, so one frame keeps the walk-in.
#[test]
fn e2e_valid_door_transition_changes_room_once() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    h.tick_n(1);

    place(&mut h, p0, (0, 0), (766.0, 320.0));
    h.hold_right(p0, 1);

    let mut transitioned = false;
    for _ in 0..10 {
        h.tick();
        if h.client_player_coords(p0) == (1, 0) {
            transitioned = true;
            break;
        }
    }
    assert!(transitioned, "client never entered room (1, 0)");

    h.release_all(p0);
    h.tick_n(2);

    assert_eq!(h.client_player_coords(p0), (1, 0));
    assert_eq!(h.server_player_coords(p0), (1, 0));
    let (x, y) = h.client_player_pos(p0);
    assert!(
        (x - 10.0).abs() <= 2.0 && (y - 321.0).abs() <= 2.0,
        "client should sit at the canonical entry (10, 321), got ({x}, {y})"
    );

    // Extra ticks without input must not bounce the client back out.
    h.tick_n(2);
    assert_eq!(
        h.client_player_coords(p0),
        (1, 0),
        "client bounced back out of (1, 0) while standing at the entry"
    );
}

/// E2 — direct requests for non-doors are rejected: the room does not change,
/// and the client snaps back to the server's authoritative position. Each
/// request gets exactly one response (per-requester routing).
#[test]
fn e2e_non_door_teleport_is_rejected() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    place(&mut h, p0, (0, 0), (300.0, 200.0));

    let id = h.client(p0).player_id;
    for direction in [Direction::North, Direction::West, Direction::East] {
        h.client_mut(p0)
            .world
            .network
            .send(GamePacket::RequestRoomChange { id, direction });
        h.tick();

        assert_eq!(h.server_player_coords(p0), (0, 0));
        assert_eq!(h.client_player_coords(p0), (0, 0));

        let (cx, cy) = h.client_player_pos(p0);
        let (sx, sy) = h.server_player_pos(p0);
        assert!(
            (cx - sx).abs() <= 2.0 && (cy - sy).abs() <= 2.0,
            "client ({cx}, {cy}) should snap back to the server ({sx}, {sy}) for {direction:?}"
        );
    }

    assert_eq!(h.client_room_change_requests, 3);
    assert_eq!(h.room_change_responses(p0), 3);
}

/// E3 — a victim cannot dodge death by claiming a room: the spoofed requests
/// are rejected and answered only to the victim, and the shooter still kills it.
#[test]
fn e2e_spoofed_room_cannot_dodge_death() {
    let mut h = TestHarness::new();
    let (shooter, victim) = setup_pvp(&mut h);

    let shooter_id = h.client(shooter).player_id;
    let victim_id = h.client(victim).player_id;

    // Two malicious requests, neither at a door.
    h.client_mut(victim)
        .world
        .network
        .send(GamePacket::RequestRoomChange {
            id: victim_id,
            direction: Direction::North,
        });
    h.client_mut(victim)
        .world
        .network
        .send(GamePacket::RequestRoomChange {
            id: victim_id,
            direction: Direction::East,
        });
    h.tick();

    assert_eq!(h.server_player_coords(victim), (0, 0));
    assert_eq!(h.client_player_coords(victim), (0, 0));

    // Per-requester routing: a broadcast would answer the shooter too.
    assert_eq!(h.room_change_responses(victim), 2);
    assert_eq!(h.room_change_responses(shooter), 0);
    assert_eq!(h.client_room_change_rejects, 2);

    // The victim is never unkillable.
    h.press_shoot(shooter);
    for _ in 0..5 {
        h.tick();
        if !h.last_deaths.is_empty() {
            break;
        }
    }

    assert_eq!(
        h.last_deaths,
        vec![PlayerHitInfo {
            shooter_id,
            victim_id
        }]
    );

    let (vx, vy) = h.client_player_pos(victim);
    assert!(
        (vx - 100.0).abs() <= 2.0 && (vy - 100.0).abs() <= 2.0,
        "victim should respawn near (100, 100), got ({vx}, {vy})"
    );
    assert_eq!(h.client_player_coords(victim), (0, 0));
}

/// E4 — the respawn room survives later client inputs (no `PlayerInput` room
/// field can overwrite it anymore).
#[test]
fn e2e_respawn_room_survives_later_inputs() {
    let mut h = TestHarness::new();
    let (shooter, victim) = setup_pvp(&mut h);

    h.press_shoot(shooter);
    for _ in 0..5 {
        h.tick();
        if !h.last_deaths.is_empty() {
            break;
        }
    }
    assert!(
        !h.last_deaths.is_empty(),
        "expected a death before ticking on"
    );

    h.hold_right(victim, 10);
    h.tick_n(10);

    assert_eq!(h.server_player_coords(victim), (0, 0));
    assert_eq!(h.client_player_coords(victim), (0, 0));
}

/// E5 — an injected rejection snaps the client back to the server position.
///
/// The snap-back target `(760, 321)` sits inside the East raw band, but the
/// reject handler sets `room_change_retry = ROOM_CHANGE_RETRY_TICKS`, which
/// suppresses a same-tick request, so the single-tick assertion is safe.
#[test]
fn e2e_rejection_snaps_client_back() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    place(&mut h, p0, (0, 0), (500.0, 300.0));

    // Move only the server to the authoritative snap-back target.
    h.set_server_player_pos(p0, 760.0, 321.0);
    h.inject_server_packet(
        p0,
        GamePacket::RoomChangeResponse {
            accepted: false,
            level_x: 0,
            level_y: 0,
            x: 760.0,
            y: 321.0,
        },
    );

    h.tick();

    let (x, y) = h.client_player_pos(p0);
    assert!(
        (x - 760.0).abs() <= 2.0 && (y - 321.0).abs() <= 2.0,
        "client should snap back to (760, 321), got ({x}, {y})"
    );
    assert_eq!(h.client_player_coords(p0), (0, 0));
}

/// E6 — the deterministic walk-in: the client leads the server by ~one tick at
/// request time, and the server-only 8 px lead tolerance accepts the first
/// request (zero rejects) with no snap-back loop.
#[test]
fn e2e_walk_in_first_request_accepted() {
    let mut h = TestHarness::new();
    let p0 = h.connect_client();
    place(&mut h, p0, (0, 0), (700.0, 320.0));

    h.hold_right(p0, 25);

    let mut transitioned = false;
    for _ in 0..25 {
        h.tick();
        if h.client_player_coords(p0) == (1, 0) {
            transitioned = true;
            break;
        }
    }
    assert!(transitioned, "walk-in never entered room (1, 0)");

    h.release_all(p0);
    assert_eq!(
        h.client_room_change_rejects, 0,
        "the first request must be accepted by the lead tolerance"
    );

    for _ in 0..3 {
        h.tick();
        assert_ne!(
            h.client_player_coords(p0),
            (0, 0),
            "client returned to (0, 0) after entering (1, 0)"
        );
    }
    assert_eq!(h.client_player_coords(p0), (1, 0));
    assert_eq!(h.server_player_coords(p0), (1, 0));
}
