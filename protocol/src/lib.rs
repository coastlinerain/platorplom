use game::room_graph::Direction;

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
        // Phase 2f: server clock for client synchronization
        server_tick: u64,
        server_time_ms: u64,
    },
    Leave {
        id: u8,
    },
    PlayerDied {
        id: u8,
    },
    PlayerInput {
        id: u8,
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
    // --- NEW (spec 3, appended to keep existing variant indices stable) ---
    /// Client → server: ask to cross the door on `direction` from the current
    /// room. The server never trusts the id for state lookup.
    RequestRoomChange {
        id: u8,
        direction: Direction,
    },
    /// Server → requesting client: authoritative room + position (the canonical
    /// entry on acceptance, the unchanged state on rejection).
    RoomChangeResponse {
        accepted: bool,
        level_x: u8,
        level_y: u8,
        x: f32,
        y: f32,
    },
}
