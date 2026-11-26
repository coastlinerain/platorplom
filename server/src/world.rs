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

/// One server-authoritative PvP kill detected in a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerHitInfo {
    pub shooter_id: u8,
    pub victim_id: u8,
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
    /// Returns the `BulletSpawnInfo`s for accepted shots and the
    /// `PlayerHitInfo`s for kills detected this tick. Victims are respawned
    /// before this returns, so callers only need to broadcast the deaths.
    pub fn update(&mut self, dt: f32) -> (Vec<BulletSpawnInfo>, Vec<PlayerHitInfo>) {
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

        // Second pass: resolve shots against the post-movement positions.
        //
        // `self.players` is a `HashMap`, so sort first: this makes bullet-id
        // assignment deterministic and credits a multi-shooter victim to the
        // lowest shooter id.
        //
        // Hit tests see this tick's post-movement positions. Respawns are
        // applied only after the loop, so a victim cannot be teleported
        // mid-pass or hit twice in the same tick. A victim that also shot this
        // tick still fires (its `ShootRequest` already exists).
        shoot_requests.sort_by_key(|r| r.player_id);

        let mut player_hits: Vec<PlayerHitInfo> = Vec::new();

        for req in shoot_requests {
            let hit = self.raycast_player(req.player_id, req.spawn_x, req.spawn_y, req.dir);

            let bullet_id = self.next_bullet_id;
            self.next_bullet_id = self.next_bullet_id.wrapping_add(1);
            bullet_spawns.push(BulletSpawnInfo {
                bullet_id,
                // On a hit the bullet starts at the impact x (owner decision);
                // on a miss it keeps the historical origin spawn.
                x: hit.map_or(req.spawn_x, |(_, hit_x)| hit_x),
                y: req.spawn_y,
                dir: req.dir,
                owner_id: req.player_id,
                seq: req.player_seq,
            });

            if let Some((victim_id, _)) = hit {
                // First shot in ascending shooter-id order wins; at most one
                // death per victim per tick.
                if !player_hits.iter().any(|h| h.victim_id == victim_id) {
                    player_hits.push(PlayerHitInfo {
                        shooter_id: req.player_id,
                        victim_id,
                    });
                }
            }
        }

        for hit in &player_hits {
            self.respawn(hit.victim_id);
        }

        self.current_tick += 1;
        (bullet_spawns, player_hits)
    }

    /// Casts a horizontal hitscan from the shooter's bullet origin and returns
    /// the closest other player in the same room that is not behind a wall, as
    /// `(victim_id, hit_x)`.
    fn raycast_player(
        &self,
        shooter_id: u8,
        spawn_x: f32,
        spawn_y: f32,
        dir: f32,
    ) -> Option<(u8, f32)> {
        let shooter_room = self.coords.get(&shooter_id).copied().unwrap_or((0, 0));
        let level = self
            .levels
            .get(shooter_room.1 as usize)
            .and_then(|row| row.get(shooter_room.0 as usize))
            .unwrap_or(&self.levels[0][0]);

        // Ascending victim-id iteration plus a strict `<` tie-break keeps the
        // lower id on an exact distance tie.
        let mut victims: Vec<u8> = self
            .players
            .keys()
            .copied()
            .filter(|&id| id != shooter_id)
            .filter(|id| self.coords.get(id).copied().unwrap_or((0, 0)) == shooter_room)
            .collect();
        victims.sort_unstable();

        let mut best: Option<(u8, f32, f32)> = None; // (victim_id, distance, hit_x)

        for victim_id in victims {
            let Some(victim) = self.players.get(&victim_id) else {
                continue;
            };

            let cx = victim.pos.x + victim.w / 2.0;
            let cy = victim.pos.y + victim.h / 2.0;
            let left = cx - victim.w / 2.0;
            let right = cx + victim.w / 2.0;

            // Inclusive vertical overlap with the ray `y = spawn_y`.
            if spawn_y < cy - victim.h / 2.0 || spawn_y > cy + victim.h / 2.0 {
                continue;
            }

            let (near_edge, far_edge) = if dir >= 0.0 {
                (left, right)
            } else {
                (right, left)
            };
            let d_near = (near_edge - spawn_x) * dir;
            let d_far = (far_edge - spawn_x) * dir;

            // Whole rect is behind the origin.
            if d_far < 0.0 {
                continue;
            }

            // Overlapping the victim clamps to distance 0 → hit at the origin.
            let distance = d_near.max(0.0);
            let hit_x = spawn_x + dir * distance;

            if level.solid_between(spawn_x, hit_x, spawn_y) {
                continue;
            }

            let replace = match best {
                Some((_, best_distance, _)) => distance < best_distance,
                None => true,
            };
            if replace {
                best = Some((victim_id, distance, hit_x));
            }
        }

        best.map(|(victim_id, _, hit_x)| (victim_id, hit_x))
    }

    /// Resets a slain player to the canonical spawn and drops its held input.
    /// `shoot_cooldown` and `max_seq` are intentionally left untouched.
    fn respawn(&mut self, id: u8) {
        if let Some(player) = self.players.get_mut(&id) {
            *player = Player::new(id);
        }
        self.coords.insert(id, (0, 0));
        self.pending.insert(id, PlayerInput::default());
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
        let (shots, _hits) = world.update(tick_dt());
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].owner_id, 0);
        assert_eq!(shots[0].dir, 1.0);

        // The latch is cleared after the tick consumes it.
        assert!(!world.pending[&0].shoot);

        // No re-fire on the next tick without a fresh input.
        let (shots2, _hits) = world.update(tick_dt());
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
        let (shots, _hits) = world.update(tick_dt());
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

    let (shots, _hits) = world.update(1.0 / 60.0);

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
    let (shots1, _hits) = world.update(1.0 / 60.0);
    assert_eq!(shots1.len(), 1);
    assert_eq!(shots1[0].bullet_id, 0);

    // Tick 2-5: no input, cooldown counts down (4 ticks to go from 4 to 0)
    for _ in 0..4 {
        let (shots, _hits) = world.update(1.0 / 60.0);
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
    let (shots6, _hits) = world.update(1.0 / 60.0);
    assert_eq!(shots6.len(), 1);
    assert_eq!(shots6[0].bullet_id, 1); // Incremented
}

// Spec 2 (authoritative player death) — raycast / hit / respawn tests. See plan
// Box 3 for the API these exercise.
#[cfg(test)]
mod raycast_and_hit_tests {
    use super::{PlayerHitInfo, ServerWorld};
    use game::entities::player::PlayerInput;
    use game::types::TileType;
    use glam::Vec2;

    /// Spawn `id`, then place it at `(x, y)` facing `dir` with the canonical
    /// player size, zero velocity, grounded, in room `(0, 0)`.
    fn place(world: &mut ServerWorld, id: u8, x: f32, y: f32, dir: f32) {
        world.spawn(id);
        let player = world.players.get_mut(&id).expect("just spawned");
        player.pos = Vec2::new(x, y);
        player.w = 30.0;
        player.h = 38.0;
        player.vel = Vec2::ZERO;
        player.dir = dir;
        player.grounded = true;
        world.coords.insert(id, (0, 0));
    }

    fn tick() -> f32 {
        1.0 / 60.0
    }

    /// S1 — a rightward ray from the shooter's bullet origin hits the victim at
    /// its left edge, and an origin already inside the victim's rect clamps the
    /// distance to 0 (`hit_x == spawn_x`).
    #[test]
    fn raycast_hits_player_same_room() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        // Bullet origin of shooter 1: (100 + 30/2, 200 + 38/2) = (115, 219).
        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)));

        // Origin inside the victim's rect: d_near < 0 clamps distance to 0.
        assert_eq!(world.raycast_player(1, 410.0, 219.0, 1.0), Some((2, 410.0)));
    }

    /// S2 — the same geometry misses when the victim claims another room.
    #[test]
    fn raycast_misses_player_in_other_room() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);
        world.coords.insert(2, (0, 1));

        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), None);
    }

    /// S3 — a wall tile in the ray's row between origin and impact occludes;
    /// clearing it makes the hit visible again.
    #[test]
    fn raycast_blocked_by_wall() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        // y = 219 → row 5; col 5 covers x ∈ [200, 240], inside the segment.
        world.levels[0][0].grid[5][5] = TileType::Wall;
        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), None);

        world.levels[0][0].grid[5][5] = TileType::Empty;
        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)));
    }

    /// S4 — two victims in line: the nearer one (smaller distance) wins.
    #[test]
    fn raycast_first_hit_wins() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 300.0, 200.0, 1.0);
        place(&mut world, 3, 500.0, 200.0, 1.0);

        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 300.0)));
    }

    /// S5 — the shooter is never a candidate (only player → `None`), and a
    /// victim entirely behind the origin is skipped (`d_far < 0`).
    #[test]
    fn raycast_never_hits_shooter() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);

        // Shooter only: there is nothing to hit.
        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), None);

        place(&mut world, 2, 400.0, 200.0, 1.0);
        let hit = world.raycast_player(1, 115.0, 219.0, 1.0);
        assert_eq!(hit, Some((2, 400.0)));
        assert_ne!(hit.map(|(id, _)| id), Some(1));

        // Victim behind the shooter: near/far edges are both to the left of the
        // rightward ray origin → d_far < 0 → skipped.
        let victim = world.players.get_mut(&2).expect("victim spawned");
        victim.pos = Vec2::new(0.0, 200.0);
        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), None);
    }

    /// S6 — exact distance tie: ascending victim-id iteration with a strict `<`
    /// keeps the lower id, even when it was spawned second.
    #[test]
    fn raycast_tie_breaks_on_lower_id() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 5, 400.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)));
    }

    /// S7 — a hit returns both the bullet (spawned at the impact x) and the
    /// `PlayerHitInfo` for the shooter/victim pair.
    #[test]
    fn update_returns_bullet_and_hit_info() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        world.apply_input(
            1,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            3,
        );

        let (shots, hits) = world.update(tick());

        assert_eq!(
            hits,
            vec![PlayerHitInfo {
                shooter_id: 1,
                victim_id: 2
            }]
        );
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].owner_id, 1);
        assert_eq!(shots[0].seq, 3);
        assert_eq!(shots[0].dir, 1.0);
        // Impact x, not the origin (115) and not the room edge (800).
        assert!(
            (shots[0].x - 400.0).abs() < 0.01,
            "expected impact x 400, got {}",
            shots[0].x
        );
    }

    /// S8 — a miss keeps the historical behavior: bullet spawned at the origin.
    #[test]
    fn update_miss_emits_bullet_at_origin() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);

        world.apply_input(
            1,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            1,
        );

        let (shots, hits) = world.update(tick());

        assert!(hits.is_empty());
        assert_eq!(shots.len(), 1);
        assert!(
            (shots[0].x - 115.0).abs() < 0.01,
            "expected origin x 115, got {}",
            shots[0].x
        );
        // Allow one tick of gravity drift (GRAVEDAD = 20 at dt = 1/60).
        assert!(
            (shots[0].y - 219.0).abs() < 1.0,
            "expected origin y ~219, got {}",
            shots[0].y
        );
    }

    /// S9 — the victim is respawned at the canonical spawn after the request
    /// loop, with held input dropped; the shooter is untouched.
    #[test]
    fn update_respawns_victim_at_origin() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        // Held input so the `pending` reset is observable.
        world.apply_input(
            2,
            0,
            0,
            PlayerInput {
                move_left: true,
                ..Default::default()
            },
            0,
        );

        let before = {
            let shooter = &world.players()[&1];
            (shooter.pos, shooter.dir)
        };

        world.apply_input(
            1,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            3,
        );

        let (shots, hits) = world.update(tick());

        assert_eq!(shots.len(), 1);
        assert_eq!(
            hits,
            vec![PlayerHitInfo {
                shooter_id: 1,
                victim_id: 2
            }]
        );

        // Victim rebuilt from the canonical spawn.
        assert_eq!(world.players()[&2].pos, Vec2::new(100.0, 100.0));
        assert_eq!(world.players()[&2].vel, Vec2::ZERO);
        assert_eq!(world.players()[&2].dir, 1.0);
        assert!(!world.players()[&2].grounded);
        assert_eq!(world.coords()[&2], (0, 0));

        // `PlayerInput` has no `PartialEq` — assert each flag individually.
        let pending = &world.pending[&2];
        assert!(!pending.move_left);
        assert!(!pending.move_right);
        assert!(!pending.jump);
        assert!(!pending.shoot);

        // Shooter unchanged (only the sub-pixel gravity drift).
        let after = &world.players()[&1];
        assert_eq!(after.pos.x, before.0.x);
        assert_eq!(after.dir, before.1);
        assert!((after.pos.y - before.0.y).abs() < 1.0);
    }

    /// S10 — two shooters hit the same victim in one tick: exactly one
    /// `PlayerHitInfo` (credited to the lower shooter id) and two bullets in
    /// deterministic ascending-shooter order.
    #[test]
    fn update_dedupes_one_victim_per_tick() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 3, 700.0, 200.0, -1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        world.apply_input(
            1,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            1,
        );
        world.apply_input(
            3,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            2,
        );

        let (shots, hits) = world.update(tick());

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0],
            PlayerHitInfo {
                shooter_id: 1,
                victim_id: 2
            }
        );
        assert_eq!(shots.len(), 2);
        assert_eq!(shots[0].owner_id, 1);
        assert_eq!(shots[1].owner_id, 3);
    }
}

// Spec 3 (authoritative room membership) — `apply_room_change`, `force_teleport`
// and the authoritative-coords invariants. See plan Box 3.
#[cfg(test)]
mod room_change_tests {
    use super::ServerWorld;
    use game::config::ROOM_CHANGE_COOLDOWN_TICKS;
    use game::entities::bullet::Bullet;
    use game::entities::player::PlayerInput;
    use game::room_graph::Direction;
    use glam::Vec2;

    /// Spawn `id`, place it at `(x, y)` facing `dir` with the canonical player
    /// size, zero velocity and `grounded = true`, in room `(0, 0)`.
    fn place(world: &mut ServerWorld, id: u8, x: f32, y: f32, dir: f32) {
        world.spawn(id);
        let player = world.players.get_mut(&id).expect("just spawned");
        player.pos = Vec2::new(x, y);
        player.w = 30.0;
        player.h = 38.0;
        player.vel = Vec2::ZERO;
        player.dir = dir;
        player.grounded = true;
        world.coords.insert(id, (0, 0));
    }

    fn tick() -> f32 {
        1.0 / 60.0
    }

    /// S1 — a request made from inside the geometric East band is accepted and
    /// teleports to the canonical entry.
    #[test]
    fn apply_room_change_accepts_at_door() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 766.0, 320.0, 1.0);

        let info = world.apply_room_change(1, Direction::East).unwrap();

        assert!(info.accepted);
        assert_eq!(info.room, (1, 0));
        assert_eq!(info.pos, (10.0, 321.0));
        assert_eq!(world.coords()[&1], (1, 0));
        assert_eq!(world.players()[&1].pos, Vec2::new(10.0, 321.0));
    }

    /// S2 — one prediction tick short of the geometric band is still accepted
    /// via the server-only lead tolerance.
    #[test]
    fn apply_room_change_accepts_within_lead_tolerance() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 730.0, 320.0, 1.0);

        let info = world.apply_room_change(1, Direction::East).unwrap();

        assert!(info.accepted);
        assert_eq!(info.room, (1, 0));
        assert_eq!(info.pos, (10.0, 321.0));
        assert_eq!(world.coords()[&1], (1, 0));
    }

    /// S3 — beyond the lead tolerance the request is rejected and nothing moves.
    #[test]
    fn apply_room_change_rejects_beyond_lead_tolerance() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 700.0, 320.0, 1.0);

        let info = world.apply_room_change(1, Direction::East).unwrap();

        assert!(!info.accepted);
        assert_eq!(info.room, (0, 0));
        assert_eq!(info.pos, (700.0, 320.0));
        assert_eq!(world.coords()[&1], (0, 0));
    }

    /// S4 — directions with no door are rejected, twice in a row, without state.
    #[test]
    fn apply_room_change_rejects_no_door() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 100.0, 1.0);

        for direction in [Direction::North, Direction::West] {
            let info = world.apply_room_change(1, direction).unwrap();
            assert!(!info.accepted);
            assert_eq!(info.room, (0, 0));
            assert_eq!(info.pos, (100.0, 100.0));
        }

        assert_eq!(world.coords()[&1], (0, 0));
        assert_eq!(world.players()[&1].pos, Vec2::new(100.0, 100.0));
    }

    /// S5 — a real door requested from the middle of the room is rejected.
    #[test]
    fn apply_room_change_rejects_off_trigger() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);

        let info = world.apply_room_change(1, Direction::East).unwrap();

        assert!(!info.accepted);
        assert_eq!(world.coords()[&1], (0, 0));
        assert_eq!(world.players()[&1].pos, Vec2::new(100.0, 200.0));
    }

    /// S6 — the cooldown rejects an immediate reverse request; once it expires
    /// the same request is accepted (the entry overlaps the reverse band).
    #[test]
    fn apply_room_change_rejects_during_cooldown() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 766.0, 320.0, 1.0);

        let first = world.apply_room_change(1, Direction::East).unwrap();
        assert!(first.accepted);

        let blocked = world.apply_room_change(1, Direction::West).unwrap();
        assert!(!blocked.accepted);
        assert_eq!(blocked.room, (1, 0));
        assert_eq!(blocked.pos, (10.0, 321.0));

        for _ in 0..ROOM_CHANGE_COOLDOWN_TICKS {
            world.update(tick());
        }

        let accepted = world.apply_room_change(1, Direction::West).unwrap();
        assert!(accepted.accepted);
        assert_eq!(accepted.room, (0, 0));
        assert_eq!(accepted.pos, (760.0, 321.0));
        assert_eq!(world.coords()[&1], (0, 0));
    }

    /// S7 — acceptance teleports to the entry, zeroes velocity, recomputes
    /// `grounded`, and preserves dir, bullets, pending input, cooldown and seq.
    #[test]
    fn apply_room_change_teleports_to_entry() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 766.0, 320.0, -1.0);

        world
            .players
            .get_mut(&1)
            .expect("just placed")
            .bullets
            .push(Bullet::new(Vec2::new(1.0, 1.0), 1.0, 1));
        world.apply_input(
            1,
            PlayerInput {
                move_right: true,
                jump: true,
                ..Default::default()
            },
            9,
        );
        world.shoot_cooldown.insert(1, 3);

        // Prove `grounded` is recomputed, not inherited from `place`.
        world.players.get_mut(&1).expect("just placed").grounded = false;

        let info = world.apply_room_change(1, Direction::East).unwrap();
        assert!(info.accepted);

        let player = &world.players()[&1];
        assert_eq!(player.pos, Vec2::new(10.0, 321.0));
        assert_eq!(player.vel, Vec2::ZERO);
        assert_eq!(player.dir, -1.0);
        assert!(player.grounded, "grounded should be recomputed in (1,0)");
        assert_eq!(player.bullets.len(), 1, "bullets must be preserved");

        assert!(world.pending[&1].move_right);
        assert!(world.pending[&1].jump);
        assert_eq!(world.shoot_cooldown[&1], 3);
        assert_eq!(world.max_seq()[&1], 9);
    }

    /// S8 — `apply_input` never writes `coords`, including after a respawn
    /// (the spec-2 respawn-overwrite bug).
    #[test]
    fn apply_input_never_changes_coords() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 100.0, 1.0);

        world.force_teleport(1, (1, 0), (100.0, 100.0));
        world.apply_input(
            1,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            1,
        );
        assert_eq!(world.coords()[&1], (1, 0));

        world.respawn(1);
        assert_eq!(world.coords()[&1], (0, 0));

        world.apply_input(
            1,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            2,
        );
        assert_eq!(world.coords()[&1], (0, 0));
    }

    /// S9 — a respawn from another room is a forced teleport with no door check.
    #[test]
    fn respawn_ignores_door_validation() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 100.0, 1.0);

        world.force_teleport(1, (1, 1), (10.0, 401.0));
        world.respawn(1);

        assert_eq!(world.coords()[&1], (0, 0));
        assert_eq!(world.players()[&1].pos, Vec2::new(100.0, 100.0));
    }

    /// S10 — `force_teleport` bypasses doors, zeroes velocity (exercised with a
    /// non-zero starting velocity) and recomputes `grounded`.
    #[test]
    fn force_teleport_bypasses_doors() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 100.0, 1.0);

        world.players.get_mut(&1).expect("just placed").vel = Vec2::new(50.0, -20.0);
        world.players.get_mut(&1).expect("just placed").grounded = false;

        world.force_teleport(1, (1, 1), (10.0, 401.0));

        assert_eq!(world.coords()[&1], (1, 1));
        assert_eq!(world.players()[&1].pos, Vec2::new(10.0, 401.0));
        assert_eq!(world.players()[&1].vel, Vec2::ZERO);
        assert!(
            world.players()[&1].grounded,
            "grounded should be recomputed in (1,1) at (10, 401)"
        );
    }

    /// S11 — a victim cannot hide from a raycast by claiming another room; the
    /// rejected request leaves it in the shooter's room.
    #[test]
    fn raycast_victim_cannot_hide_by_room_claim() {
        let mut world = ServerWorld::new();
        place(&mut world, 1, 100.0, 200.0, 1.0);
        place(&mut world, 2, 400.0, 200.0, 1.0);

        assert!(
            !world
                .apply_room_change(2, Direction::East)
                .unwrap()
                .accepted
        );
        assert!(
            !world
                .apply_room_change(2, Direction::South)
                .unwrap()
                .accepted
        );
        assert_eq!(world.coords()[&2], (0, 0));

        assert_eq!(world.raycast_player(1, 115.0, 219.0, 1.0), Some((2, 400.0)));
    }

    /// S12 — an unknown player returns `None` and creates no state.
    #[test]
    fn apply_room_change_returns_none_for_missing_player() {
        let mut world = ServerWorld::new();

        assert_eq!(world.apply_room_change(9, Direction::East), None);
        assert!(world.coords().get(&9).is_none());
    }

    /// S13 — an out-of-range forced room must not index-panic and the input
    /// still drives movement (replaces the old client-claimed-coords test).
    #[test]
    fn force_teleport_out_of_range_room_falls_back_to_origin() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.force_teleport(0, (255, 255), (100.0, 100.0));
        world.apply_input(
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick());

        assert!(world.players()[&0].pos.x > 100.0);
        assert_eq!(world.coords()[&0], (255, 255));
    }
}
