use macroquad::prelude::*;

use crate::{
    config::TILE_SIZE,
    traits::Colisionable,
    types::{EntityType, TileType},
};

#[derive(PartialEq, Clone, Debug)]
pub struct Level {
    pub grid: Vec<Vec<TileType>>,
    pub width: usize,
    pub height: usize,
    pub data: Vec<Vec<EntityType>>,
}

impl Level {
    pub fn new(grid: Vec<Vec<TileType>>, data: Vec<Vec<EntityType>>) -> Self {
        let height = grid.len();
        let width = if height > 0 { grid[0].len() } else { 0 };
        Self {
            grid,
            width,
            height,
            data,
        }
    }

    pub fn pixel_width(&self) -> f32 {
        self.width as f32 * crate::config::TILE_SIZE
    }
    pub fn pixel_height(&self) -> f32 {
        self.height as f32 * crate::config::TILE_SIZE
    }

    pub fn map_colision<T: Colisionable>(&self, objeto: &T) -> bool {
        let r = objeto.get_rect();
        self.colision(r.x, r.y, r.w, r.h)
    }

    fn solid_point(&self, x: f32, y: f32) -> bool {
        let gx = (x / TILE_SIZE) as usize;
        let gy = (y / TILE_SIZE) as usize;
        if gx < self.width && gy < self.height {
            return self.grid[gy][gx] == TileType::Wall;
        }
        true
    }

    pub fn colision(&self, rect_x: f32, rect_y: f32, w: f32, h: f32) -> bool {
        let puntos = [
            (rect_x + 2.0, rect_y),
            (rect_x + w - 2.0, rect_y),
            (rect_x + 2.0, rect_y + h),
            (rect_x + w - 2.0, rect_y + h),
            (rect_x, rect_y + h / 2.0),
            (rect_x + w, rect_y + h / 2.0),
        ];
        puntos.iter().any(|&(px, py)| self.solid_point(px, py))
    }

    pub fn grounded(&self, rect: Rect) -> bool {
        let chequeo_suelo = Rect::new(rect.x, rect.y + 1.0, rect.w, rect.h);
        self.solid_in_rect(chequeo_suelo)
    }

    pub fn solid_in_rect(&self, r: Rect) -> bool {
        let x_start = (r.x / TILE_SIZE).floor() as i32;
        let x_end = ((r.x + r.w) / TILE_SIZE).floor() as i32;
        let y_start = (r.y / TILE_SIZE).floor() as i32;
        let y_end = ((r.y + r.h) / TILE_SIZE).floor() as i32;

        for gy in y_start..=y_end {
            for gx in x_start..=x_end {
                if gx >= 0
                    && gx < self.width as i32
                    && gy >= 0
                    && gy < self.height as i32
                    && self.grid[gy as usize][gx as usize] == TileType::Wall
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn draw(&self) {
        for (y, fila) in self.grid.iter().enumerate() {
            for (x, &tile) in fila.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use super::Level;
    use crate::types::{EntityType, TileType};
    use macroquad::prelude::Rect;

    fn wall_ring_level() -> Level {
        let grid = vec![
            vec![TileType::Wall, TileType::Wall, TileType::Wall],
            vec![TileType::Wall, TileType::Empty, TileType::Wall],
            vec![TileType::Wall, TileType::Empty, TileType::Wall],
            vec![TileType::Wall, TileType::Wall, TileType::Wall],
        ];
        Level::new(grid, vec![vec![EntityType::Empty; 3]; 4])
    }

    fn single_wall_cell_level() -> Level {
        let grid = vec![
            vec![TileType::Wall, TileType::Empty, TileType::Empty],
            vec![TileType::Empty, TileType::Empty, TileType::Empty],
            vec![TileType::Empty, TileType::Empty, TileType::Empty],
        ];
        Level::new(grid, vec![vec![EntityType::Empty; 3]; 3])
    }

    #[test]
    fn solid_in_rect_overlaps_wall() {
        let level = wall_ring_level();
        assert!(level.solid_in_rect(Rect::new(0.0, 0.0, 40.0, 40.0)));
    }

    #[test]
    fn solid_in_rect_empty_interior() {
        let level = wall_ring_level();
        assert!(!level.solid_in_rect(Rect::new(45.0, 80.0, 10.0, 10.0)));
    }

    #[test]
    fn solid_in_rect_straddles_boundary() {
        let level = wall_ring_level();
        assert!(level.solid_in_rect(Rect::new(35.0, 85.0, 10.0, 10.0)));
    }

    #[test]
    fn solid_in_rect_out_of_bounds_not_solid() {
        let level = wall_ring_level();
        assert!(!level.solid_in_rect(Rect::new(-100.0, -100.0, 50.0, 50.0)));
    }

    #[test]
    fn grounded_within_one_px_above_floor() {
        let level = wall_ring_level();
        assert!(level.grounded(Rect::new(45.0, 79.0, 10.0, 40.0)));
    }

    #[test]
    fn grounded_floating_above() {
        let level = wall_ring_level();
        assert!(!level.grounded(Rect::new(45.0, 40.0, 10.0, 40.0)));
    }

    #[test]
    fn colision_empty_area() {
        let level = wall_ring_level();
        assert!(!level.colision(45.0, 80.0, 10.0, 10.0));
    }

    #[test]
    fn colision_overlaps_wall() {
        let level = wall_ring_level();
        assert!(level.colision(0.0, 0.0, 40.0, 40.0));
    }

    #[test]
    fn colision_out_of_bounds_is_solid() {
        let level = wall_ring_level();
        assert!(level.colision(200.0, 40.0, 10.0, 10.0));
    }

    #[test]
    fn colision_ignores_two_px_corner_inset() {
        let level = single_wall_cell_level();
        assert!(!level.colision(38.0, 30.0, 42.0, 60.0));
    }
}
