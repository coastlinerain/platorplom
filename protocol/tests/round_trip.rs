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
    });
    round_trip(GamePacket::PlayerPos {
        id: 255,
        x: 0.0,
        y: 0.0,
        dir: 0.0,
        level_x: 255,
        level_y: 255,
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
