use crate::network::packet_io::PacketIO;
use crate::network::prediction::{self, InputEntry, InputRingBuffer};
use crate::world::game::World;
use game::entities::bullet::Bullet;
use game::entities::nemesis::Nemesis;
use protocol::GamePacket;

pub fn update_network(world: &mut World) {
    // Receive: drain all pending packets from the server.
    while let Some(decoded) = world.network.try_recv() {
        handle_packet(world, decoded);
    }

    // Send: forward inputs to the server each frame (once assigned an id).
    if let Some(id) = world.id {
        // --- Sticky edge handling (D6) ---
        // Read edge keys from World (sampled once in World::update — is_key_pressed is
        // stateful and consumed on first call).
        let (jump, jump_sticky) = prediction::advance_sticky(world.jump_sticky, world.pressed_jump);
        let (shoot, shoot_sticky) =
            prediction::advance_sticky(world.shoot_sticky, world.pressed_shoot);
        world.jump_sticky = jump_sticky;
        world.shoot_sticky = shoot_sticky;

        // --- Sequence number ---
        world.input_seq = world.input_seq.wrapping_add(1);
        let seq = world.input_seq;

        let move_left = world.move_left;
        let move_right = world.move_right;

        let input_msg = GamePacket::PlayerInput {
            id,
            level_x: world.current_coords.0 as u8,
            level_y: world.current_coords.1 as u8,
            move_left,
            move_right,
            jump,
            shoot,
            seq,
        };

        // Store for reconciliation replay (shoot is NOT stored — D7).
        world.input_buffer.push(InputEntry {
            seq,
            move_left,
            move_right,
            jump,
        });

        world.network.send(input_msg);
    }
}

pub fn handle_packet(world: &mut World, decoded: GamePacket) {
    match decoded {
        GamePacket::JoinResponse { assigned_id } => {
            world.id = Some(assigned_id);
            world.player.id = assigned_id;
            // Reset ack state for the new session.
            world.input_seq = u16::MAX;
            world.last_acked_seq = u16::MAX;
            world.input_buffer = InputRingBuffer::new();
            world.jump_sticky = 0;
            world.shoot_sticky = 0;
            println!("Connected! My ID is {}", assigned_id);
        }
        GamePacket::PlayerPos {
            id,
            x,
            y,
            dir,
            level_x,
            level_y,
            last_seq,
        } => {
            if Some(id) == world.id {
                // --- RECONCILIATION for own player ---
                // Guard: only reconcile when the server's reported room matches
                // the client's current room AND the server position is not inside
                // a solid tile. Without this, crossing a room boundary rewinds the
                // player into a wall (the server never teleports on room transition
                // — D9 scope boundary — so its position lags behind the client's).
                let coords_match = level_x as usize == world.current_coords.0
                    && level_y as usize == world.current_coords.1;
                if coords_match {
                    // Bounds-guard: a bogus (99,99) coords echoed from the server
                    // must not index-panic levels_matrix.  Only reconcile when
                    // the client's current room is valid for its own matrix.
                    let maybe_level = world
                        .levels_matrix
                        .get(world.current_coords.1)
                        .and_then(|row| row.get(world.current_coords.0));
                    let Some(level) = maybe_level else {
                        // Coords out of range — skip reconciliation this frame.
                        return;
                    };
                    // Use solid_in_rect (handles OOB gracefully, as non-solid)
                    // instead of colision (treats OOB as solid) so that
                    // reconciliation works at room edges where the player rect
                    // extends beyond the level boundary.
                    let not_in_wall = !level.solid_in_rect(game::rect::Rect::new(
                        x,
                        y,
                        world.player.w,
                        world.player.h,
                    ));
                    if not_in_wall {
                        prediction::reconcile(
                            &mut world.player,
                            &mut world.input_buffer,
                            &mut world.last_acked_seq,
                            x,
                            y,
                            dir,
                            last_seq,
                            level,
                        );
                    }
                }
                // else: server is reporting position from a different room (or
                // inside a wall) — skip reconciliation; client prediction is
                // more correct until the server position catches up.
            } else {
                // --- Remote player: push interpolation snapshot ---
                let nemesis = world.other_players.entry(id).or_insert_with(|| {
                    let mut n = Nemesis::new(id, glam::Vec2::new(x, y));
                    n.last_dir = dir;
                    n.coords = (level_x as usize, level_y as usize);
                    n
                });
                // Only update raw pos when the server-reported position is
                // within the bounds of the VIEWER's current room. After a room
                // transition, the server's position may still be at the old
                // room boundary (e.g., x=790 in a 400 px-wide room) which
                // renders the nemesis off-screen.
                let same_room = level_x as usize == world.current_coords.0
                    && level_y as usize == world.current_coords.1;
                let pos_in_viewer_bounds = if same_room {
                    if let Some(viewer_level) = world
                        .levels_matrix
                        .get(world.current_coords.1)
                        .and_then(|row| row.get(world.current_coords.0))
                    {
                        x >= 0.0
                            && y >= 0.0
                            && x < viewer_level.pixel_width()
                            && y < viewer_level.pixel_height()
                    } else {
                        false
                    }
                } else {
                    true // different room — position doesn't affect viewer
                };
                if pos_in_viewer_bounds {
                    nemesis.pos = glam::Vec2::new(x, y);
                    nemesis.last_dir = dir;
                }
                nemesis.coords = (level_x as usize, level_y as usize);

                let interp = world.other_interp.entry(id).or_default();
                interp.push(nemesis, x, y, dir);
            }
        }
        GamePacket::Action { id, kind, dir } => {
            if kind == "shoot" {
                if let Some(enemy) = world.other_players.get(&id) {
                    println!("Player {} shot from {:?}", id, enemy.pos);

                    let spawn_center = glam::Vec2::new(
                        enemy.pos.x + (enemy.w / 2.0),
                        enemy.pos.y + (enemy.h / 2.0),
                    );

                    if enemy.coords == world.current_coords {
                        println!("SAME room");

                        world
                            .enemy_bullets
                            .push(Bullet::new(spawn_center, dir, enemy.id));
                    }
                } else {
                    println!("DEBUG: Received shot from ID {} but not in my list!", id);
                }
            }
        }
        GamePacket::Leave { id } => {
            if let Some(_removed) = world.other_players.remove(&id) {
                world.other_interp.remove(&id);
                println!("Player {} left. Removed from world.", id);
            } else {
                println!(
                    "DEBUG: Tried to remove ID {} that didn't exist on this client.",
                    id
                );
            }
        }
        _ => {}
    }
}

pub fn send_join_req(network: &mut dyn PacketIO) {
    let join_msg = GamePacket::JoinRequest;
    network.send(join_msg);
}
