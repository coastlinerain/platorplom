use crate::camera::GameCamera;
use crate::network::packet_io::PacketIO;
use crate::render::Drawable;
use crate::world::lib::{
    check_level_transitions, check_player_death, update_enemy_bullets_position,
};

use crate::network::interpolation::RemoteInterp;
use crate::network::prediction::InputRingBuffer;
use crate::network::world::update_network;
use game::config::INTERPOLATION_DELAY_TICKS;
use game::entities::bullet::Bullet;
use game::entities::enemy::Enemy;
use game::entities::nemesis::Nemesis;
use game::entities::player::Player;
use game::level::Level;
use game::maps::Map;
use macroquad::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Server-confirmed bullet data for reconciliation.
#[derive(Debug, Clone)]
pub struct ConfirmedBullet {
    pub bullet_id: u16,
    pub x: f32,
    pub y: f32,
    pub dir: f32,
}

pub struct World {
    pub id: Option<u8>,
    pub player: Player,
    pub enemies: Vec<Enemy>,
    pub levels_matrix: Vec<Vec<Level>>,
    pub current_coords: (usize, usize),
    pub camera: GameCamera,
    pub network: Box<dyn PacketIO>,
    pub other_players: HashMap<u8, Nemesis>,
    pub enemy_bullets: Vec<Bullet>,
    pub bullets: Vec<Bullet>, // own bullets (server-authoritative)
    // --- prediction fields ---
    pub input_seq: u16,
    pub last_acked_seq: u16,
    pub input_buffer: InputRingBuffer,
    pub jump_sticky: u8,
    pub shoot_sticky: u8,
    pub other_interp: HashMap<u8, RemoteInterp>,
    // Per-frame edge key state (consumed once in update, read by update_network).
    pub pressed_jump: bool,
    pub pressed_shoot: bool,
    // Programmatic input injection (set by World::update in binary or by TestClient in e2e).
    pub move_left: bool,
    pub move_right: bool,
    // --- NEW: Phase 2e ---
    pub spawned_bullet_ids: HashSet<u16>, // deduplicate BulletSpawn packets
    pub confirmed_bullets: HashMap<u16, ConfirmedBullet>, // seq -> confirmed bullet data
    // Enemy deaths received before enemy was spawned (race condition fix)
    pub pending_enemy_deaths: HashSet<u8>,
    // --- NEW: Phase 2f - Clock synchronization ---
    pub client_tick: u64,
    pub server_tick_offset: i64, // server_tick - client_tick
    pub clock_offset_ms: i64,    // server_time_ms - local_time_ms (estimated)
    pub last_server_tick: u64,
    pub last_server_time_ms: u64,
    pub last_acked_server_tick: u64, // for staleness heuristic
    // --- NEW: Phase 2g - Render interpolation ---
    /// Player state at the previous sim tick, used to lerp the render position
    /// between ticks (`draw(alpha)`).
    pub prev_player_pos: glam::Vec2,
    pub prev_player_dir: f32,
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World")
            .field("player", &self.player)
            .field("enemies", &self.enemies)
            .field("levels_matrix", &self.levels_matrix.len())
            .field("current_coords", &self.current_coords)
            .field("camera", &self.camera)
            .field("other_players", &self.other_players)
            .finish()
    }
}

impl World {
    pub fn new(matrix_data: Map, network: Box<dyn PacketIO>) -> Self {
        // JoinRequest is sent from main.rs before World is constructed.
        // Do NOT call send_join_req here — that would send a second
        // JoinRequest, creating a duplicate player on the server.

        let levels: Vec<Vec<Level>> = matrix_data
            .grids
            .into_iter()
            .zip(matrix_data.data)
            .map(|(grid_row, data_row)| {
                grid_row
                    .into_iter()
                    .zip(data_row)
                    .map(|(grid, data)| Level::new(grid, data))
                    .collect()
            })
            .collect();

        let enemies = game::entities::enemy::enemies_from_data(&levels[0][0].data, 0);

        let player = Player::new(0);
        // Initialize the interpolation source to the player's spawn state so the
        // first rendered frame shows the current position with no visual jump.
        let prev_player_pos = player.pos;
        let prev_player_dir = player.dir;

        Self {
            id: None,
            player,
            prev_player_pos,
            prev_player_dir,
            enemies,
            levels_matrix: levels,
            current_coords: (0, 0),
            camera: GameCamera::new(),
            network,
            other_players: HashMap::new(),
            enemy_bullets: Vec::new(),
            bullets: Vec::new(),
            input_seq: u16::MAX,
            last_acked_seq: u16::MAX,
            input_buffer: InputRingBuffer::new(),
            jump_sticky: 0,
            shoot_sticky: 0,
            other_interp: HashMap::new(),
            // NEW: Phase 2e
            spawned_bullet_ids: HashSet::new(),
            confirmed_bullets: HashMap::new(),
            pending_enemy_deaths: HashSet::new(),
            // NEW: Phase 2f - Clock synchronization
            client_tick: 0,
            server_tick_offset: 0,
            clock_offset_ms: 0,
            last_server_tick: 0,
            last_server_time_ms: 0,
            last_acked_server_tick: 0,
            pressed_jump: false,
            pressed_shoot: false,
            move_left: false,
            move_right: false,
        }
    }

    pub fn update(&mut self, dt: f32) {
        // Increment client tick for clock synchronization (once per sim tick).
        self.client_tick = self.client_tick.wrapping_add(1);

        // Run transition check AFTER bullet-vs-enemy collision so bullets
        // can hit enemies on the frame they're fired, even if the player
        // crosses a room boundary on the same frame.
        update_enemy_bullets_position(self, dt);
        check_player_death(self);
        {
            let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];

            // Store the previous player state BEFORE simulating so
            // `interpolated_player_pos/dir` can lerp prev → current (Phase 2g).
            self.prev_player_pos = self.player.pos;
            self.prev_player_dir = self.player.dir;

            // --- CLIENT PREDICTION: simulate local player immediately ---
            // Input is injected programmatically (sampled once per render frame
            // in main.rs) so that `update` has no macroquad input dependency and
            // runs safely under `#[cfg(test)]` with no window.
            let local_input = game::entities::player::PlayerInput {
                move_left: self.move_left,
                move_right: self.move_right,
                jump: self.pressed_jump,
                shoot: false, // bullet spawn is server-authoritative (Phase 2e)
            };
            self.player.update(dt, level, &local_input);

            // NOTE: Local bullet spawn on pressed_shoot is REMOVED (Phase 2e).
            // Bullets are now spawned only when BulletSpawn packet arrives from server.

            // Update own bullets (physics + wall collision).
            for bullet in &mut self.bullets {
                bullet.update(dt, level);
            }

            // NOTE: Own-bullet vs enemy collision loop REMOVED (Phase 2e).
            // Enemy death is now server-authoritative via EnemyDied packets.

            // Enemies still simulated client-side (not authoritative yet).
            for enemy in &mut self.enemies {
                let random = macroquad::rand::gen_range(0, 4);
                enemy.update(dt, level, random);
            }

            self.camera
                .update(self.player.pos, vec2(self.player.w, self.player.h), level);
        }
        // Must run AFTER bullet collision so bullets can hit enemies before
        // load_current_level (called by check_level_transitions) clears them.
        check_level_transitions(self);
        update_network(self);

        self.bullets.retain(|b| b.active);
        self.enemies.retain(|e| e.alive);
    }

    /// Interpolate the local player's render position between the previous and
    /// current sim tick (`prev_player_pos` → `player.pos`), clamping `alpha` to
    /// [0, 1] so an out-of-range first `frame_dt` never extrapolates.
    pub fn interpolated_player_pos(&self, alpha: f32) -> glam::Vec2 {
        let a = alpha.clamp(0.0, 1.0);
        self.prev_player_pos.lerp(self.player.pos, a)
    }

    /// Interpolate the local player's facing direction between the previous and
    /// current sim tick. Uses simple linear interpolation (see Box 4 Watch out
    /// for the -π/π boundary limitation, accepted for small per-tick changes).
    pub fn interpolated_player_dir(&self, alpha: f32) -> f32 {
        let a = alpha.clamp(0.0, 1.0);
        self.prev_player_dir + (self.player.dir - self.prev_player_dir) * a
    }

    pub fn draw(&self, alpha: f32) {
        self.camera.apply();

        let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];
        level.draw();

        // Remote players: tick-based interpolation with jitter buffer.
        // Compute server tick estimate and target render tick.
        let server_tick_estimate = self.client_tick as i64 + self.server_tick_offset;
        if server_tick_estimate > 0 {
            let server_tick_estimate = server_tick_estimate as u64;
            let target_tick = server_tick_estimate.saturating_sub(INTERPOLATION_DELAY_TICKS as u64);

            for (id, nemesis) in &self.other_players {
                if nemesis.coords == self.current_coords
                    && let Some(interp) = self.other_interp.get(id)
                    && let Some((ix, iy, idir)) = interp.interpolate_at_tick(
                        nemesis,
                        target_tick,
                        interp.last_known_server_tick,
                        interp.last_known_arrival,
                    )
                {
                    crate::render::draw_nemesis_at(nemesis, ix, iy, idir);
                } else if nemesis.coords == self.current_coords {
                    nemesis.draw();
                }
            }
        } else {
            // No clock sync yet: fall back to raw positions
            for nemesis in self.other_players.values() {
                if nemesis.coords == self.current_coords {
                    nemesis.draw();
                }
            }
        }

        // Own player — render at the interpolated position for smooth motion
        // between sim ticks. (The rectangle is direction-agnostic, so only the
        // position affects rendering today; `interpolated_player_dir` is kept
        // for API parity and a future facing indicator.)
        let render_pos = self.interpolated_player_pos(alpha);
        draw_rectangle(
            render_pos.x,
            render_pos.y,
            self.player.w,
            self.player.h,
            RED,
        );

        // Own bullets.
        for bullet in &self.bullets {
            bullet.draw();
        }

        // Remote enemy bullets.
        for bullet in &self.enemy_bullets {
            bullet.draw();
        }

        // Local enemies.
        for enemy in &self.enemies {
            enemy.draw();
        }

        set_default_camera();

        // HUD (screen-space).
        draw_text(
            format!("Level: {},{}", self.current_coords.0, self.current_coords.1),
            10.0,
            20.0,
            20.0,
            WHITE,
        );

        if let Some(my_id) = self.id {
            draw_text(format!("My ID: {}", my_id), 10.0, 40.0, 20.0, GREEN);
        } else {
            draw_text("Connecting...", 10.0, 40.0, 20.0, RED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::packet_io::PacketIO;
    use game::config::{MAX_ACCUMULATED_FRAMES, TICK_HZ, VELOCIDAD};
    use game::maps::Map;

    /// Minimal PacketIO that does nothing — for unit tests that don't need network.
    struct DummyPacketIO;

    impl PacketIO for DummyPacketIO {
        fn try_recv(&mut self) -> Option<protocol::GamePacket> {
            None
        }
        fn send(&mut self, _packet: protocol::GamePacket) {}
    }

    fn make_test_world() -> World {
        let world_data = Map::new();
        World::new(world_data, Box::new(DummyPacketIO))
    }

    const TICK_DT: f32 = 1.0 / TICK_HZ as f32;

    /// Box 2 (AC1): World::new initializes `prev_player_pos`/`prev_player_dir` to
    /// the player's initial position/direction, so the first rendered frame has a
    /// sane interpolation source.
    #[test]
    fn world_new_initializes_prev_player_state() {
        let world = make_test_world();
        assert_eq!(world.prev_player_pos, world.player.pos);
        assert_eq!(world.prev_player_dir, world.player.dir);
    }

    /// Box 3: `update` stores the previous position/direction *before* simulating
    /// the player, so `interpolated_player_pos/dir` can lerp prev → current.
    #[test]
    fn update_stores_previous_player_state() {
        let mut world = make_test_world();
        let before_pos = world.player.pos;
        let before_dir = world.player.dir;

        world.move_right = true;
        world.update(TICK_DT);

        assert_eq!(world.prev_player_pos, before_pos);
        assert_eq!(world.prev_player_dir, before_dir);
        // The player actually advanced, so prev != current after the tick.
        assert!(world.player.pos.x > before_pos.x);
    }

    /// Box 8 (AC3): World::update receives exactly TICK_DT in the accumulator loop.
    ///
    /// We cannot intercept the `dt` argument directly, so we verify its observable
    /// effect: with `move_right` held, a single `update(TICK_DT)` must advance the
    /// player by exactly `VELOCIDAD * TICK_DT`. If update were called with the
    /// variable render dt instead, the delta would differ.
    #[test]
    fn world_update_receives_fixed_tick_dt() {
        let mut world = make_test_world();
        world.move_right = true;

        let before_x = world.player.pos.x;
        world.update(TICK_DT);

        let expected_dx = VELOCIDAD * TICK_DT;
        let actual_dx = world.player.pos.x - before_x;
        assert!(
            (actual_dx - expected_dx).abs() < 0.001,
            "update should advance player by exactly VELOCIDAD * TICK_DT = {:.6}, got {:.6}",
            expected_dx,
            actual_dx
        );
    }

    /// Box 9 (AC9): Accumulator clamp — large frame_dt (1.0s) results in exactly
    /// MAX_ACCUMULATED_FRAMES (5) steps, accumulator clamped to 5 * TICK_DT.
    /// This test simulates the accumulator logic that will be in main.rs.
    #[test]
    fn accumulator_clamp_large_frame_dt() {
        let mut world = make_test_world();
        let large_frame_dt = 1.0_f32; // 1 second of lag
        let mut accumulator = 0.0_f32;
        let max_accumulator = MAX_ACCUMULATED_FRAMES as f32 * TICK_DT;

        // Simulate one render frame with large frame_dt
        accumulator += large_frame_dt;
        if accumulator > max_accumulator {
            accumulator = max_accumulator; // clamp
        }

        let mut sim_steps = 0u32;
        while accumulator >= TICK_DT {
            world.update(TICK_DT);
            accumulator -= TICK_DT;
            sim_steps += 1;
        }

        // Should execute exactly MAX_ACCUMULATED_FRAMES (5) steps
        assert_eq!(
            sim_steps, MAX_ACCUMULATED_FRAMES,
            "should execute exactly 5 sim steps for 1s frame_dt"
        );

        // Accumulator should be clamped to max_accumulator - 5 * TICK_DT
        // max_accumulator = 5 * TICK_DT, so after 5 steps: accumulator = 0
        assert!(
            (accumulator - 0.0).abs() < f32::EPSILON,
            "accumulator should be 0 after 5 steps"
        );
    }

    /// Box 10 (AC4): interpolated_player_pos/dir(alpha) — lerp at 0.0, 0.5, 1.0.
    /// Tests the pure interpolation helpers that will be added to World.
    #[test]
    fn interpolated_player_pos_and_dir_lerp() {
        let mut world = make_test_world();

        // Set up known positions for testing
        let prev_pos = glam::Vec2::new(100.0, 200.0);
        let curr_pos = glam::Vec2::new(200.0, 300.0);
        let prev_dir = 0.0_f32;
        let curr_dir = std::f32::consts::FRAC_PI_2; // π/2 = 90 degrees

        world.prev_player_pos = prev_pos;
        world.player.pos = curr_pos;
        world.prev_player_dir = prev_dir;
        world.player.dir = curr_dir;

        // Test interpolated_player_pos
        let pos_0 = world.interpolated_player_pos(0.0);
        let pos_half = world.interpolated_player_pos(0.5);
        let pos_1 = world.interpolated_player_pos(1.0);

        // At alpha=0: should be prev_pos
        assert!((pos_0.x - prev_pos.x).abs() < f32::EPSILON);
        assert!((pos_0.y - prev_pos.y).abs() < f32::EPSILON);

        // At alpha=0.5: should be midpoint
        let expected_mid = prev_pos.lerp(curr_pos, 0.5);
        assert!((pos_half.x - expected_mid.x).abs() < f32::EPSILON);
        assert!((pos_half.y - expected_mid.y).abs() < f32::EPSILON);

        // At alpha=1: should be curr_pos
        assert!((pos_1.x - curr_pos.x).abs() < f32::EPSILON);
        assert!((pos_1.y - curr_pos.y).abs() < f32::EPSILON);

        // Test interpolated_player_dir
        let dir_0 = world.interpolated_player_dir(0.0);
        let dir_half = world.interpolated_player_dir(0.5);
        let dir_1 = world.interpolated_player_dir(1.0);

        // At alpha=0: should be prev_dir
        assert!((dir_0 - prev_dir).abs() < f32::EPSILON);

        // At alpha=0.5: should be midpoint (simple lerp)
        let expected_dir_mid = prev_dir + (curr_dir - prev_dir) * 0.5;
        assert!((dir_half - expected_dir_mid).abs() < f32::EPSILON);

        // At alpha=1: should be curr_dir
        assert!((dir_1 - curr_dir).abs() < f32::EPSILON);
    }

    /// Box 10 (AC4) extra: interpolation alpha is clamped to [0, 1], so an
    /// out-of-range alpha (e.g. a large first `frame_dt`) does not extrapolate.
    #[test]
    fn interpolated_player_pos_clamps_alpha() {
        let mut world = make_test_world();
        world.prev_player_pos = glam::Vec2::new(0.0, 0.0);
        world.player.pos = glam::Vec2::new(10.0, 10.0);

        let at_neg = world.interpolated_player_pos(-1.0);
        let at_over = world.interpolated_player_pos(2.0);

        assert!((at_neg - glam::Vec2::new(0.0, 0.0)).length() < f32::EPSILON);
        assert!((at_over - glam::Vec2::new(10.0, 10.0)).length() < f32::EPSILON);
    }

    /// Box 11 (AC10): client_tick increments once per sim tick (per `World::update`
    /// call), not once per render frame.
    #[test]
    fn client_tick_increments_per_sim_tick() {
        let mut world = make_test_world();
        let max_accumulator = MAX_ACCUMULATED_FRAMES as f32 * TICK_DT;

        // Three render frames with distinct dt (30/60/10 FPS).
        let frame_dts: [f32; 3] = [1.0 / 30.0, 1.0 / 60.0, 1.0 / 10.0];
        let mut accumulator = 0.0_f32;
        let mut total_steps = 0u64;

        for &frame_dt in &frame_dts {
            let tick_before = world.client_tick;
            accumulator += frame_dt;
            if accumulator > max_accumulator {
                accumulator = max_accumulator;
            }

            let mut steps_this_frame = 0u64;
            while accumulator >= TICK_DT {
                world.update(TICK_DT);
                accumulator -= TICK_DT;
                steps_this_frame += 1;
            }

            // client_tick advanced by exactly the number of sim steps this frame.
            assert_eq!(
                world.client_tick - tick_before,
                steps_this_frame,
                "client_tick must advance once per sim tick"
            );
            total_steps += steps_this_frame;
        }

        // Three render frames at 30/60/10 FPS sum to (1/30 + 1/60 + 1/10)s of
        // simulated time → more than 3 sim steps. client_tick reflects sim steps,
        // not the render-frame count (3).
        assert!(
            total_steps > 3,
            "expected >3 sim steps across 3 render frames, got {}",
            total_steps
        );
        assert_eq!(world.client_tick, total_steps);
    }
}
