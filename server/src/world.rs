use game::entities::player::{Player, PlayerInput};
use game::level::Level;
use game::maps::Map;
use std::collections::HashMap;

pub struct ServerWorld {
    players: HashMap<u8, Player>,
    pending: HashMap<u8, PlayerInput>,
    coords: HashMap<u8, (u8, u8)>,
    levels: Vec<Vec<Level>>,
}

impl ServerWorld {
    pub fn new() -> Self {
        let map_data = Map::new();

        let levels: Vec<Vec<Level>> = map_data
            .grids
            .into_iter()
            .zip(map_data.data)
            .map(|(grid_row, data_row)| {
                grid_row
                    .into_iter()
                    .zip(data_row)
                    .map(|(grid, data)| Level::new(grid, data))
                    .collect()
            })
            .collect();

        Self {
            players: HashMap::new(),
            pending: HashMap::new(),
            coords: HashMap::new(),
            levels,
        }
    }

    pub fn spawn(&mut self, id: u8) {
        self.players.insert(id, Player::new(id));
        self.pending.insert(id, PlayerInput::default());
        self.coords.insert(id, (0, 0));
    }

    pub fn remove(&mut self, id: u8) {
        self.players.remove(&id);
        self.pending.remove(&id);
        self.coords.remove(&id);
    }

    pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput) {
        // coords: (level_x, level_y) = (column, row)
        self.coords.insert(id, (x, y));

        let pending = self.pending.entry(id).or_default();

        // Held flags overwrite (latest-wins)
        pending.move_left = input.move_left;
        pending.move_right = input.move_right;

        // Edge flags OR-accumulate (latched until consumed by update)
        pending.jump |= input.jump;
        pending.shoot |= input.shoot;
    }

    /// Runs one authoritative tick. Returns `(id, dir)` for every player whose
    /// `shoot` edge was latched this tick.
    pub fn update(&mut self, dt: f32) -> Vec<(u8, f32)> {
        let mut shots = Vec::new();

        for (&id, player) in &mut self.players {
            let (cx, cy) = self.coords.get(&id).copied().unwrap_or((0, 0));

            // Row-major: levels[row_y][col_x]. Client-trusted coords use `.get()`
            // fallback so a bogus (255,255) does not index-panic the server.
            let level = self
                .levels
                .get(cy as usize)
                .and_then(|row| row.get(cx as usize))
                .unwrap_or(&self.levels[0][0]);

            let pending = self.pending.entry(id).or_default();

            // Build the input for this tick: held flags from the accumulator,
            // jump from the accumulator, shoot forced false (server never spawns
            // bullets — D6).
            let input = PlayerInput {
                move_left: pending.move_left,
                move_right: pending.move_right,
                jump: pending.jump,
                shoot: false,
            };

            player.update(dt, level, &input);

            // Clear latched edge flags now that they've been consumed.
            pending.jump = false;

            // If shoot was latched, record the shot for the rebroadcast (id, dir).
            if pending.shoot {
                shots.push((id, player.dir));
            }
            pending.shoot = false;
        }

        shots
    }

    pub fn players(&self) -> &HashMap<u8, Player> {
        &self.players
    }

    pub fn coords(&self) -> &HashMap<u8, (u8, u8)> {
        &self.coords
    }
}

// NOTE (tester, Phase 2b): the tests below are written against the plan's
// revived `ServerWorld` public API (`new` / `spawn` / `apply_input` / `update` /
// `players` / `coords`) plus the private `pending` accumulator. They do NOT
// compile until the implementer lands Box 2 (ServerWorld skeleton + `mod world;`)
// — the current file is an orphaned stub (`iter::Map`, undefined `PlayerData`).
#[cfg(test)]
mod tests {
    use super::ServerWorld;
    use game::config::TICK_HZ;
    use game::entities::player::PlayerInput;

    fn tick_dt() -> f32 {
        1.0 / TICK_HZ as f32
    }

    #[test]
    fn input_held_flags_latest_wins() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_left: true,
                ..Default::default()
            },
            0,
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        // Held flags overwrite: the latest input (move_right) wins.
        assert!(world.pending[&0].move_right);
        assert!(!world.pending[&0].move_left);

        // Latest-wins holds through the tick: the authoritative player moves right.
        world.update(tick_dt());
        assert!(world.players()[&0].pos.x > 100.0);
    }

    #[test]
    fn input_edge_flags_latch_and_clear() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );

        // Two shoot edges OR-latch into a single shot: exactly one is consumed.
        let shots = world.update(tick_dt());
        assert_eq!(shots, vec![(0, 1.0)]);

        // The latch is cleared after the tick consumes it.
        assert!(!world.pending[&0].shoot);

        // No re-fire on the next tick without a fresh input.
        assert!(world.update(tick_dt()).is_empty());
    }

    #[test]
    fn input_edge_flag_not_lost_between_ticks() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Tick 1: no inputs yet.
        world.update(tick_dt());

        // The shoot edge arrives between ticks.
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
            0,
        );

        // Tick 2: the latched edge fires instead of being dropped.
        assert_eq!(world.update(tick_dt()), vec![(0, 1.0)]);
    }

    #[test]
    fn apply_input_update_drives_authoritative_position() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        assert!(world.players()[&0].pos.x > 100.0);
        assert_eq!(world.players()[&0].dir, 1.0);
    }

    #[test]
    fn apply_input_update_jump_moves_up_when_grounded() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Set grounded so the jump is accepted (Player::update only jumps when grounded).
        world.players.get_mut(&0).unwrap().grounded = true;

        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                jump: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        // Net vertical velocity (SALTO_FUERZA + GRAVEDAD) * dt is negative.
        assert!(world.players()[&0].pos.y < 100.0);
    }

    #[test]
    fn apply_input_out_of_range_coords_fall_back_to_origin() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // A client-reported (255,255) room must not index-panic; it falls back
        // to the origin level where the (100,100) spawn is in-bounds.
        world.apply_input(
            0,
            255,
            255,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
            0,
        );

        world.update(tick_dt());

        assert!(world.players()[&0].pos.x > 100.0);
    }

    // --- max_seq tests (Phase 2c) ---

    /// AC 28 — max_seq is updated on apply_input with wrapping-aware comparison.
    /// Older seq values are ignored; newer seq values overwrite.
    #[test]
    fn max_seq_updated_on_apply_input() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // Initial state: max_seq is 0 for a newly-spawned player.
        assert_eq!(world.max_seq().get(&0).copied().unwrap_or(0), 0);

        // Apply input with seq 5.
        world.apply_input(0, 0, 0, PlayerInput::default(), 5);
        assert_eq!(world.max_seq()[&0], 5);

        // Older seq is ignored.
        world.apply_input(0, 0, 0, PlayerInput::default(), 3);
        assert_eq!(world.max_seq()[&0], 5);

        // Newer seq overwrites.
        world.apply_input(0, 0, 0, PlayerInput::default(), 10);
        assert_eq!(world.max_seq()[&0], 10);
    }

    /// AC 29 — max_seq starts at 0 for newly-spawned players.
    /// The server reports 0 for players who have not sent any inputs yet.
    #[test]
    fn playerpos_broadcast_includes_last_seq() {
        let mut world = ServerWorld::new();
        world.spawn(0);

        // New player has max_seq == 0.
        assert_eq!(world.max_seq().get(&0).copied().unwrap_or(0), 0);

        // After applying an input with seq 7, max_seq is updated.
        world.apply_input(0, 0, 0, PlayerInput::default(), 7);
        assert_eq!(world.max_seq()[&0], 7);

        // A second player starts at 0.
        world.spawn(1);
        assert_eq!(world.max_seq().get(&1).copied().unwrap_or(0), 0);
    }
}
