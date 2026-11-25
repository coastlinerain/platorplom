#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    JoinRequest,
    JoinResponse {
        assigned_id: u8,
    },
    PlayerPos {
        id: u8,
        x: f32,
        y: f32,
        dir: f32,
        level_x: u8,
        level_y: u8,
        last_seq: u16,
    },
    Leave {
        id: u8,
    },
    Hit {
        entity_id: u8,
    },
    PlayerInput {
        id: u8,
        level_x: u8,
        level_y: u8,
        move_left: bool,
        move_right: bool,
        jump: bool,
        shoot: bool,
        seq: u16,
    },
    // --- NEW (Phase 2e) ---
    BulletSpawn {
        bullet_id: u16,
        x: f32,
        y: f32,
        dir: f32,
        owner_id: u8,
        seq: u16,
    },
    EnemyDied {
        enemy_id: u8,
        room_x: u8,
        room_y: u8,
    },
    EnemyStateRequest {
        room_x: u8,
        room_y: u8,
    },
    EnemyStateResponse {
        room_x: u8,
        room_y: u8,
        enemies: Vec<(u8, bool, f32, f32)>, // (enemy_id, alive, spawn_x, spawn_y)
    },
}
