use glam::Vec2;

use crate::config::*;
use crate::level::Level;
use crate::rect::Rect;
use crate::traits::Colisionable;
use crate::types::EntityType;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum EnemyState {
    Idle,
    WalkingLeft,
    WalkingRight,
    Jumping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyKind {
    Enemy,
    Potion,
}

#[derive(Debug)]
pub struct Enemy {
    pub pos: Vec2,
    pub vel: Vec2,
    pub w: f32,
    pub h: f32,
    pub grounded: bool,
    pub state: EnemyState,
    pub timer: f32,
    pub alive: bool,
    pub kind: EnemyKind,
    pub id: u8, // Stable enemy ID (room_x * 16 + spawn_index)
}

impl Colisionable for Enemy {
    fn get_rect(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.w, self.h)
    }
}

impl Enemy {
    pub fn new(id: u8, pos: Vec2, kind: EnemyKind) -> Self {
        Self {
            pos,
            vel: Vec2::new(0.0, 0.0),
            w: 30.0,
            h: 38.0,
            grounded: false,
            state: EnemyState::Idle,
            timer: 0.0,
            alive: true,
            kind,
            id,
        }
    }

    pub fn die(&mut self) {
        self.alive = false;
    }

    fn set_rand_state(&mut self, random: u32) {
        self.state = match random % 4 {
            0 => EnemyState::Idle,
            1 => EnemyState::WalkingLeft,
            2 => EnemyState::WalkingRight,
            _ => EnemyState::Jumping,
        };
    }

    fn apply_physics(&mut self, dt: f32, nivel: &Level) {
        self.vel.y += GRAVEDAD * dt;

        // X axis
        self.pos.x += self.vel.x;
        if nivel.map_colision(self) {
            self.pos.x -= self.vel.x;
        }

        // Y axis
        self.pos.y += self.vel.y;
        if nivel.map_colision(self) {
            if self.vel.y > 0.0 {
                self.grounded = true;
            }
            self.pos.y -= self.vel.y;
            self.vel.y = 0.0;
        } else {
            self.grounded = false;
        }
    }

    pub fn update(&mut self, dt: f32, nivel: &Level, random: u32) {
        self.timer += dt;

        if self.timer >= 0.1 {
            self.set_rand_state(random);
            self.timer = 0.0;
        }

        match self.state {
            EnemyState::Idle => self.vel.x = 0.0,
            EnemyState::WalkingLeft => self.vel.x = -VELOCIDAD * dt,
            EnemyState::WalkingRight => self.vel.x = VELOCIDAD * dt,
            EnemyState::Jumping => {
                if self.grounded {
                    self.vel.y = SALTO_FUERZA * dt;
                    self.grounded = false;
                }
            }
        }

        self.apply_physics(dt, nivel);
    }
}

/// Iterate one room's `data` grid and build the enemies it declares.
/// Pure: no `World`, no `NetworkClient`, no socket, no thread.
/// Assigns stable IDs: `id = room_x * 16 + spawn_index` (spawn_index is 0-based row-major order).
/// The `room_x` parameter is required for correct ID assignment.
pub fn enemies_from_data(data: &[Vec<EntityType>], room_x: u8) -> Vec<Enemy> {
    let mut enemies = Vec::new();
    let mut spawn_index = 0u8;

    for (y, row) in data.iter().enumerate() {
        for (x, &cell) in row.iter().enumerate() {
            let pos = Vec2::new(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE);

            match cell {
                EntityType::EnemySpawn | EntityType::PotionSpawn => {
                    let kind = match cell {
                        EntityType::EnemySpawn => EnemyKind::Enemy,
                        EntityType::PotionSpawn => EnemyKind::Potion,
                        _ => unreachable!(),
                    };
                    let id = room_x * 16 + spawn_index;
                    enemies.push(Enemy::new(id, pos, kind));
                    spawn_index += 1;
                }
                _ => {}
            }
        }
    }

    enemies
}

#[cfg(test)]
mod tests {
    use super::{Enemy, EnemyKind, EnemyState, enemies_from_data};
    use crate::config::TILE_SIZE;
    use crate::types::EntityType;
    use glam::Vec2;

    #[test]
    fn enemies_from_data_spawns_enemy_and_potion() {
        let data = vec![
            vec![EntityType::EnemySpawn, EntityType::Empty],
            vec![EntityType::BossSpawn, EntityType::PotionSpawn],
        ];
        let enemies = enemies_from_data(&data, 0);

        assert_eq!(enemies.len(), 2);
        assert_eq!(enemies[0].pos, Vec2::new(0.0, 0.0));
        assert_eq!(enemies[1].pos, Vec2::new(TILE_SIZE, TILE_SIZE));
        assert_eq!(enemies[0].kind, EnemyKind::Enemy);
        assert_eq!(enemies[1].kind, EnemyKind::Potion);
        assert_eq!(enemies[0].id, 0); // room_x=0, spawn_index=0
        assert_eq!(enemies[1].id, 1); // room_x=0, spawn_index=1
    }

    #[test]
    fn enemies_from_data_skips_boss_and_empty() {
        let data = vec![vec![EntityType::BossSpawn, EntityType::Empty]];
        assert_eq!(enemies_from_data(&data, 0).len(), 0);

        let data = vec![vec![EntityType::Empty, EntityType::Empty]];
        assert_eq!(enemies_from_data(&data, 0).len(), 0);
    }

    #[test]
    fn enemies_from_data_count_matches_spawn_cells() {
        let data = vec![
            vec![
                EntityType::EnemySpawn,
                EntityType::PotionSpawn,
                EntityType::Empty,
            ],
            vec![
                EntityType::BossSpawn,
                EntityType::EnemySpawn,
                EntityType::PotionSpawn,
            ],
        ];
        // 2 EnemySpawn + 2 PotionSpawn -> 4 enemies; BossSpawn and Empty are skipped.
        let enemies = enemies_from_data(&data, 0);
        assert_eq!(enemies.len(), 4);
        // Check IDs are sequential
        assert_eq!(enemies[0].id, 0);
        assert_eq!(enemies[1].id, 1);
        assert_eq!(enemies[2].id, 2);
        assert_eq!(enemies[3].id, 3);
    }

    #[test]
    fn enemy_set_rand_state_maps_random_mod_4() {
        let mut enemy = Enemy::new(0, Vec2::new(0.0, 0.0), EnemyKind::Enemy);

        enemy.set_rand_state(0);
        assert_eq!(enemy.state, EnemyState::Idle);
        enemy.set_rand_state(1);
        assert_eq!(enemy.state, EnemyState::WalkingLeft);
        enemy.set_rand_state(2);
        assert_eq!(enemy.state, EnemyState::WalkingRight);
        enemy.set_rand_state(3);
        assert_eq!(enemy.state, EnemyState::Jumping);
        enemy.set_rand_state(4);
        assert_eq!(enemy.state, EnemyState::Idle);
        enemy.set_rand_state(255);
        assert_eq!(enemy.state, EnemyState::Jumping);
    }
}
