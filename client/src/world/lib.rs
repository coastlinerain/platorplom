use crate::network::prediction::InputRingBuffer;
use crate::world::game::World;
use game::entities::player::Player;
use game::rect::Rect;
use game::room_graph::RoomCoords;
use protocol::GamePacket;

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
    // The canonical spawn is not inside any trigger band: start armed with no
    // request in flight.
    world.pending_room_change = None;
    world.room_change_retry = 0;
    world.room_change_armed = true;
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
/// Client half of the room-change handshake: detect a door under the
/// **predicted** body and, if armed, send one `RequestRoomChange`.
///
/// It never mutates `current_coords` or teleports; the server's ack (or an own
/// `PlayerPos` that arrives first) applies the authoritative room.
pub fn request_room_change(world: &mut World) {
    let Some(id) = world.id else {
        return;
    };

    // At most one outstanding request: wait for the ack before sending again.
    if world.pending_room_change.is_some() {
        return;
    }

    // Rejection backoff.
    if world.room_change_retry > 0 {
        world.room_change_retry -= 1;
        return;
    }

    let rect = Rect::new(
        world.player.pos.x,
        world.player.pos.y,
        world.player.w,
        world.player.h,
    );

    // Detection uses the raw geometric trigger, never `server_trigger`.
    match world.room_graph.door_at(world.current_room(), rect) {
        // Left every trigger band: safe to detect again.
        None => world.room_change_armed = true,
        // Still inside the entry band of the door just used: no bounce.
        Some(_) if !world.room_change_armed => {}
        Some(door) => {
            world.network.send(GamePacket::RequestRoomChange {
                id,
                direction: door.direction,
            });
            world.pending_room_change = Some(door.direction);
        }
    }
}

/// Applies a server-authoritative room/position (accept ack, defensive reject
/// adoption, or an own `PlayerPos` that named a new room).
///
/// Idempotent: applying the same state twice is harmless. Clears the
/// unacknowledged input buffer so old-room inputs are not replayed into the new
/// room, and `load_current_level` resets bullets/clock/sticky edges.
pub fn apply_room_change_local(world: &mut World, room: RoomCoords, pos: glam::Vec2) {
    world.current_coords = (room.0 as usize, room.1 as usize);
    world.player.pos = pos;
    world.player.vel = glam::Vec2::ZERO;
    // `dir` is preserved: this is a room change, not a respawn.

    // Snap the interpolation source so `draw(alpha)` does not lerp from the old
    // room's position.
    world.prev_player_pos = pos;
    world.prev_player_dir = world.player.dir;

    world.input_buffer = InputRingBuffer::new();
    world.pending_room_change = None;
    world.room_change_retry = 0;
    // Disarmed until the body leaves every band (anti-bounce).
    world.room_change_armed = false;

    load_current_level(world);
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
