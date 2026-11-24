use crossbeam_channel::Sender;
use laminar::{Packet, Socket, SocketEvent};
use protocol::GamePacket;
use std::{collections::HashMap, net::SocketAddr, time::Instant};

mod world;
use world::ServerWorld;

fn main() {
    let mut socket = Socket::bind("127.0.0.1:12345").unwrap();
    let packet_sender = socket.get_packet_sender();
    let event_receiver = socket.get_event_receiver();

    let _thread = std::thread::spawn(move || socket.start_polling());

    let mut next_id: u8 = 1;
    let mut addrs: HashMap<SocketAddr, u8> = HashMap::new();

    // Build the authoritative simulation world from the shared map data.
    let mut world = ServerWorld::new();

    let tick_interval = std::time::Duration::from_secs_f64(1.0 / game::config::TICK_HZ as f64);
    let tick_dt = 1.0 / game::config::TICK_HZ as f32;
    let mut next_tick = Instant::now() + tick_interval;

    println!(
        "Rustvania server running on port 12345 ({} Hz)",
        game::config::TICK_HZ
    );

    loop {
        // Drain all pending events (non-blocking).
        while let Ok(event) = event_receiver.try_recv() {
            match event {
                SocketEvent::Packet(packet) => {
                    let addr = packet.addr();

                    if let Ok(decoded) = postcard::from_bytes::<GamePacket>(packet.payload()) {
                        match decoded {
                            GamePacket::JoinRequest => {
                                let id = next_id;
                                addrs.insert(addr, id);
                                next_id += 1;

                                world.spawn(id);

                                let response = GamePacket::JoinResponse { assigned_id: id };
                                let bytes = postcard::to_allocvec(&response).unwrap();
                                let _ =
                                    packet_sender.send(Packet::reliable_ordered(addr, bytes, None));
                                println!("Player {} joined from {}", id, addr);
                            }
                            GamePacket::PlayerInput {
                                id,
                                level_x,
                                level_y,
                                move_left,
                                move_right,
                                jump,
                                shoot,
                            } => {
                                // Resolve the id from the transport map (do not trust
                                // the packet id for state lookup).
                                let resolved = addrs.get(&addr).copied();
                                if resolved != Some(id) {
                                    // Mismatched id — ignore.
                                    continue;
                                }

                                world.apply_input(
                                    id,
                                    level_x,
                                    level_y,
                                    game::entities::player::PlayerInput {
                                        move_left,
                                        move_right,
                                        jump,
                                        shoot,
                                    },
                                );
                            }
                            GamePacket::Leave { id } => {
                                println!("Player {} left voluntarily", id);
                                addrs.remove(&addr);
                                world.remove(id);
                                broadcast_leave(&addrs, &packet_sender, id);
                            }
                            GamePacket::Hit { entity_id: id } => {
                                println!("Player {} died", id);

                                for (&player_addr, &player_id) in addrs.iter() {
                                    if player_id != id {
                                        let bytes = postcard::to_allocvec(&GamePacket::Hit {
                                            entity_id: id,
                                        })
                                        .unwrap();
                                        let _ = packet_sender.send(Packet::reliable_ordered(
                                            player_addr,
                                            bytes,
                                            None,
                                        ));
                                    }
                                }
                            }
                            // Client-sent PlayerPos and Action are no longer
                            // accepted — the server is authoritative.
                            GamePacket::PlayerPos { .. } | GamePacket::Action { .. } => {}
                            _ => {}
                        }
                    }
                }
                SocketEvent::Timeout(addr) => {
                    if let Some(id) = addrs.remove(&addr) {
                        println!("Player {} disconnected", id);
                        world.remove(id);
                        broadcast_leave(&addrs, &packet_sender, id);
                    }
                }
                _ => {}
            }
        }

        // Run one authoritative tick when it's time (fixed step, not wall-clock).
        // Catch up if the event drain stole too much time (rare, but safe).
        let now = Instant::now();
        while now >= next_tick {
            let shots = world.update(tick_dt);

            // Broadcast authoritative PlayerPos to ALL connected players,
            // INCLUDING the sender (each player needs its own echo so the
            // client's own-player branch applies the authoritative position).
            for (&addr, &id) in &addrs {
                if let Some(player) = world.players().get(&id) {
                    let (lx, ly) = world.coords().get(&id).copied().unwrap_or((0, 0));
                    let bytes = postcard::to_allocvec(&GamePacket::PlayerPos {
                        id,
                        x: player.pos.x,
                        y: player.pos.y,
                        dir: player.dir,
                        level_x: lx,
                        level_y: ly,
                    })
                    .unwrap();
                    let _ = packet_sender.send(Packet::unreliable(addr, bytes));
                }
            }

            // Rebroadcast shoot notifications to the OTHER players only
            // (the shooter's own client spawns its bullet locally).
            for &(shooter_id, dir) in &shots {
                for (&addr, &player_id) in &addrs {
                    if player_id == shooter_id {
                        continue; // skip the shooter
                    }
                    let bytes = postcard::to_allocvec(&GamePacket::Action {
                        id: shooter_id,
                        kind: "shoot".to_string(),
                        dir,
                    })
                    .unwrap();
                    let _ = packet_sender.send(Packet::reliable_ordered(addr, bytes, None));
                }
            }

            next_tick += tick_interval;
        }

        // Idle a few ms so we don't busy-spin.
        let remaining = next_tick
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(5)));
    }
}

fn broadcast_leave(addrs: &HashMap<SocketAddr, u8>, sender: &Sender<Packet>, dropped_id: u8) {
    let leave_msg = GamePacket::Leave { id: dropped_id };
    let bytes = postcard::to_allocvec(&leave_msg).expect("Error serializing Leave");

    for &target_addr in addrs.keys() {
        let packet = Packet::reliable_ordered(target_addr, bytes.clone(), None);
        if let Err(e) = sender.send(packet) {
            eprintln!("Error broadcasting leave to {}: {}", target_addr, e);
        }
    }
}
