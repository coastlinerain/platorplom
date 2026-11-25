use game::config::TILE_SIZE;
use game::entities::player::{Player, PlayerInput};
use game::level::Level;
use game::maps::Map;
use game::types::EntityType;
use std::collections::HashMap;

#[derive(Debug)]
pub struct EnemyState {
    pub id: u8,
    pub alive: bool,
    pub spawn_pos: (f32, f32), // tile center (x * TILE_SIZE + TILE_SIZE/2, y * TILE_SIZE + TILE_SIZE/2)
}

#[derive(Debug)]
pub struct BulletSpawnInfo {
    pub bullet_id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: f32,
    pub owner_id: u8,
    pub seq: u16,
}

#[derive(Debug)]
pub struct EnemyDiedInfo {
    pub enemy_id: u8,
    pub room_x: u8,
    pub room_y: u8,
}

pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords: HashMap<u8, (u8, u8)>,
    levels: Vec<Vec<Level>>,
    max_seq: HashMap<u8, u16>,
    shoot_cooldown: HashMap<u8, u8>,
    // NEW: Phase 2e
    enemies: HashMap<(u8, u8), Vec<EnemyState>>, // key: (room_x, room_y)
    next_bullet_id: u16,
}

impl ServerWorld {
    pub fn new() -> Self {
        let map_data = Map::new();

        // Build enemy state for all rooms BEFORE consuming map_data (Phase 2e)
        let mut enemies: HashMap<(u8, u8), Vec<EnemyState>> = HashMap::new();
        // Iterate in row-major room order: room_y outer, room_x inner
        for room_y in 0..2 {
            for room_x in 0..2 {
                let room_enemies = Self::build_enemies_for_room(&map_data, room_x, room_y);
                enemies.insert((room_x, room_y), room_enemies);
            }
        }

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
            enemies,
            next_bullet_id: 0,
        }
    }

    /// Build enemy state for a single room from map data.
    /// Iterates in row-major order (y then x) within the room.
    fn build_enemies_for_room(map_data: &Map, room_x: u8, room_y: u8) -> Vec<EnemyState> {
        let mut room_enemies = Vec::new();
        let mut spawn_index = 0u8;

        let room_data = &map_data.data[room_y as usize][room_x as usize];

        for (y, row) in room_data.iter().enumerate() {
            for (x, &cell) in row.iter().enumerate() {
                if matches!(cell, EntityType::EnemySpawn | EntityType::PotionSpawn) {
                    let spawn_x = x as f32 * TILE_SIZE + TILE_SIZE / 2.0;
                    let spawn_y = y as f32 * TILE_SIZE + TILE_SIZE / 2.0;
                    let id = room_x * 16 + spawn_index;
                    room_enemies.push(EnemyState {
                        id,
                        alive: true,
                        spawn_pos: (spawn_x, spawn_y),
                    });
                    spawn_index += 1;
                }
            }
        }

        room_enemies
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
    /// Returns (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>) for shots and enemy deaths this tick.
    pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>) {
        let mut bullet_spawns = Vec::new();
        let mut enemy_deaths = Vec::new();

        // First pass: update all players, collect shoot requests
        struct ShootRequest {
            player_id: u8,
            spawn_x: f32,
            spawn_y: f32,
            dir: f32,
            room_coords: (u8, u8),
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
                let room_coords = self.coords.get(&id).copied().unwrap_or((0, 0));
                let player_seq = self.max_seq.get(&id).copied().unwrap_or(0);

                shoot_requests.push(ShootRequest {
                    player_id: id,
                    spawn_x: player.pos.x + player.w / 2.0,
                    spawn_y: player.pos.y + player.h / 2.0,
                    dir: player.dir,
                    room_coords,
                    player_seq,
                });

                *cd = 5; // ~83 ms cooldown at 60 Hz
            }
            if *cd > 0 {
                *cd -= 1;
            }
            pending.shoot = false;
        }

        // Second pass: process shoot requests (raycast, emit packets)
        for req in shoot_requests {
            let (hit_spawn_x, hit_spawn_y, hit_enemy_id) =
                self.raycast_hit(req.spawn_x, req.spawn_y, req.dir, req.room_coords);

            let bullet_id = self.next_bullet_id;
            self.next_bullet_id = self.next_bullet_id.wrapping_add(1);
            bullet_spawns.push(BulletSpawnInfo {
                bullet_id,
                x: hit_spawn_x,
                y: hit_spawn_y,
                dir: req.dir,
                owner_id: req.player_id,
                seq: req.player_seq,
            });

            // If hit an enemy, mark dead and emit EnemyDiedInfo
            if let Some(enemy_id) = hit_enemy_id
                && let Some(room_enemies) = self.enemies.get_mut(&req.room_coords)
                && let Some(enemy) = room_enemies.iter_mut().find(|e| e.id == enemy_id)
            {
                enemy.alive = false;
                enemy_deaths.push(EnemyDiedInfo {
                    enemy_id,
                    room_x: req.room_coords.0,
                    room_y: req.room_coords.1,
                });
            }
        }

        (bullet_spawns, enemy_deaths)
    }

    /// Raycast horizontal bullet path against alive enemies in the given room.
    /// Returns (spawn_x, spawn_y, hit_enemy_id).
    /// Bullet spawns from (origin_x, origin_y) and travels horizontally in `dir` up to 400px.
    /// Enemy hitbox: Rect centered on spawn_pos with w=30, h=38 (matches client Enemy::get_rect()).
    fn raycast_hit(
        &self,
        origin_x: f32,
        origin_y: f32,
        dir: f32,
        room_coords: (u8, u8),
    ) -> (f32, f32, Option<u8>) {
        let ray_end_x = origin_x + dir * 400.0;
        let ray_y = origin_y;

        let room_enemies = self.enemies.get(&room_coords);
        let Some(room_enemies) = room_enemies else {
            return (origin_x, origin_y, None);
        };

        let mut closest_hit: Option<(f32, u8)> = None; // (hit_x, enemy_id)

        for enemy in room_enemies {
            if !enemy.alive {
                continue;
            }
            let (spawn_x, spawn_y) = enemy.spawn_pos;
            // Enemy hitbox: centered on spawn_pos, w=30, h=38
            // (Rect not used directly, we compute bounds manually)

            // Check if horizontal ray intersects enemy rect
            // Ray: from (origin_x, ray_y) to (ray_end_x, ray_y)
            // Rect: [spawn_x - 15, spawn_x + 15] x [spawn_y - 19, spawn_y + 19]
            let rect_left = spawn_x - 15.0;
            let rect_right = spawn_x + 15.0;
            let rect_top = spawn_y - 19.0;
            let rect_bottom = spawn_y + 19.0;

            // Check if ray_y is within enemy's vertical bounds
            if ray_y >= rect_top && ray_y <= rect_bottom {
                // Check horizontal intersection
                let hit_x = if dir > 0.0 {
                    // Ray goes right: hit at rect_left if origin_x <= rect_left <= ray_end_x
                    if origin_x <= rect_left && rect_left <= ray_end_x {
                        Some(rect_left)
                    } else {
                        None
                    }
                } else {
                    // Ray goes left: hit at rect_right if ray_end_x <= rect_right <= origin_x
                    if ray_end_x <= rect_right && rect_right <= origin_x {
                        Some(rect_right)
                    } else {
                        None
                    }
                };

                if let Some(hit_x) = hit_x {
                    // Keep the closest hit (first hit wins)
                    match closest_hit {
                        None => closest_hit = Some((hit_x, enemy.id)),
                        Some((current_hit_x, _)) => {
                            if dir > 0.0 {
                                if hit_x < current_hit_x {
                                    closest_hit = Some((hit_x, enemy.id));
                                }
                            } else {
                                if hit_x > current_hit_x {
                                    closest_hit = Some((hit_x, enemy.id));
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some((_, enemy_id)) = closest_hit {
            (origin_x, origin_y, Some(enemy_id))
        } else {
            (origin_x, origin_y, None)
        }
    }

    /// Get enemy state for a room (for EnemyStateResponse).
    /// Returns Vec<(enemy_id, alive, spawn_x, spawn_y)> sorted by enemy_id for deterministic ordering.
    pub fn get_enemy_state(&self, room_x: u8, room_y: u8) -> Vec<(u8, bool, f32, f32)> {
        self.enemies
            .get(&(room_x, room_y))
            .map(|enemies| {
                let mut sorted: Vec<_> = enemies.iter().collect();
                sorted.sort_by_key(|e| e.id);
                sorted
                    .into_iter()
                    .map(|e| (e.id, e.alive, e.spawn_pos.0, e.spawn_pos.1))
                    .collect()
            })
            .unwrap_or_default()
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
        let (shots, _deaths) = world.update(tick_dt());
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].owner_id, 0);
        assert_eq!(shots[0].dir, 1.0);

        // The latch is cleared after the tick consumes it.
        assert!(!world.pending[&0].shoot);

        // No re-fire on the next tick without a fresh input.
        let (shots2, _deaths2) = world.update(tick_dt());
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
        let (shots, _deaths) = world.update(tick_dt());
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

// ============================================================
// Phase 2e — Server Enemy State & Hit Validation (AC 5, 6, 7, 8)
// ============================================================

/// AC 5 — ServerWorld::new() builds enemies HashMap for all rooms with
/// correct IDs (room_x * 16 + spawn_index) and tile-center spawn_pos.
/// This test will fail to compile until the `enemies` field is added to ServerWorld.
#[test]
fn serverworld_builds_enemies_all_rooms() {
    let world = ServerWorld::new();

    // The map has 2x2 rooms (room_x: 0..1, room_y: 0..1).
    // Each room's enemies are stored under (room_x, room_y) key.
    // This will fail until `enemies: HashMap<(u8,u8), Vec<EnemyState>>` is added.
    assert!(world.enemies.contains_key(&(0, 0)));
    assert!(world.enemies.contains_key(&(1, 0)));
    assert!(world.enemies.contains_key(&(0, 1)));
    assert!(world.enemies.contains_key(&(1, 1)));

    // Room (0,0): DATA_0_0 has no EnemySpawn/PotionSpawn (all zeros).
    assert!(world.enemies[&(0, 0)].is_empty());
    assert!(world.enemies[&(1, 0)].is_empty());
    assert!(world.enemies[&(0, 1)].is_empty());
    assert!(world.enemies[&(1, 1)].is_empty());

    // NOTE: Current map data has zero enemies. Once test enemies are added,
    // verify: id = room_x * 16 + spawn_index, spawn_pos = tile center, alive = true
}

/// AC 6 — Raycast hits enemy at tile center with centered rect.
/// Server hitbox = Rect::new(spawn_x - 15.0, spawn_y - 19.0, 30.0, 38.0)
/// centered on spawn_pos (tile center).
#[test]
fn raycast_hits_enemy() {
    let mut world = ServerWorld::new();
    world.spawn(0);

    // Manually add an enemy to room (0,0) for testing
    // We need to directly manipulate the enemies map since the map data has no enemies
    let enemy_state = EnemyState {
        id: 0,
        alive: true,
        spawn_pos: (200.0, 200.0), // Tile center
    };
    world.enemies.get_mut(&(0, 0)).unwrap().push(enemy_state);

    // Position player to shoot at the enemy (from left, shooting right)
    {
        let player = world.players.get_mut(&0).unwrap();
        player.pos = glam::Vec2::new(50.0, 181.0); // y = 200 - 19 = 181 (within enemy vertical bounds)
        player.dir = 1.0;
        player.w = 30.0;
        player.h = 38.0;
        player.grounded = true;
    }
    world.coords.insert(0, (0, 0));

    // Apply shoot input
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

    let (shots, deaths) = world.update(1.0 / 60.0);

    // Should have one shot and one death
    assert_eq!(shots.len(), 1);
    assert_eq!(deaths.len(), 1);
    assert_eq!(deaths[0].enemy_id, 0);
    assert_eq!(deaths[0].room_x, 0);
    assert_eq!(deaths[0].room_y, 0);

    // Enemy should be marked dead
    let room_enemies = &world.enemies[&(0, 0)];
    let enemy = room_enemies.iter().find(|e| e.id == 0).unwrap();
    assert!(!enemy.alive);
}

/// AC 7 — Raycast passing above/below enemy rect does NOT hit.
#[test]
fn raycast_miss_above_below() {
    let mut world = ServerWorld::new();
    world.spawn(0);

    // Add enemy at (200, 200)
    let enemy_state = EnemyState {
        id: 0,
        alive: true,
        spawn_pos: (200.0, 200.0),
    };
    world.enemies.get_mut(&(0, 0)).unwrap().push(enemy_state);

    // Position player to shoot ABOVE the enemy.
    // Enemy rect vertical bounds: spawn_y ± 19 = 181 to 219.
    // Bullet spawns from player center: player.pos.y + player.h/2.
    // To be above, need spawn_y < 181. With player.h=38, player.pos.y < 162.
    {
        let player = world.players.get_mut(&0).unwrap();
        player.pos = glam::Vec2::new(50.0, 150.0); // spawn_y = 150 + 19 = 169 < 181
        player.dir = 1.0;
        player.w = 30.0;
        player.h = 38.0;
        player.grounded = true;
    }
    world.coords.insert(0, (0, 0));

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

    let (shots, deaths) = world.update(1.0 / 60.0);

    // Should have shot but no death
    assert_eq!(shots.len(), 1);
    assert_eq!(deaths.len(), 0);

    // Enemy should still be alive
    let room_enemies = &world.enemies[&(0, 0)];
    let enemy = room_enemies.iter().find(|e| e.id == 0).unwrap();
    assert!(enemy.alive);
}

/// AC 7 (2nd) — Two enemies in line; only closer one hit (first hit wins).
#[test]
fn raycast_first_hit_wins() {
    let mut world = ServerWorld::new();
    world.spawn(0);

    // Add two enemies in line horizontally: closer at x=200, farther at x=300
    let enemy1 = EnemyState {
        id: 0,
        alive: true,
        spawn_pos: (200.0, 200.0),
    };
    let enemy2 = EnemyState {
        id: 1,
        alive: true,
        spawn_pos: (300.0, 200.0),
    };
    world.enemies.get_mut(&(0, 0)).unwrap().push(enemy1);
    world.enemies.get_mut(&(0, 0)).unwrap().push(enemy2);

    // Player shoots from left (x=50) going right
    {
        let player = world.players.get_mut(&0).unwrap();
        player.pos = glam::Vec2::new(50.0, 181.0); // Within vertical bounds
        player.dir = 1.0;
        player.w = 30.0;
        player.h = 38.0;
        player.grounded = true;
    }
    world.coords.insert(0, (0, 0));

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

    let (shots, deaths) = world.update(1.0 / 60.0);

    // Should have one shot, only first enemy dies
    assert_eq!(shots.len(), 1);
    assert_eq!(deaths.len(), 1);
    assert_eq!(deaths[0].enemy_id, 0); // Closer enemy (id=0 at x=200)

    // Check enemy states
    let room_enemies = &world.enemies[&(0, 0)];
    let enemy1 = room_enemies.iter().find(|e| e.id == 0).unwrap();
    let enemy2 = room_enemies.iter().find(|e| e.id == 1).unwrap();
    assert!(!enemy1.alive, "Closer enemy should be dead");
    assert!(enemy2.alive, "Farther enemy should still be alive");
}

/// AC 8 — ServerWorld::update returns (Vec<BulletSpawnInfo>, Vec<EnemyDiedInfo>)
/// and main.rs broadcasts both.
#[test]
fn update_returns_shots_and_deaths() {
    let mut world = ServerWorld::new();
    world.spawn(0);
    // Apply shoot input (player 0, room 0,0, seq=1)
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

    // update should return tuple of (shots, deaths)
    let (shots, deaths) = world.update(1.0 / 60.0);

    // Verify types
    let _: Vec<BulletSpawnInfo> = shots;
    let _: Vec<EnemyDiedInfo> = deaths;

    // Should have one shot, zero deaths (no enemies in map)
    assert_eq!(shots.len(), 1);
    assert_eq!(deaths.len(), 0);
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

    let (shots, _deaths) = world.update(1.0 / 60.0);

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
    let (shots1, _) = world.update(1.0 / 60.0);
    assert_eq!(shots1.len(), 1);
    assert_eq!(shots1[0].bullet_id, 0);

    // Tick 2-5: no input, cooldown counts down (4 ticks to go from 4 to 0)
    for _ in 0..4 {
        let (shots, _) = world.update(1.0 / 60.0);
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
    let (shots6, _) = world.update(1.0 / 60.0);
    assert_eq!(shots6.len(), 1);
    assert_eq!(shots6[0].bullet_id, 1); // Incremented
}
