use protocol::GamePacket;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// Log a packet as JSONL to stdout if `RUSTVANIA_PACKET_LOG` env var is set.
pub fn log_packet(dir: &str, packet: &GamePacket) {
    if std::env::var("RUSTVANIA_PACKET_LOG").is_err() {
        return;
    }
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let entry = serde_json::json!({
        "ts_ms": ts_ms,
        "dir": dir,
        "type": packet_type_name(packet),
        "id": packet_id(packet),
        "seq": packet_seq(packet),
        "server_tick": packet_server_tick(packet),
        "server_time_ms": packet_server_time_ms(packet),
        "key_fields": packet_key_fields(packet),
    });
    println!("{}", entry);
}

/// Returns the variant name of the packet.
fn packet_type_name(p: &GamePacket) -> &'static str {
    match p {
        GamePacket::JoinRequest => "JoinRequest",
        GamePacket::JoinResponse { .. } => "JoinResponse",
        GamePacket::PlayerInput { .. } => "PlayerInput",
        GamePacket::PlayerPos { .. } => "PlayerPos",
        GamePacket::Leave { .. } => "Leave",
        GamePacket::PlayerDied { .. } => "PlayerDied",
        GamePacket::BulletSpawn { .. } => "BulletSpawn",
        GamePacket::RequestRoomChange { .. } => "RequestRoomChange",
        GamePacket::RoomChangeResponse { .. } => "RoomChangeResponse",
    }
}

/// Returns the player/entity ID if the packet has one.
fn packet_id(p: &GamePacket) -> Option<u8> {
    match p {
        GamePacket::JoinResponse { assigned_id } => Some(*assigned_id),
        GamePacket::PlayerInput { id, .. } => Some(*id),
        GamePacket::PlayerPos { id, .. } => Some(*id),
        GamePacket::Leave { id } => Some(*id),
        GamePacket::PlayerDied { id } => Some(*id),
        GamePacket::BulletSpawn { owner_id, .. } => Some(*owner_id),
        GamePacket::RequestRoomChange { id, .. } => Some(*id),
        _ => None,
    }
}

/// Returns the sequence number if the packet has one.
fn packet_seq(p: &GamePacket) -> Option<u16> {
    match p {
        GamePacket::PlayerInput { seq, .. } => Some(*seq),
        GamePacket::PlayerPos { last_seq, .. } => Some(*last_seq),
        GamePacket::BulletSpawn { seq, .. } => Some(*seq),
        _ => None,
    }
}

/// Returns the server_tick if the packet has one.
fn packet_server_tick(p: &GamePacket) -> Option<u64> {
    match p {
        GamePacket::PlayerPos { server_tick, .. } => Some(*server_tick),
        _ => None,
    }
}

/// Returns the server_time_ms if the packet has one.
fn packet_server_time_ms(p: &GamePacket) -> Option<u64> {
    match p {
        GamePacket::PlayerPos { server_time_ms, .. } => Some(*server_time_ms),
        _ => None,
    }
}

/// Returns abbreviated key fields for the packet as a JSON value.
pub fn packet_key_fields(p: &GamePacket) -> Value {
    match p {
        GamePacket::JoinRequest => serde_json::json!({}),
        GamePacket::JoinResponse { assigned_id } => {
            serde_json::json!({ "assigned_id": assigned_id })
        }
        GamePacket::PlayerInput {
            id,
            move_left,
            move_right,
            jump,
            shoot,
            seq,
        } => serde_json::json!({
            "id": id,
            "move_left": move_left,
            "move_right": move_right,
            "jump": jump,
            "shoot": shoot,
            "seq": seq,
        }),
        GamePacket::PlayerPos {
            id,
            x,
            y,
            dir,
            level_x,
            level_y,
            last_seq,
            server_tick,
            server_time_ms,
        } => serde_json::json!({
            "id": id,
            "pos": [x, y],
            "dir": dir,
            "level": [level_x, level_y],
            "last_seq": last_seq,
            "server_tick": server_tick,
            "server_time_ms": server_time_ms,
        }),
        GamePacket::Leave { id } => serde_json::json!({ "id": id }),
        GamePacket::PlayerDied { id } => serde_json::json!({ "id": id }),
        GamePacket::BulletSpawn {
            bullet_id,
            x,
            y,
            dir,
            owner_id,
            seq,
        } => serde_json::json!({
            "bullet_id": bullet_id,
            "pos": [x, y],
            "dir": dir,
            "owner_id": owner_id,
            "seq": seq,
        }),
        GamePacket::RequestRoomChange { id, direction } => {
            serde_json::json!({ "id": id, "direction": direction })
        }
        GamePacket::RoomChangeResponse {
            accepted,
            level_x,
            level_y,
            x,
            y,
        } => serde_json::json!({
            "accepted": accepted,
            "level": [level_x, level_y],
            "pos": [x, y],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game::room_graph::Direction;
    use protocol::GamePacket;

    #[test]
    fn packet_type_name_all_variants() {
        assert_eq!(packet_type_name(&GamePacket::JoinRequest), "JoinRequest");
        assert_eq!(
            packet_type_name(&GamePacket::JoinResponse { assigned_id: 1 }),
            "JoinResponse"
        );
        assert_eq!(
            packet_type_name(&GamePacket::PlayerInput {
                id: 1,
                move_left: false,
                move_right: false,
                jump: false,
                shoot: false,
                seq: 0
            }),
            "PlayerInput"
        );
        assert_eq!(
            packet_type_name(&GamePacket::PlayerPos {
                id: 1,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                level_x: 0,
                level_y: 0,
                last_seq: 0,
                server_tick: 0,
                server_time_ms: 0
            }),
            "PlayerPos"
        );
        assert_eq!(packet_type_name(&GamePacket::Leave { id: 1 }), "Leave");
        assert_eq!(
            packet_type_name(&GamePacket::PlayerDied { id: 1 }),
            "PlayerDied"
        );
        assert_eq!(
            packet_type_name(&GamePacket::BulletSpawn {
                bullet_id: 1,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                owner_id: 1,
                seq: 0
            }),
            "BulletSpawn"
        );
        assert_eq!(
            packet_type_name(&GamePacket::RequestRoomChange {
                id: 1,
                direction: Direction::East
            }),
            "RequestRoomChange"
        );
        assert_eq!(
            packet_type_name(&GamePacket::RoomChangeResponse {
                accepted: false,
                level_x: 0,
                level_y: 0,
                x: 0.0,
                y: 0.0
            }),
            "RoomChangeResponse"
        );
    }

    #[test]
    fn packet_id_all_variants() {
        assert_eq!(packet_id(&GamePacket::JoinRequest), None);
        assert_eq!(
            packet_id(&GamePacket::JoinResponse { assigned_id: 5 }),
            Some(5)
        );
        assert_eq!(
            packet_id(&GamePacket::PlayerInput {
                id: 3,
                move_left: false,
                move_right: false,
                jump: false,
                shoot: false,
                seq: 0
            }),
            Some(3)
        );
        assert_eq!(
            packet_id(&GamePacket::PlayerPos {
                id: 7,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                level_x: 0,
                level_y: 0,
                last_seq: 0,
                server_tick: 0,
                server_time_ms: 0
            }),
            Some(7)
        );
        assert_eq!(packet_id(&GamePacket::Leave { id: 2 }), Some(2));
        assert_eq!(packet_id(&GamePacket::PlayerDied { id: 9 }), Some(9));
        assert_eq!(
            packet_id(&GamePacket::BulletSpawn {
                bullet_id: 1,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                owner_id: 4,
                seq: 0
            }),
            Some(4)
        );
        assert_eq!(
            packet_id(&GamePacket::RequestRoomChange {
                id: 6,
                direction: Direction::South
            }),
            Some(6)
        );
        assert_eq!(
            packet_id(&GamePacket::RoomChangeResponse {
                accepted: false,
                level_x: 0,
                level_y: 0,
                x: 0.0,
                y: 0.0
            }),
            None
        );
    }

    #[test]
    fn packet_seq_all_variants() {
        assert_eq!(packet_seq(&GamePacket::JoinRequest), None);
        assert_eq!(
            packet_seq(&GamePacket::JoinResponse { assigned_id: 0 }),
            None
        );
        assert_eq!(
            packet_seq(&GamePacket::PlayerInput {
                id: 0,
                move_left: false,
                move_right: false,
                jump: false,
                shoot: false,
                seq: 42
            }),
            Some(42)
        );
        assert_eq!(
            packet_seq(&GamePacket::PlayerPos {
                id: 0,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                level_x: 0,
                level_y: 0,
                last_seq: 99,
                server_tick: 0,
                server_time_ms: 0
            }),
            Some(99)
        );
        assert_eq!(
            packet_seq(&GamePacket::BulletSpawn {
                bullet_id: 0,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                owner_id: 0,
                seq: 123
            }),
            Some(123)
        );
        assert_eq!(packet_seq(&GamePacket::Leave { id: 0 }), None);
        assert_eq!(
            packet_seq(&GamePacket::RequestRoomChange {
                id: 0,
                direction: Direction::North
            }),
            None
        );
        assert_eq!(
            packet_seq(&GamePacket::RoomChangeResponse {
                accepted: true,
                level_x: 1,
                level_y: 0,
                x: 10.0,
                y: 321.0
            }),
            None
        );
    }

    #[test]
    fn packet_server_tick_only_playerpos() {
        assert_eq!(
            packet_server_tick(&GamePacket::PlayerPos {
                id: 0,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                level_x: 0,
                level_y: 0,
                last_seq: 0,
                server_tick: 1000,
                server_time_ms: 0
            }),
            Some(1000)
        );
        assert_eq!(packet_server_tick(&GamePacket::JoinRequest), None);
        assert_eq!(
            packet_server_tick(&GamePacket::BulletSpawn {
                bullet_id: 0,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                owner_id: 0,
                seq: 0
            }),
            None
        );
    }

    #[test]
    fn packet_server_time_ms_only_playerpos() {
        assert_eq!(
            packet_server_time_ms(&GamePacket::PlayerPos {
                id: 0,
                x: 0.0,
                y: 0.0,
                dir: 1.0,
                level_x: 0,
                level_y: 0,
                last_seq: 0,
                server_tick: 0,
                server_time_ms: 1234567890
            }),
            Some(1234567890)
        );
        assert_eq!(packet_server_time_ms(&GamePacket::JoinRequest), None);
    }

    #[test]
    fn packet_key_fields_all_variants() {
        // Just verify they serialize without panic
        let _ = packet_key_fields(&GamePacket::JoinRequest);
        let _ = packet_key_fields(&GamePacket::JoinResponse { assigned_id: 1 });
        let _ = packet_key_fields(&GamePacket::PlayerInput {
            id: 1,
            move_left: true,
            move_right: false,
            jump: true,
            shoot: false,
            seq: 5,
        });
        let _ = packet_key_fields(&GamePacket::PlayerPos {
            id: 1,
            x: 100.0,
            y: 200.0,
            dir: -1.0,
            level_x: 0,
            level_y: 1,
            last_seq: 10,
            server_tick: 500,
            server_time_ms: 1234567890,
        });
        let _ = packet_key_fields(&GamePacket::Leave { id: 1 });
        let _ = packet_key_fields(&GamePacket::PlayerDied { id: 1 });
        let _ = packet_key_fields(&GamePacket::BulletSpawn {
            bullet_id: 1,
            x: 100.0,
            y: 100.0,
            dir: 1.0,
            owner_id: 1,
            seq: 5,
        });
        let _ = packet_key_fields(&GamePacket::RequestRoomChange {
            id: 1,
            direction: Direction::East,
        });
        let _ = packet_key_fields(&GamePacket::RoomChangeResponse {
            accepted: true,
            level_x: 1,
            level_y: 0,
            x: 10.0,
            y: 321.0,
        });
    }

    #[test]
    fn log_packet_disabled_by_default() {
        // Ensure env var is not set
        unsafe {
            std::env::remove_var("RUSTVANIA_PACKET_LOG");
        }
        // Should not panic and not print
        log_packet("sent", &GamePacket::JoinRequest);
        log_packet("recv", &GamePacket::JoinResponse { assigned_id: 1 });
    }
}
