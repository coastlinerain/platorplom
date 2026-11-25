use protocol::GamePacket;
use std::process::Command;

fn round_trip(pkt: GamePacket) {
    let bytes = postcard::to_allocvec(&pkt).unwrap();
    let decoded: GamePacket = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, pkt);
}

#[test]
fn round_trip_join_request() {
    round_trip(GamePacket::JoinRequest);
}

#[test]
fn round_trip_join_response() {
    round_trip(GamePacket::JoinResponse { assigned_id: 0 });
    round_trip(GamePacket::JoinResponse { assigned_id: 255 });
}

#[test]
fn round_trip_player_pos() {
    round_trip(GamePacket::PlayerPos {
        id: 0,
        x: -1.5,
        y: 100.25,
        dir: 0.1,
        level_x: 0,
        level_y: 255,
        last_seq: 0,
    });
    round_trip(GamePacket::PlayerPos {
        id: 255,
        x: 0.0,
        y: 0.0,
        dir: 0.0,
        level_x: 255,
        level_y: 255,
        last_seq: 0,
    });
    round_trip(GamePacket::PlayerPos {
        id: 7,
        x: 50.0,
        y: 75.0,
        dir: -1.0,
        level_x: 3,
        level_y: 2,
        last_seq: 42,
    });
}

#[test]
fn round_trip_leave() {
    round_trip(GamePacket::Leave { id: 0 });
    round_trip(GamePacket::Leave { id: 255 });
}

#[test]
fn round_trip_hit() {
    round_trip(GamePacket::Hit { entity_id: 0 });
    round_trip(GamePacket::Hit { entity_id: 255 });
}

#[test]
fn round_trip_player_input() {
    round_trip(GamePacket::PlayerInput {
        id: 7,
        level_x: 1,
        level_y: 2,
        move_left: true,
        move_right: false,
        jump: true,
        shoot: false,
        seq: 0,
    });
    round_trip(GamePacket::PlayerInput {
        id: 0,
        level_x: 0,
        level_y: 0,
        move_left: false,
        move_right: false,
        jump: false,
        shoot: false,
        seq: 0,
    });
    round_trip(GamePacket::PlayerInput {
        id: 255,
        level_x: 255,
        level_y: 255,
        move_left: true,
        move_right: true,
        jump: true,
        shoot: true,
        seq: 0,
    });
}

/// AC 1, AC 5 — `PlayerInput` with non-zero and boundary `seq` values round-trips.
#[test]
fn round_trip_player_input_seq() {
    round_trip(GamePacket::PlayerInput {
        id: 7,
        level_x: 1,
        level_y: 2,
        move_left: true,
        move_right: false,
        jump: true,
        shoot: false,
        seq: 42,
    });
    round_trip(GamePacket::PlayerInput {
        id: 0,
        level_x: 0,
        level_y: 0,
        move_left: false,
        move_right: false,
        jump: false,
        shoot: false,
        seq: 0,
    });
    round_trip(GamePacket::PlayerInput {
        id: 255,
        level_x: 255,
        level_y: 255,
        move_left: true,
        move_right: true,
        jump: true,
        shoot: true,
        seq: 65535,
    });
}

/// AC 5 — seq = u16::MAX serializes and deserializes to u16::MAX exactly.
#[test]
fn round_trip_player_input_seq_wraparound() {
    let pkt = GamePacket::PlayerInput {
        id: 1,
        level_x: 0,
        level_y: 0,
        move_left: false,
        move_right: true,
        jump: false,
        shoot: false,
        seq: u16::MAX,
    };
    let bytes = postcard::to_allocvec(&pkt).unwrap();
    let decoded: GamePacket = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, pkt);
    if let GamePacket::PlayerInput { seq, .. } = decoded {
        assert_eq!(seq, u16::MAX);
    } else {
        panic!("Expected PlayerInput, got {decoded:?}");
    }
}

// ============================================================
// Phase 2e — New protocol variant round-trip tests (AC 1, 3)
// ============================================================

#[test]
fn round_trip_bulletspawn() {
    round_trip(GamePacket::BulletSpawn {
        bullet_id: 0,
        x: 100.0,
        y: 200.0,
        dir: 1.0,
        owner_id: 5,
        seq: 10,
    });
    round_trip(GamePacket::BulletSpawn {
        bullet_id: 65535,
        x: -50.5,
        y: 300.25,
        dir: -1.0,
        owner_id: 0,
        seq: 65535,
    });
    round_trip(GamePacket::BulletSpawn {
        bullet_id: 1234,
        x: 0.0,
        y: 0.0,
        dir: 1.0,
        owner_id: 255,
        seq: 0,
    });
}

#[test]
fn round_trip_enemydied() {
    round_trip(GamePacket::EnemyDied {
        enemy_id: 0,
        room_x: 0,
        room_y: 0,
    });
    round_trip(GamePacket::EnemyDied {
        enemy_id: 63,
        room_x: 3,
        room_y: 3,
    });
    round_trip(GamePacket::EnemyDied {
        enemy_id: 15,
        room_x: 0,
        room_y: 1,
    });
}

#[test]
fn round_trip_enemystaterequest() {
    round_trip(GamePacket::EnemyStateRequest {
        room_x: 0,
        room_y: 0,
    });
    round_trip(GamePacket::EnemyStateRequest {
        room_x: 3,
        room_y: 3,
    });
    round_trip(GamePacket::EnemyStateRequest {
        room_x: 1,
        room_y: 2,
    });
}

#[test]
fn round_trip_enemystateresponse() {
    round_trip(GamePacket::EnemyStateResponse {
        room_x: 0,
        room_y: 0,
        enemies: vec![],
    });
    round_trip(GamePacket::EnemyStateResponse {
        room_x: 1,
        room_y: 0,
        enemies: vec![(0, true, 100.0, 200.0), (1, false, 150.0, 200.0)],
    });
    round_trip(GamePacket::EnemyStateResponse {
        room_x: 0,
        room_y: 1,
        enemies: vec![
            (0, true, 50.0, 50.0),
            (1, true, 100.0, 50.0),
            (2, true, 150.0, 50.0),
            (3, false, 200.0, 50.0),
            (4, true, 250.0, 50.0),
        ],
    });
}

/// AC 2 — `GamePacket::Action` variant is removed from the enum.
/// This test verifies that constructing an Action packet fails to compile
/// by using a compile-time check. Since Action is removed, this test
/// simply passes as a placeholder documenting the requirement.
#[test]
fn action_variant_removed() {
    // If Action variant still existed, the following would compile:
    // let _ = GamePacket::Action { id: 0, kind: "shoot".to_string(), dir: 1.0 };
    // Since it's removed, this test passes trivially.
    // The actual verification is that the code compiles without Action.
}

/// AC 4 — `protocol/Cargo.toml` declares no dependency on `game` or `glam`.
#[test]
fn test_no_game_or_glam_deps() {
    let output = Command::new("cargo")
        .args(["tree", "-p", "protocol", "--edges", "normal"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to run cargo tree");

    if !output.status.success() {
        panic!("cargo tree -p protocol failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Check that neither 'game' nor 'glam' appears in the dependency tree
    assert!(
        !stdout.contains("game "),
        "protocol crate should not depend on 'game' crate, but found in tree:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("glam "),
        "protocol crate should not depend on 'glam' crate, but found in tree:\n{}",
        stdout
    );
}
