use crate::world::game::World;
use game::entities::player::Player;

pub fn update_remote_bullets_position(world: &mut World, dt: f32) {
    let current_room = world.current_coords;
    let level = &world.levels_matrix[current_room.1][current_room.0];

    world.remote_bullets.retain_mut(|bullet| {
        if world.other_players.contains_key(&bullet.owner_id) {
            bullet.update(dt, level);
            bullet.active
        } else {
            false
        }
    });
}
/// Applies a server-authoritative respawn (`PlayerDied` naming our own id).
/// Rebuilds the local player from the canonical spawn so the next `PlayerPos`
/// reconciles cleanly; never sends a packet.
pub fn respawn_local(world: &mut World) {
    println!("You died! Returning to start...");
    world.current_coords = (0, 0);
    // Match the server's `Player::new` respawn exactly (pos/vel/grounded/dir).
    let id = world.player.id;
    world.player = Player::new(id);
    // Snap the interpolation source to the respawn position so the next
    // `draw(alpha)` renders at the spawn point instead of lerping from a
    // stale pre-death position (defensive — the prev-state store in
    // `World::update` already runs after this, but keep the invariant local).
    world.prev_player_pos = world.player.pos;
    world.prev_player_dir = world.player.dir;
    // Clear bullet tracking on death/respawn
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();
    // Reset clock sync on death
    world.client_tick = 0;
    world.server_tick_offset = 0;
    world.clock_offset_ms = 0;
    world.last_server_tick = 0;
    world.last_server_time_ms = 0;
    world.last_acked_server_tick = 0;
    load_current_level(world);
}

fn load_current_level(world: &mut World) {
    world.bullets.clear();
    world.remote_bullets.clear();
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();
    // Reset clock sync on room transition
    world.client_tick = 0;
    world.server_tick_offset = 0;
    world.clock_offset_ms = 0;
    world.last_server_tick = 0;
    world.last_server_time_ms = 0;
    world.last_acked_server_tick = 0;
    // Reset sticky edges to avoid carry-over from previous room
    world.jump_sticky = 0;
    world.shoot_sticky = 0;

    println!(
        "Level {},{} loaded",
        world.current_coords.0, world.current_coords.1
    );
}
pub fn check_level_transitions(world: &mut World) {
    let (cx, cy) = world.current_coords;

    // Bounds-guard: skip when coords are outside the levels matrix.
    let maybe_level = world.levels_matrix.get(cy).and_then(|row| row.get(cx));
    let Some(level) = maybe_level else {
        return;
    };

    let level_w = level.pixel_width();
    let level_h = level.pixel_height();
    let p = &mut world.player;

    let mut changed = false;

    // --- SALIDA DERECHA ---
    if p.pos.x + p.w > level_w - 5.0 {
        // A small 5px margin
        if cx < world.levels_matrix[0].len() - 1 {
            world.current_coords.0 += 1;
            p.pos.x = 10.0;
            changed = true;
        }
    }
    // --- SALIDA IZQUIERDA ---
    else if p.pos.x < -5.0 && cx > 0 {
        world.current_coords.0 -= 1;
        p.pos.x = world.levels_matrix[cy][cx - 1].pixel_width() - p.w - 10.0;
        changed = true;
    }

    // --- SALIDA ABAJO ---
    if p.pos.y + p.h > level_h - 5.0 {
        if cy < world.levels_matrix.len() - 1 {
            world.current_coords.1 += 1;
            p.pos.y = 10.0;
            changed = true;
        }
    }
    // --- SALIDA ARRIBA ---
    else if p.pos.y + p.h < 5.0 && cy > 0 {
        world.current_coords.1 -= 1;
        p.pos.y = world.levels_matrix[cy - 1][cx].pixel_height() - p.h - 10.0;
        changed = true;
    }

    if changed {
        println!(
            "Level changed: {},{}",
            world.current_coords.0, world.current_coords.1
        );
        load_current_level(world);
        // Snap the interpolation source to the post-teleport entry position so
        // `draw(alpha)` renders at the new-room spawn instead of lerping from
        // the old-room position for one frame (the level is already drawn at
        // the new coords).
        world.prev_player_pos = world.player.pos;
        world.prev_player_dir = world.player.dir;
    }
}

// Spec 3 — request/ack primitives. See plan Box 5 (C1–C6).
#[cfg(test)]
mod tests {
    use super::{apply_room_change_local, request_room_change, respawn_local};
    use crate::network::packet_io::PacketIO;
    use crate::network::prediction::InputEntry;
    use crate::world::game::World;
    use game::entities::bullet::Bullet;
    use game::maps::Map;
    use game::room_graph::Direction;
    use glam::Vec2;
    use protocol::GamePacket;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared send log: the fake `PacketIO` pushes into it, the test reads it.
    #[derive(Clone, Default)]
    struct Sends(Rc<RefCell<Vec<GamePacket>>>);

    struct RecordingPacketIO {
        sent: Sends,
    }

    impl PacketIO for RecordingPacketIO {
        fn try_recv(&mut self) -> Option<GamePacket> {
            None
        }

        fn send(&mut self, packet: GamePacket) {
            self.sent.0.borrow_mut().push(packet);
        }
    }

    fn make_world() -> (World, Sends) {
        let sends = Sends::default();
        let world = World::new(
            Map::new(),
            Box::new(RecordingPacketIO {
                sent: sends.clone(),
            }),
        );
        (world, sends)
    }

    /// C1 — a predicted body inside the (0,0) East band sends exactly one
    /// request and records it as pending.
    #[test]
    fn room_change_request_sent_when_predicted_pos_at_door() {
        let (mut world, sends) = make_world();
        world.id = Some(1);
        world.current_coords = (0, 0);
        world.player.pos = Vec2::new(766.0, 320.0);
        world.pending_room_change = None;
        world.room_change_armed = true;

        request_room_change(&mut world);

        assert_eq!(
            sends.0.borrow().as_slice(),
            &[GamePacket::RequestRoomChange {
                id: 1,
                direction: Direction::East
            }]
        );
        assert_eq!(world.pending_room_change, Some(Direction::East));
    }

    /// C2 — away from every door nothing is sent.
    #[test]
    fn room_change_not_sent_away_from_door() {
        let (mut world, sends) = make_world();
        world.id = Some(1);
        world.current_coords = (0, 0);
        world.player.pos = Vec2::new(300.0, 200.0);
        world.pending_room_change = None;
        world.room_change_armed = true;

        request_room_change(&mut world);

        assert!(sends.0.borrow().is_empty());
        assert_eq!(world.pending_room_change, None);
    }

    /// C3 — at a door but disarmed (just came through) sends nothing.
    #[test]
    fn room_change_not_sent_when_disarmed() {
        let (mut world, sends) = make_world();
        world.id = Some(1);
        world.current_coords = (0, 0);
        world.player.pos = Vec2::new(766.0, 320.0);
        world.pending_room_change = None;
        world.room_change_armed = false;

        request_room_change(&mut world);

        assert!(sends.0.borrow().is_empty());
        assert_eq!(world.pending_room_change, None);
    }

    /// C4 — leaving the entry band re-arms; returning to it sends the request.
    #[test]
    fn room_change_rearms_after_leaving_band() {
        let (mut world, sends) = make_world();
        world.id = Some(1);
        world.current_coords = (1, 0);
        world.pending_room_change = None;
        world.room_change_armed = false;

        // Still inside the (1,0) West entry band while disarmed.
        world.player.pos = Vec2::new(10.0, 321.0);
        request_room_change(&mut world);
        assert!(sends.0.borrow().is_empty());
        assert!(!world.room_change_armed);

        // Away from every band: re-armed, still no send.
        world.player.pos = Vec2::new(100.0, 320.0);
        request_room_change(&mut world);
        assert!(sends.0.borrow().is_empty());
        assert!(world.room_change_armed);

        // Back in the West band while armed: one West request.
        world.player.pos = Vec2::new(5.0, 320.0);
        request_room_change(&mut world);
        assert_eq!(
            sends.0.borrow().as_slice(),
            &[GamePacket::RequestRoomChange {
                id: 1,
                direction: Direction::West
            }]
        );
        assert_eq!(world.pending_room_change, Some(Direction::West));
    }

    /// C5 — accepting a room change sets the room/entry, zeroes velocity,
    /// preserves `dir`, snaps the interpolation source and resets request state.
    #[test]
    fn apply_room_change_local_sets_room_entry_and_resets_state() {
        let (mut world, _sends) = make_world();
        world.id = Some(1);
        world.player.pos = Vec2::new(500.0, 300.0);
        world.player.dir = -1.0;
        world.pending_room_change = Some(Direction::East);
        world.room_change_retry = 4;
        world.input_buffer.push(InputEntry {
            seq: 1,
            move_left: true,
            move_right: false,
            jump: false,
        });
        world.bullets.push(Bullet::new(Vec2::new(0.0, 0.0), 1.0, 1));

        apply_room_change_local(&mut world, (1, 0), Vec2::new(10.0, 321.0));

        assert_eq!(world.current_coords, (1, 0));
        assert_eq!(world.player.pos, Vec2::new(10.0, 321.0));
        assert_eq!(world.player.vel, Vec2::ZERO);
        assert_eq!(world.player.dir, -1.0);
        assert_eq!(world.prev_player_pos, world.player.pos);
        assert_eq!(world.prev_player_dir, -1.0);
        assert!(world.input_buffer.is_empty());
        assert!(world.bullets.is_empty());
        assert_eq!(world.pending_room_change, None);
        assert_eq!(world.room_change_retry, 0);
        assert!(!world.room_change_armed);
    }

    /// C6 — a local respawn returns to (0,0)/(100,100) and resets the request
    /// state to a clean, armed spawn.
    #[test]
    fn respawn_local_resets_room_request_state() {
        let (mut world, _sends) = make_world();
        world.id = Some(1);
        world.current_coords = (1, 0);
        world.player.pos = Vec2::new(500.0, 300.0);
        world.pending_room_change = Some(Direction::East);
        world.room_change_retry = 5;
        world.room_change_armed = false;

        respawn_local(&mut world);

        assert_eq!(world.current_coords, (0, 0));
        assert_eq!(world.player.pos, Vec2::new(100.0, 100.0));
        assert_eq!(world.pending_room_change, None);
        assert_eq!(world.room_change_retry, 0);
        assert!(world.room_change_armed);
    }
}
