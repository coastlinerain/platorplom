use crate::{
    config::TILE_SIZE,
    rect::Rect,
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
        self.width as f32 * TILE_SIZE
    }
    pub fn pixel_height(&self) -> f32 {
        self.height as f32 * TILE_SIZE
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
}

#[cfg(test)]
mod tests {
    use super::Level;
    use crate::rect::Rect;
    use crate::types::{EntityType, TileType};

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

    /// 4 rows × 4 cols, every tile `Empty` except `grid[3][1]` (the row *below*
    /// the ray row used by G3).
    fn wall_below_row_level() -> Level {
        let mut grid = vec![vec![TileType::Empty; 4]; 4];
        grid[3][1] = TileType::Wall;
        Level::new(grid, vec![vec![EntityType::Empty; 4]; 4])
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

    // `colision` delegates to `solid_point`, which treats coordinates outside the
    // grid as solid (its `else` branch returns true). This is the OPPOSITE of
    // `solid_in_rect`, which treats out-of-bounds as not solid — an existing
    // asymmetry, locked here rather than fixed.
    #[test]
    fn colision_out_of_bounds_is_solid() {
        let level = wall_ring_level();
        assert!(level.colision(200.0, 40.0, 10.0, 10.0));
    }

    // `colision` samples six points with a ±2.0 px horizontal inset, so a rect
    // whose only wall overlap is a corner (2 px or less) is NOT detected. Here the
    // single-wall fixture's top-left corner sits inside that inset, so all six
    // sample points miss the wall.
    #[test]
    fn colision_ignores_two_px_corner_inset() {
        let level = single_wall_cell_level();
        assert!(!level.colision(38.0, 30.0, 42.0, 60.0));
    }

    // --- solid_between tests (spec 2, authoritative player death) ---
    //
    // `solid_in_rect` is not usable for a 1-px-high ray: at `y = 119`
    // (`TILE_SIZE = 40`) a 1-px rect spans rows 2 AND 3. `solid_between` scans
    // only the single row containing the ray (G3 proves the regression).

    #[test]
    fn solid_between_wall_between_true() {
        let level = wall_ring_level();
        // Row 1: col 1 is Empty, col 2 is Wall (x ∈ [80, 120]).
        assert!(level.solid_between(50.0, 110.0, 60.0));
        // Order-insensitive: the caller may pass x1 < x0.
        assert!(level.solid_between(110.0, 50.0, 60.0));
    }

    #[test]
    fn solid_between_stops_before_wall_false() {
        let level = wall_ring_level();
        // Only col 1 is touched, and it is Empty.
        assert!(!level.solid_between(45.0, 75.0, 60.0));
    }

    #[test]
    fn solid_between_ignores_adjacent_row_wall() {
        let level = wall_below_row_level();
        // y = 119 → gy = 2, which is empty; the wall lives in row 3.
        assert!(!level.solid_between(0.0, 80.0, 119.0));
        // Regression proof: a 1-px rect at y = 119 spans rows 2 and 3, so
        // `solid_in_rect` WOULD see grid[3][1] — hence solid_between exists.
        assert!(level.solid_in_rect(Rect::new(0.0, 119.0, 80.0, 1.0)));
    }

    #[test]
    fn solid_between_out_of_grid_row_not_solid() {
        let level = wall_ring_level();
        // gy < 0.
        assert!(!level.solid_between(50.0, 110.0, -10.0));
        // gy >= height (4 rows → y = 1000 is row 25).
        assert!(!level.solid_between(50.0, 110.0, 1000.0));
    }

    #[test]
    fn solid_between_out_of_grid_columns_not_solid() {
        let level = single_wall_cell_level();
        // Cols >= width (3) are ignored; col 2 at row 2 is Empty.
        assert!(!level.solid_between(100.0, 400.0, 100.0));
        // Negative cols are ignored; col 0 at row 2 is Empty.
        assert!(!level.solid_between(-100.0, 10.0, 100.0));
    }
}
