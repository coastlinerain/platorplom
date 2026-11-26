use crate::network::packet_io::PacketIO;
use crate::network::prediction::{self, InputEntry, InputRingBuffer};
use crate::world::game::{ConfirmedBullet, World};
use game::entities::bullet::Bullet;
use game::entities::nemesis::Nemesis;
use protocol::GamePacket;

pub fn update_network(world: &mut World) {
    recv_and_process(world);
    send_input(world);
}

/// Receive half: drain all pending packets and run the (currently inert)
/// staleness check. Public so `Paused` can receive/process packets without
/// stepping the simulation or sending input.
pub fn recv_and_process(world: &mut World) {
    // Receive: drain all pending packets from the server.
    while let Some(decoded) = world.network.try_recv() {
        handle_packet(world, decoded);
    }

    // Staleness check: if we haven't received a PlayerPos for our own player
    // in STALENESS_THRESHOLD_TICKS server ticks, force reconciliation.
    if world.id.is_some() {
        // We need the current server tick estimate to compare against last_acked_server_tick
        // server_tick_estimate = client_tick + server_tick_offset
        let server_tick_estimate = world.client_tick as i64 + world.server_tick_offset;
        if server_tick_estimate > 0 {
            let estimated_server_tick = server_tick_estimate as u64;
            if estimated_server_tick.saturating_sub(world.last_acked_server_tick)
                > game::config::STALENESS_THRESHOLD_TICKS
            {
                // Force reconciliation with the last known server position
                // This is a fallback - the actual reconciliation happens in handle_packet
                // when PlayerPos arrives, but we can trigger a re-send of inputs or log a warning
                // For now, we just note the staleness; the next PlayerPos will trigger reconcile
                // TODO: Could implement input re-send or position query here
            }
        }
    }
}

/// Send half: send one `PlayerInput` if `world.id.is_some()`.
pub fn send_input(world: &mut World) {
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
            shoot, // still sent to server for cooldown/raycast
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
            world.spawned_bullet_ids.clear();
            world.confirmed_bullets.clear();
            // Reset clock sync fields on new connection
            world.client_tick = 0;
            world.server_tick_offset = 0;
            world.clock_offset_ms = 0;
            world.last_server_tick = 0;
            world.last_server_time_ms = 0;
            world.last_acked_server_tick = 0;
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
            server_tick,
            server_time_ms,
        } => {
            // Any PlayerPos (own or remote) proves the link is alive — the
            // disconnect watchdog consumes this flag each frame.
            world.received_player_pos = true;

            if Some(id) == world.id {
                // --- PHASE 2f: CLOCK SYNCHRONIZATION ---
                // Update clock offset using server timestamp
                let local_time_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                // Calculate clock offset: server_time - local_time
                world.clock_offset_ms = server_time_ms as i64 - local_time_ms;

                // Update tick offset: server_tick - client_tick
                world.server_tick_offset = server_tick as i64 - world.client_tick as i64;

                world.last_acked_server_tick = server_tick;
                world.last_server_time_ms = server_time_ms;
                // Track last acknowledged server tick for staleness check
                world.last_acked_server_tick = server_tick;
                // --- END CLOCK SYNCHRONIZATION ---

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
                // Set anchor for tick-based interpolation
                interp.set_anchor(server_tick);
            }
        }
        // NEW: Phase 2e - BulletSpawn from server (authoritative bullet spawn)
        GamePacket::BulletSpawn {
            bullet_id,
            x,
            y,
            dir,
            owner_id,
            seq,
        } => {
            // Deduplicate by bullet_id
            if world.spawned_bullet_ids.contains(&bullet_id) {
                return;
            }
            world.spawned_bullet_ids.insert(bullet_id);

            // Store confirmed bullet for reconciliation (keyed by input seq)
            world.confirmed_bullets.insert(
                seq,
                ConfirmedBullet {
                    bullet_id,
                    x,
                    y,
                    dir,
                },
            );

            let spawn_pos = glam::Vec2::new(x, y);
            if Some(owner_id) == world.id {
                // Own bullet: spawn at server-authoritative position
                world.bullets.push(Bullet::new(spawn_pos, dir, owner_id));
            } else {
                // Remote player's bullet: spawn as remote_bullet
                world
                    .remote_bullets
                    .push(Bullet::new(spawn_pos, dir, owner_id));
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
        GamePacket::Hit { entity_id } if Some(entity_id) == world.id => {
            // Reset clock sync on own death
            world.client_tick = 0;
            world.server_tick_offset = 0;
            world.clock_offset_ms = 0;
            world.last_server_tick = 0;
            world.last_server_time_ms = 0;
            world.last_acked_server_tick = 0;
        }
        _ => {}
    }
}

pub fn send_join_req(network: &mut dyn PacketIO) {
    let join_msg = GamePacket::JoinRequest;
    network.send(join_msg);
}
