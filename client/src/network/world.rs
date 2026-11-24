use crate::network::client::NetworkClient;
use crate::network::interpolation::RemoteInterp;
use crate::network::prediction::{self, InputEntry, InputRingBuffer};
use crate::world::game::World;
use game::entities::bullet::Bullet;
use game::entities::nemesis::Nemesis;
use laminar::SocketEvent;
use macroquad::prelude::*;
use protocol::GamePacket;

pub fn update_network(world: &mut World) {
    // Receive: drain all pending packets from the server.
    while let Ok(event) = world.network.receiver.try_recv() {
        if let SocketEvent::Packet(packet) = event
            && let Ok(decoded) = postcard::from_bytes::<GamePacket>(packet.payload())
        {
            handle_packet(world, decoded);
        }
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

        let move_left = is_key_down(KeyCode::Left);
        let move_right = is_key_down(KeyCode::Right);

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

        let bytes = postcard::to_allocvec(&input_msg).unwrap();
        let _ = world.network.sender.send(laminar::Packet::unreliable(
            world.network.server_addr,
            bytes,
        ));
    }
}

fn handle_packet(world: &mut World, decoded: GamePacket) {
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
                let level = &world.levels_matrix[world.current_coords.1][world.current_coords.0];
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
            } else {
                // --- Remote player: push interpolation snapshot ---
                let nemesis = world.other_players.entry(id).or_insert_with(|| {
                    let mut n = Nemesis::new(id, vec2(x, y));
                    n.last_dir = dir;
                    n.coords = (level_x as usize, level_y as usize);
                    n
                });
                // Update raw pos/dir/coords for non-interpolation fallback.
                nemesis.pos = vec2(x, y);
                nemesis.last_dir = dir;
                nemesis.coords = (level_x as usize, level_y as usize);

                let interp = world
                    .other_interp
                    .entry(id)
                    .or_insert_with(RemoteInterp::new);
                interp.push(nemesis, x, y, dir);
            }
        }
        GamePacket::Action { id, kind, dir } => {
            if kind == "shoot" {
                if let Some(enemy) = world.other_players.get(&id) {
                    println!("Player {} shot from {:?}", id, enemy.pos);

                    let spawn_center =
                        vec2(enemy.pos.x + (enemy.w / 2.0), enemy.pos.y + (enemy.h / 2.0));

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

pub fn send_join_req(network: &NetworkClient) {
    let join_msg = GamePacket::JoinRequest;
    let bytes = postcard::to_allocvec(&join_msg).unwrap();

    let _ = network.sender.send(laminar::Packet::reliable_ordered(
        network.server_addr,
        bytes,
        None,
    ));
}
