use game::entities::player::{Player, PlayerInput};
use game::level::Level;
use game::maps::Map;
use std::collections::HashMap;

#[derive(Debug)]
pub struct BulletSpawnInfo {
    pub bullet_id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: f32,
    pub owner_id: u8,
    pub seq: u16,
}

pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords: HashMap<u8, (u8, u8)>,
    levels: Vec<Vec<Level>>,
    max_seq: HashMap<u8, u16>,
    shoot_cooldown: HashMap<u8, u8>,
    next_bullet_id: u16,
    // NEW: Phase 2f - server clock for synchronization
    current_tick: u64,
    start_time: std::time::Instant,
}

impl ServerWorld {
    pub fn new() -> Self {
        let map_data = Map::new();

        let levels: Vec<Vec<Level>> = map_data
            .grids
            .into_iter()
            .zip(map_data.data)
            .map(|(grid_row, data_row)| {
                grid_row
                    .into_iter()
                    .zip(data_row)
                    .map(|(grid, data)| Level::new(grid, data))
                    .collect()
            })
            .collect();

        Self {
            players: HashMap::new(),
            pending: HashMap::new(),
            coords: HashMap::new(),
            levels,
            max_seq: HashMap::new(),
            shoot_cooldown: HashMap::new(),
            next_bullet_id: 0,
            current_tick: 0,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn spawn(&mut self, id: u8) {
        self.players.insert(id, Player::new(id));
        self.pending.insert(id, PlayerInput::default());
        self.coords.insert(id, (0, 0));
        self.max_seq.insert(id, 0);
    }

    pub fn remove(&mut self, id: u8) {
        self.players.remove(&id);
        self.pending.remove(&id);
        self.coords.remove(&id);
        self.max_seq.remove(&id);
    }

    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16) {
        // coords: (level_x, level_y) = (column, row)
        self.coords.insert(id, (x, y));

        let pending = self.pending.entry(id).or_default();

        // Held flags overwrite (latest-wins)
        pending.move_left = input.move_left;
        pending.move_right = input.move_right;

        // Edge flags OR-accumulate (latched until consumed by update)
        pending.jump |= input.jump;
        pending.shoot |= input.shoot;

        // Track the highest received sequence number.
        let current = self.max_seq.entry(id).or_insert(0);
        if game::is_newer_seq(seq, *current) {
            *current = seq;
        }
    }

    /// Runs one authoritative tick.
    /// Returns the `BulletSpawnInfo`s for accepted shots this tick.
    pub fn update(&mut self, dt: f32) -> Vec<BulletSpawnInfo> {
        let mut bullet_spawns = Vec::new();

        // First pass: update all players, collect shoot requests
        struct ShootRequest {
            player_id: u8,
            spawn_x: f32,
            spawn_y: f32,
            dir: f32,
            player_seq: u16,
        }
        let mut shoot_requests = Vec::new();

        for (&id, player) in &mut self.players {
            let (cx, cy) = self.coords.get(&id).copied().unwrap_or((0, 0));

            // Row-major: levels[row_y][col_x]. Client-trusted coords use `.get()`
            // fallback so a bogus (255,255) does not index-panic the server.
            let level = self
                .levels
                .get(cy as usize)
                .and_then(|row| row.get(cx as usize))
                .unwrap_or(&self.levels[0][0]);

            let pending = self.pending.entry(id).or_default();

            // Build the input for this tick: held flags from the accumulator,
            // jump from the accumulator, shoot forced false (server never spawns
            // bullets — D6).
            let input = PlayerInput {
                move_left: pending.move_left,
                move_right: pending.move_right,
                jump: pending.jump,
                shoot: false,
            };

            player.update(dt, level, &input);

            // Clear latched edge flags now that they've been consumed.
            pending.jump = false;

            // If shoot was latched and cooldown is expired, queue for processing.
            let cd = self.shoot_cooldown.entry(id).or_insert(0);
            if pending.shoot && *cd == 0 {
                let player_seq = self.max_seq.get(&id).copied().unwrap_or(0);

                shoot_requests.push(ShootRequest {
                    player_id: id,
                    spawn_x: player.pos.x + player.w / 2.0,
                    spawn_y: player.pos.y + player.h / 2.0,
                    dir: player.dir,
                    player_seq,
                });

                *cd = 5; // ~83 ms cooldown at 60 Hz
            }
            if *cd > 0 {
                *cd -= 1;
            }
            pending.shoot = false;
        }

        // Second pass: assign bullet IDs and emit spawn info.
        // There is no hit-detection target after the PvE removal (spec 1); the
        // next spec re-adds server-authoritative PvP hit detection.
        for req in shoot_requests {
            let bullet_id = self.next_bullet_id;
            self.next_bullet_id = self.next_bullet_id.wrapping_add(1);
            bullet_spawns.push(BulletSpawnInfo {
                bullet_id,
                x: req.spawn_x,
                y: req.spawn_y,
                dir: req.dir,
                owner_id: req.player_id,
                seq: req.player_seq,
            });
        }

        self.current_tick += 1;
        bullet_spawns
    }

    /// Get current server tick number.
    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// Get elapsed time since server start in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Get current wall-clock time in milliseconds since UNIX_EPOCH.
    /// Used for clock synchronization with clients (NTP-style offset).
    pub fn wall_time_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    pub fn players(&self) -> &HashMap<u8, Player> {
        &self.players
    }

    pub fn players_mut(&mut self) -> &mut HashMap<u8, Player> {
        &mut self.players
    }

    pub fn coords(&self) -> &HashMap<u8, (u8, u8)> {
        &self.coords
    }

    pub fn max_seq(&self) -> &HashMap<u8, u16> {
        &self.max_seq
    }
}

impl Default for ServerWorld {
    fn default() -> Self {
        Self::new()
    }
}

// NOTE (tester, Phase 2b): the tests below are written against the plan's
// revived `ServerWorld` public API (`new` / `spawn` / `apply_input` / `update` /
// `players` / `coords`) plus the private `pending` accumulator. They do NOT
// compile until the implementer lands Box 2 (ServerWorld skeleton + `mod world;`)
// — the current file is an orphaned stub (`iter::Map`, undefined `PlayerData`).
#[cfg(test)]
mod tests {
    use super::ServerWorld;
    use game::config::TICK_HZ;
    use game::entities::player::PlayerInput;

    fn tick_dt() -> f32 {
        1.0 / TICK_HZ as f32
    }

    #[test]
    fn input_held_flags_latest_wins() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_left: true,
                ..Default::default()
            },
            0,
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        // Held flags overwrite: the latest input (move_right) wins.
        assert!(world.pending[&0].move_right);
        assert!(!world.pending[&0].move_left);

        // Latest-wins holds through the tick: the authoritative player moves right.
        world.update(tick_dt());
        assert!(world.players()[&0].pos.x > 100.0);
    }

    #[test]
    fn input_edge_flags_latch_and_clear() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );

        // Two shoot edges OR-latch into a single shot: exactly one is consumed.
        let shots = world.update(tick_dt());
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].owner_id, 0);
        assert_eq!(shots[0].dir, 1.0);

        // The latch is cleared after the tick consumes it.
        assert!(!world.pending[&0].shoot);

        // No re-fire on the next tick without a fresh input.
        let shots2 = world.update(tick_dt());
        assert!(shots2.is_empty());
    }

    #[test]
    fn input_edge_flag_not_lost_between_ticks() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Tick 1: no inputs yet.
        world.update(tick_dt());

        // The shoot edge arrives between ticks.
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );

        // Tick 2: the latched edge fires instead of being dropped.
        let shots = world.update(tick_dt());
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].owner_id, 0);
        assert_eq!(shots[0].dir, 1.0);
    }

    #[test]
    fn apply_input_update_drives_authoritative_position() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        assert!(world.players()[&0].pos.x > 100.0);
        assert_eq!(world.players()[&0].dir, 1.0);
    }

    #[test]
    fn apply_input_update_jump_moves_up_when_grounded() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Set grounded so the jump is accepted (Player::update only jumps when grounded).
        world.players.get_mut(&0).unwrap().grounded = true;

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                jump: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        // Net vertical velocity (SALTO_FUERZA + GRAVEDAD) * dt is negative.
        assert!(world.players()[&0].pos.y < 100.0);
    }

    #[test]
    fn apply_input_out_of_range_coords_fall_back_to_origin() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // A client-reported (255,255) room must not index-panic; it falls back
        // to the origin level where the (100,100) spawn is in-bounds.
        world.apply_input(
            0,
            255,
            255,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        assert!(world.players()[&0].pos.x > 100.0);
    }

    // --- max_seq tests (Phase 2c) ---

    /// AC 28 — max_seq is updated on apply_input with wrapping-aware comparison.
    /// Older seq values are ignored; newer seq values overwrite.
    #[test]
    fn max_seq_updated_on_apply_input() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Initial state: max_seq is 0 for a newly-spawned player.
        assert_eq!(world.max_seq().get(&0).copied().unwrap_or(0), 0);

        // Apply input with seq 5.
        world.apply_input(0, 0, 0, PlayerInput::default(), 5);
        assert_eq!(world.max_seq()[&0], 5);

        // Older seq is ignored.
        world.apply_input(0, 0, 0, PlayerInput::default(), 3);
        assert_eq!(world.max_seq()[&0], 5);

        // Newer seq overwrites.
        world.apply_input(0, 0, 0, PlayerInput::default(), 10);
        assert_eq!(world.max_seq()[&0], 10);
    }

    /// AC 29 — max_seq starts at 0 for newly-spawned players.
    /// The server reports 0 for players who have not sent any inputs yet.
    #[test]
    fn playerpos_broadcast_includes_last_seq() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // New player has max_seq == 0.
        assert_eq!(world.max_seq().get(&0).copied().unwrap_or(0), 0);

        // After applying an input with seq 7, max_seq is updated.
        world.apply_input(0, 0, 0, PlayerInput::default(), 7);
        assert_eq!(world.max_seq()[&0], 7);

        // A second player starts at 0.
        world.spawn(1);
        assert_eq!(world.max_seq().get(&1).copied().unwrap_or(0), 0);
    }
}

/// AC 9 — Confirmed shoot with cooldown=0 emits BulletSpawnInfo with
/// incrementing bullet_id, correct spawn pos from player center, dir, owner_id, input seq.
#[test]
fn shoot_emits_bulletspawninfo() {
    let mut world = ServerWorld::new();
    world.spawn(0);

    // Position player
    {
        let player = world.players.get_mut(&0).unwrap();
        player.pos = glam::Vec2::new(100.0, 100.0);
        player.dir = 1.0;
        player.w = 30.0;
        player.h = 38.0;
        player.grounded = true;
    }
    world.coords.insert(0, (0, 0));

    // Apply shoot input with seq=5
    world.apply_input(
        0,
        0,
        0,
        PlayerInput {
            shoot: true,
            ..Default::default()
        },
        5,
    );

    let shots = world.update(1.0 / 60.0);

    assert_eq!(shots.len(), 1);
    let shot = &shots[0];
    assert_eq!(shot.bullet_id, 0); // First bullet_id
    assert_eq!(shot.dir, 1.0); // Player dir
    assert_eq!(shot.owner_id, 0);
    assert_eq!(shot.seq, 5); // Input seq
    // Spawn pos should be player center
    assert!((shot.x - 115.0).abs() < 1.0); // 100 + 30/2 = 115
    assert!((shot.y - 119.0).abs() < 1.0); // 100 + 38/2 = 119
}

/// AC 12 — Two shoot inputs within 5 ticks → only first produces BulletSpawnInfo.
/// Cooldown is 5 ticks: after firing, cd=5, then decrements each tick.
/// Tick 1: fire, cd=5→4; Tick 2: cd=4→3; Tick 3: cd=3→2; Tick 4: cd=2→1; Tick 5: cd=1→0;
/// Tick 6: cd=0, can fire again.
#[test]
fn shoot_cooldown_enforced() {
    let mut world = ServerWorld::new();
    world.spawn(0);

    // Position player
    {
        let player = world.players.get_mut(&0).unwrap();
        player.pos = glam::Vec2::new(100.0, 100.0);
        player.dir = 1.0;
        player.w = 30.0;
        player.h = 38.0;
        player.grounded = true;
    }
    world.coords.insert(0, (0, 0));

    // Tick 1: shoot
    world.apply_input(
        0,
        0,
        0,
        PlayerInput {
            shoot: true,
            ..Default::default()
        },
        1,
    );
    let shots1 = world.update(1.0 / 60.0);
    assert_eq!(shots1.len(), 1);
    assert_eq!(shots1[0].bullet_id, 0);

    // Tick 2-5: no input, cooldown counts down (4 ticks to go from 4 to 0)
    for _ in 0..4 {
        let shots = world.update(1.0 / 60.0);
        assert!(shots.is_empty());
    }

    // Tick 6: cooldown expired, new shoot input
    world.apply_input(
        0,
        0,
        0,
        PlayerInput {
            shoot: true,
            ..Default::default()
        },
        5,
    );
    let shots6 = world.update(1.0 / 60.0);
    assert_eq!(shots6.len(), 1);
    assert_eq!(shots6[0].bullet_id, 1); // Incremented
}
