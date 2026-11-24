use crate::camera::GameCamera;
use crate::network::packet_io::PacketIO;
use crate::render::Drawable;
use crate::world::lib::{
    check_level_transitions, check_player_death, update_enemy_bullets_position,
};

use crate::network::interpolation::RemoteInterp;
use crate::network::prediction::InputRingBuffer;
use crate::network::world::update_network;
use game::entities::bullet::Bullet;
use game::entities::enemy::Enemy;
use game::entities::nemesis::Nemesis;
use game::entities::player::Player;
use game::level::Level;
use game::maps::Map;
use macroquad::prelude::*;
use std::collections::HashMap;
use std::fmt;

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
    pub bullets: Vec<Bullet>, // own bullets (relocated from Player; server-authoritative)
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

        let enemies = game::entities::enemy::enemies_from_data(&levels[0][0].data);

        Self {
            id: None,
            player: Player::new(0),
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
            pressed_jump: false,
            pressed_shoot: false,
            move_left: false,
            move_right: false,
        }
    }

    pub fn update(&mut self, dt: f32) {
        // Run transition check AFTER bullet-vs-enemy collision so bullets
        // can hit enemies on the frame they're fired, even if the player
        // crosses a room boundary on the same frame.
        update_enemy_bullets_position(self, dt);
        check_player_death(self);
        {
            let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];

            // Sample edge keys ONCE per frame (is_key_pressed is stateful — first call consumes).
            let pressed_jump = is_key_pressed(KeyCode::Space);
            let pressed_shoot = is_key_pressed(KeyCode::Z);

            // Sample held keys — stored on World so update_network can read them.
            self.move_left = is_key_down(KeyCode::Left);
            self.move_right = is_key_down(KeyCode::Right);

            // --- CLIENT PREDICTION: simulate local player immediately ---
            let local_input = game::entities::player::PlayerInput {
                move_left: self.move_left,
                move_right: self.move_right,
                jump: pressed_jump,
                shoot: false, // bullet spawn is separate
            };
            self.player.update(dt, level, &local_input);

            // Store edge keys so update_network can read them (is_key_pressed is consumed).
            self.pressed_jump = pressed_jump;
            self.pressed_shoot = pressed_shoot;

            // Spawn own bullet from the authoritative dir when Z is pressed.
            if pressed_shoot {
                self.bullets.push(Bullet::new(
                    self.player.pos + glam::Vec2::new(self.player.w / 2.0, self.player.h / 2.0),
                    self.player.dir,
                    self.player.id,
                ));
            }

            // Update own bullets.
            for bullet in &mut self.bullets {
                bullet.update(dt, level);
            }

            // Collide own bullets with local enemies.
            for bullet in &mut self.bullets {
                for enemy in &mut self.enemies {
                    if bullet.check_collision(enemy) {
                        bullet.active = false;
                        enemy.die();
                        println!("Enemy hit!");
                    }
                }
            }

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

    pub fn draw(&self) {
        self.camera.apply();

        let level = &self.levels_matrix[self.current_coords.1][self.current_coords.0];
        level.draw();

        // Remote players: interpolated when available, raw fallback otherwise.
        for (id, nemesis) in &self.other_players {
            if nemesis.coords == self.current_coords
                && let Some(interp) = self.other_interp.get(id)
                && let Some((ix, iy, idir)) = interp.interpolate(nemesis)
            {
                crate::render::draw_nemesis_at(nemesis, ix, iy, idir);
            } else if nemesis.coords == self.current_coords {
                nemesis.draw();
            }
        }

        // Own player.
        self.player.draw();

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
