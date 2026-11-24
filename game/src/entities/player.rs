use glam::Vec2;

use crate::config::*;
use crate::entities::bullet::Bullet;
use crate::level::Level;
use crate::rect::Rect;
use crate::traits::Colisionable;

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerInput {
    /// held
    pub move_left: bool,
    /// held
    pub move_right: bool,
    /// pressed (edge)
    pub jump: bool,
    /// pressed (edge)
    pub shoot: bool,
}

#[derive(Debug)]
pub struct Player {
    pub id: u8,
    pub pos: Vec2,
    pub vel: Vec2,
    pub w: f32,
    pub h: f32,
    pub grounded: bool,
    pub dir: f32,
    pub bullets: Vec<Bullet>,
}

impl Colisionable for Player {
    fn get_rect(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.w, self.h)
    }
}

impl Player {
    pub fn new(id: u8) -> Self {
        Self {
            id,
            pos: Vec2::new(100.0, 100.0),
            vel: Vec2::new(0.0, 0.0),
            w: 30.0,
            h: 38.0,
            grounded: false,
            dir: 1.0,
            bullets: Vec::new(),
        }
    }

    pub fn update(&mut self, dt: f32, nivel: &Level, input: &PlayerInput) {
        self.vel.x = 0.0;
        if input.move_right {
            self.vel.x = VELOCIDAD * dt;
        }
        if input.move_left {
            self.vel.x = -VELOCIDAD * dt;
        }

        if input.jump && self.grounded {
            self.vel.y = SALTO_FUERZA * dt;
        }

        // direction
        if input.move_right {
            self.dir = 1.0;
        }
        if input.move_left {
            self.dir = -1.0;
        }
        // shoot
        if input.shoot {
            self.bullets.push(Bullet::new(
                self.pos + Vec2::new(self.w / 2.0, self.h / 2.0),
                self.dir,
                self.id,
            ));
        }
        // update bullets
        self.bullets.iter_mut().for_each(|b| b.update(dt, nivel));

        self.vel.y += GRAVEDAD * dt;

        // X axis
        self.pos.x += self.vel.x;
        if nivel.colision(self.pos.x, self.pos.y, self.w, self.h) {
            self.pos.x -= self.vel.x;
        }

        // Y axis
        self.pos.y += self.vel.y;
        if nivel.colision(self.pos.x, self.pos.y, self.w, self.h) {
            self.pos.y -= self.vel.y;
            self.vel.y = 0.0;
        }

        self.grounded = nivel.grounded(self.get_rect());

        self.bullets.retain(|b| b.active);
    }
}

#[cfg(test)]
mod tests {
    use super::{Player, PlayerInput};
    use crate::config::BULLET_SPEED;
    use crate::level::Level;
    use crate::types::{EntityType, TileType};
    use glam::Vec2;

    // NOTE: the fixture uses a 12x12 empty grid (480x480 px) so the player's
    // (100,100) spawn with w=30/h=38 is fully in bounds. A 3x3 grid (120x120 px)
    // would leave the player out of bounds, which `colision` (via `solid_point`)
    // treats as solid — permanently stuck, rolling back every move/jump/shoot.
    fn empty_level() -> Level {
        Level::new(
            vec![vec![TileType::Empty; 12]; 12],
            vec![vec![EntityType::Empty; 12]; 12],
        )
    }

    #[test]
    fn player_input_move_right_moves_right() {
        let mut player = Player::new(0);
        let nivel = empty_level();
        let input = PlayerInput {
            move_right: true,
            ..Default::default()
        };
        player.update(1.0 / 60.0, &nivel, &input);

        assert!(player.pos.x > 100.0);
        assert_eq!(player.dir, 1.0);
    }

    #[test]
    fn player_input_move_left_moves_left() {
        let mut player = Player::new(0);
        let nivel = empty_level();
        let input = PlayerInput {
            move_left: true,
            ..Default::default()
        };
        player.update(1.0 / 60.0, &nivel, &input);

        assert_eq!(player.dir, -1.0);
        assert!(player.pos.x < 100.0);
    }

    #[test]
    fn player_input_jump_when_grounded_moves_up() {
        let mut player = Player::new(0);
        player.grounded = true;
        let nivel = empty_level();
        let input = PlayerInput {
            jump: true,
            ..Default::default()
        };
        player.update(1.0 / 60.0, &nivel, &input);

        assert!(player.pos.y < 100.0);
    }

    #[test]
    fn player_input_jump_ignored_when_airborne() {
        let mut player = Player::new(0);
        player.grounded = false;
        let nivel = empty_level();
        let input = PlayerInput {
            jump: true,
            ..Default::default()
        };
        player.update(1.0 / 60.0, &nivel, &input);

        assert!(player.pos.y > 100.0);
    }

    #[test]
    fn player_input_shoot_spawns_bullet() {
        let mut player = Player::new(0);
        let nivel = empty_level();
        let input = PlayerInput {
            shoot: true,
            ..Default::default()
        };
        player.update(1.0 / 60.0, &nivel, &input);

        assert_eq!(player.bullets.len(), 1);
        assert_eq!(player.bullets[0].owner_id, 0);
        assert_eq!(player.bullets[0].vel, Vec2::new(BULLET_SPEED, 0.0));
    }
}
