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
        GamePacket::Hit { .. } => "Hit",
        GamePacket::BulletSpawn { .. } => "BulletSpawn",
        GamePacket::EnemyDied { .. } => "EnemyDied",
        GamePacket::EnemyStateRequest { .. } => "EnemyStateRequest",
        GamePacket::EnemyStateResponse { .. } => "EnemyStateResponse",
    }
}

/// Returns the player/entity ID if the packet has one.
fn packet_id(p: &GamePacket) -> Option<u8> {
    match p {
        GamePacket::JoinResponse { assigned_id } => Some(*assigned_id),
        GamePacket::PlayerInput { id, .. } => Some(*id),
        GamePacket::PlayerPos { id, .. } => Some(*id),
        GamePacket::Leave { id } => Some(*id),
        GamePacket::Hit { entity_id } => Some(*entity_id),
        GamePacket::BulletSpawn { owner_id, .. } => Some(*owner_id),
        GamePacket::EnemyDied {
            enemy_id,
            room_x: _,
            room_y: _,
        } => Some(*enemy_id),
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
            level_x,
            level_y,
            move_left,
            move_right,
            jump,
            shoot,
            seq,
        } => serde_json::json!({
            "id": id,
            "level": [level_x, level_y],
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
        GamePacket::Hit { entity_id } => serde_json::json!({ "entity_id": entity_id }),
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
        GamePacket::EnemyDied {
            enemy_id,
            room_x,
            room_y,
        } => serde_json::json!({
            "enemy_id": enemy_id,
            "room": [room_x, room_y],
        }),
        GamePacket::EnemyStateRequest { room_x, room_y } => {
            serde_json::json!({ "room": [room_x, room_y] })
        }
        GamePacket::EnemyStateResponse {
            room_x,
            room_y,
            enemies,
        } => serde_json::json!({
            "room": [room_x, room_y],
            "enemy_count": enemies.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                level_x: 0,
                level_y: 0,
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
        assert_eq!(packet_type_name(&GamePacket::Hit { entity_id: 1 }), "Hit");
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
            packet_type_name(&GamePacket::EnemyDied {
                enemy_id: 1,
                room_x: 0,
                room_y: 0
            }),
            "EnemyDied"
        );
        assert_eq!(
            packet_type_name(&GamePacket::EnemyStateRequest {
                room_x: 0,
                room_y: 0
            }),
            "EnemyStateRequest"
        );
        assert_eq!(
            packet_type_name(&GamePacket::EnemyStateResponse {
                room_x: 0,
                room_y: 0,
                enemies: vec![]
            }),
            "EnemyStateResponse"
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
                level_x: 0,
                level_y: 0,
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
        assert_eq!(packet_id(&GamePacket::Hit { entity_id: 9 }), Some(9));
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
            packet_id(&GamePacket::EnemyDied {
                enemy_id: 6,
                room_x: 0,
                room_y: 0
            }),
            Some(6)
        );
        assert_eq!(
            packet_id(&GamePacket::EnemyStateRequest {
                room_x: 0,
                room_y: 0
            }),
            None
        );
        assert_eq!(
            packet_id(&GamePacket::EnemyStateResponse {
                room_x: 0,
                room_y: 0,
                enemies: vec![]
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
                level_x: 0,
                level_y: 0,
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
            level_x: 0,
            level_y: 0,
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
        let _ = packet_key_fields(&GamePacket::Hit { entity_id: 1 });
        let _ = packet_key_fields(&GamePacket::BulletSpawn {
            bullet_id: 1,
            x: 100.0,
            y: 100.0,
            dir: 1.0,
            owner_id: 1,
            seq: 5,
        });
        let _ = packet_key_fields(&GamePacket::EnemyDied {
            enemy_id: 3,
            room_x: 0,
            room_y: 1,
        });
        let _ = packet_key_fields(&GamePacket::EnemyStateRequest {
            room_x: 0,
            room_y: 0,
        });
        let _ = packet_key_fields(&GamePacket::EnemyStateResponse {
            room_x: 0,
            room_y: 0,
            enemies: vec![(1, true, 100.0, 100.0)],
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
