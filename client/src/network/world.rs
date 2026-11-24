use crate::network::client::NetworkClient;
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
        // Build the PlayerInput from macroquad keys (same mapping as before).
        // Held: Left/Right; Edge: Space (jump), Z (shoot).
        let input_msg = GamePacket::PlayerInput {
            id,
            level_x: world.current_coords.0 as u8,
            level_y: world.current_coords.1 as u8,
            move_left: is_key_down(KeyCode::Left),
            move_right: is_key_down(KeyCode::Right),
            jump: is_key_pressed(KeyCode::Space),
            shoot: is_key_pressed(KeyCode::Z),
        };
        let bytes = postcard::to_allocvec(&input_msg).unwrap();
        let _ = world.network.sender.send(laminar::Packet::reliable_ordered(
            world.network.server_addr,
            bytes,
            None,
        ));
    }
}

fn handle_packet(world: &mut World, decoded: GamePacket) {
    match decoded {
        GamePacket::JoinResponse { assigned_id } => {
            world.id = Some(assigned_id);
            world.player.id = assigned_id;
            println!("Connected! My ID is {}", assigned_id);
        }
        GamePacket::PlayerPos {
            id,
            x,
            y,
            dir,
            level_x,
            level_y,
        } => {
            if Some(id) == world.id {
                // Own player — apply authoritative position (dumb-terminal model).
                world.player.pos = vec2(x, y);
                world.player.dir = dir;
            } else {
                // Remote player — mirror as Nemesis.
                if let Some(nemesis) = world.other_players.get_mut(&id) {
                    nemesis.pos = vec2(x, y);
                    nemesis.last_dir = dir;
                    nemesis.coords = (level_x as usize, level_y as usize);
                } else {
                    println!("New player {}", id);
                    let mut new_nemesis = Nemesis::new(id, vec2(x, y));
                    new_nemesis.last_dir = dir;
                    new_nemesis.coords = (level_x as usize, level_y as usize);
                    world.other_players.insert(id, new_nemesis);
                }
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
