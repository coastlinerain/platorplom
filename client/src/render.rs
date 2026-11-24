// Client-side rendering for the `game` entity types.
//
// The `game` crate owns the simulation (state + physics) and must have no
// macroquad dependency, so the `draw` methods that used to live on each entity
// are expressed here as impls of a *local* `Drawable` trait. This is allowed by
// the orphan rule: `Drawable` is defined in this crate, so we may implement it
// for the foreign `game` types. The `EnemyKind -> Color` mapping lives here too,
// keeping `Color` (a macroquad type) out of `game`.

use game::config::TILE_SIZE;
use game::entities::bullet::Bullet;
use game::entities::enemy::{Enemy, EnemyKind};
use game::entities::nemesis::Nemesis;
use game::entities::player::Player;
use game::level::Level;
use game::types::TileType;
use macroquad::prelude::*;

/// Trait implemented for every `game` type that can be drawn on screen.
pub trait Drawable {
    fn draw(&self);
}

impl Drawable for Level {
    fn draw(&self) {
        for (y, row) in self.grid.iter().enumerate() {
            for (x, &tile) in row.iter().enumerate() {
                if tile == TileType::Wall {
                    draw_rectangle(
                        x as f32 * TILE_SIZE,
                        y as f32 * TILE_SIZE,
                        TILE_SIZE - 1.0,
                        TILE_SIZE - 1.0,
                        GRAY,
                    );
                }
            }
        }
    }
}

impl Drawable for Player {
    fn draw(&self) {
        draw_rectangle(self.pos.x, self.pos.y, self.w, self.h, RED);
    }
}

impl Drawable for Enemy {
    fn draw(&self) {
        let color = match self.kind {
            EnemyKind::Enemy => BLUE,
            EnemyKind::Potion => GREEN,
        };
        draw_rectangle(self.pos.x, self.pos.y, self.w, self.h, color);
    }
}

impl Drawable for Nemesis {
    fn draw(&self) {
        draw_rectangle(self.pos.x, self.pos.y, self.w, self.h, ORANGE);

        // Draw a small indicator of which way it is facing.
        let eye_x = if self.last_dir > 0.0 {
            self.pos.x + 20.0
        } else {
            self.pos.x + 5.0
        };
        draw_rectangle(eye_x, self.pos.y + 10.0, 5.0, 5.0, WHITE);

        // Draw the player's ID above its head.
        draw_text(
            format!("P: {}", self.id),
            self.pos.x,
            self.pos.y - 10.0,
            15.0,
            WHITE,
        );
    }
}

impl Drawable for Bullet {
    fn draw(&self) {
        draw_circle(self.pos.x, self.pos.y, 5.0, YELLOW);
    }
}
