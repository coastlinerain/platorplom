use glam::Vec2;

use crate::config::BULLET_SPEED;
use crate::level::Level;
use crate::rect::Rect;
use crate::traits::{Colisionable, detect_collision};

#[derive(Debug)]
pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub active: bool,
    pub owner_id: u8,
}

impl Colisionable for Bullet {
    fn get_rect(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, 5.0, 5.0)
    }
}

impl Bullet {
    pub fn new(pos: Vec2, dir_x: f32, id: u8) -> Self {
        Self {
            pos,
            vel: Vec2::new(dir_x * BULLET_SPEED, 0.0),
            active: true,
            owner_id: id,
        }
    }

    pub fn check_collision<T: Colisionable>(&self, target: &T) -> bool {
        detect_collision(self, target)
    }

    pub fn update(&mut self, dt: f32, nivel: &Level) {
        self.pos += self.vel * dt;
        if nivel.map_colision(self) {
            self.active = false;
        }
    }
}
