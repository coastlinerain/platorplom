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
    },
    Action {
        id: u8,
        kind: String,
        dir: f32,
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
    },
}
