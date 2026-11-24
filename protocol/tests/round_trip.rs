use protocol::GamePacket;

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
fn round_trip_action() {
    round_trip(GamePacket::Action {
        id: 7,
        kind: "☃é🔥".to_string(),
        dir: -0.5,
    });
    round_trip(GamePacket::Action {
        id: 0,
        kind: String::new(),
        dir: 0.0,
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
    // Complement bits / extremes: prove every bool and byte survives postcard.
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
    // Non-zero seq.
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
    // seq = 0 (backward compat).
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
    // seq = u16::MAX (wraparound boundary).
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
