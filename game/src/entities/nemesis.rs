use glam::Vec2;

#[derive(Debug)]
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,
    pub w: f32,
    pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
}

impl Nemesis {
    pub fn new(id: u8, pos: Vec2) -> Self {
        Self {
            id,
            pos,
            w: 30.0,
            h: 38.0,
            last_dir: 1.0,
            coords: (0, 0),
        }
    }
}
