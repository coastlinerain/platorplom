use std::{collections::HashMap, iter::Map};

struct ServerWorld {
    players: HashMap<u64, PlayerData>,
    map: Map,
}

impl ServerWorld {
    fn update(&mut self) {}
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
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                move_right: true,
                ..Default::default()
            },
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
        );
        world.apply_input(
            0,
            0,
            0,
            PlayerInput {
                shoot: true,
                ..Default::default()
            },
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
        );

        world.update(tick_dt());

        assert!(world.players()[&0].pos.x > 100.0);
    }
}
