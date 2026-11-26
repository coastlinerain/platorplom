use glam::Vec2;

#[derive(Debug)]
pub struct Nemesis {
    pub id: u8,
    pub pos: Vec2,
    pub w: f32,
    pub h: f32,
    pub last_dir: f32,
    pub coords: (usize, usize),
    // --- interpolation fields ---
    /// Ring of 5 position snapshots: oldest at index 0, newest at index 4
    /// (or empty slots). Each snapshot is `(x, y, dir)`.
    pub interp_snapshots: [Option<(f32, f32, f32)>; 5],
    /// Number of valid snapshots (0..5).
    pub interp_len: u8,
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
            interp_snapshots: [None, None, None, None, None],
            interp_len: 0,
        }
    }

    /// Push a new `(x, y, dir)` snapshot into the 5-entry ring.
    /// On the 6th push, the oldest is evicted (shift [1]→[0], [2]→[1], [3]→[2], [4]→[3], put new at [4]).
    pub fn push_interp_snapshot(&mut self, x: f32, y: f32, dir: f32) {
        if self.interp_len < 5 {
            // Still filling the buffer
            self.interp_snapshots[self.interp_len as usize] = Some((x, y, dir));
            self.interp_len += 1;
        } else {
            // Buffer full: shift left by 1, put new at index 4
            self.interp_snapshots[0] = self.interp_snapshots[1];
            self.interp_snapshots[1] = self.interp_snapshots[2];
            self.interp_snapshots[2] = self.interp_snapshots[3];
            self.interp_snapshots[3] = self.interp_snapshots[4];
            self.interp_snapshots[4] = Some((x, y, dir));
            // interp_len stays 5
        }
    }
}
