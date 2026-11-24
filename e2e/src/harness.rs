use game::config::TICK_HZ;
use game::entities::bullet::Bullet;
use game::entities::player::PlayerInput;
use game::maps::Map;
use metroidvania_rust::network::prediction;
use metroidvania_rust::network::world::handle_packet;
use metroidvania_rust::world::game::World;
use metroidvania_rust::world::lib::check_level_transitions;
use metroidvania_server::ServerWorld;
use protocol::GamePacket;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::transport::LoopbackTransport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientHandle(pub u8);

pub struct VirtualClock {
    pub us: u64,
    pub anchor: Instant,
}

impl VirtualClock {
    pub fn new() -> Self {
        Self {
            us: 0,
            anchor: Instant::now(),
        }
    }

    pub fn now(&self) -> Instant {
        self.anchor + Duration::from_micros(self.us)
    }

    pub fn advance(&mut self, ms: u64) {
        self.us += ms * 1000;
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Headless client wrapper for a single player.
pub struct TestClient {
    pub handle: ClientHandle,
    pub world: World,
    pub player_id: u8,
    pub hold_right_frames: u32,
    pub hold_left_frames: u32,
    pub jump_queued: bool,
    pub shoot_queued: bool,
}

pub struct TestHarness {
    pub server: ServerWorld,
    pub clients: Vec<TestClient>,
    pub clock: VirtualClock,
    pub from_clients: Vec<mpsc::Receiver<Vec<u8>>>,
    pub to_clients: Vec<mpsc::Sender<Vec<u8>>>,
    pub next_handle: u8,
    pub next_player_id: u8,
    pub last_shots: Vec<(u8, f32)>,
    pub last_broadcasts: Vec<(ClientHandle, u8, f32, f32)>,
    pub last_actions: Vec<(ClientHandle, String, f32)>,
    /// Cumulative shot count per player (incremented each tick from
    /// server.world.update's return value).  Used by server_bullet_count
    /// because the server never stores Bullet objects (D6).
    pub total_shots: std::collections::HashMap<u8, usize>,
}

impl TestHarness {
    pub fn new() -> Self {
        Self {
            server: ServerWorld::new(),
            clients: Vec::new(),
            clock: VirtualClock::new(),
            from_clients: Vec::new(),
            to_clients: Vec::new(),
            next_handle: 0,
            next_player_id: 1,
            last_shots: Vec::new(),
            last_broadcasts: Vec::new(),
            last_actions: Vec::new(),
            total_shots: std::collections::HashMap::new(),
        }
    }

    fn player_id_for(&self, handle: ClientHandle) -> u8 {
        self.clients
            .iter()
            .find(|c| c.handle == handle)
            .map(|c| c.player_id)
            .unwrap_or(0)
    }

    pub fn connect_client(&mut self) -> ClientHandle {
        let handle = ClientHandle(self.next_handle);
        self.next_handle += 1;
        let player_id = self.next_player_id;
        self.next_player_id += 1;

        let (tx_to_server, rx_server_side) = mpsc::channel();
        let (tx_to_client, rx_client_side) = mpsc::channel();

        let loopback = LoopbackTransport {
            rx: rx_client_side,
            tx: tx_to_server,
        };

        let world_data = Map::new();
        let mut world = World::new(world_data, Box::new(loopback));

        self.server.spawn(player_id);

        let response = GamePacket::JoinResponse {
            assigned_id: player_id,
        };
        let bytes = postcard::to_allocvec(&response).expect("serialize JoinResponse");
        let _ = tx_to_client.send(bytes);

        handle_packet(&mut world, response);

        let client = TestClient {
            handle,
            world,
            player_id,
            hold_right_frames: 0,
            hold_left_frames: 0,
            jump_queued: false,
            shoot_queued: false,
        };

        self.clients.push(client);
        self.from_clients.push(rx_server_side);
        self.to_clients.push(tx_to_client);

        handle
    }

    pub fn tick(&mut self) {
        let tick_dt = 1.0 / TICK_HZ as f32;

        // Step 1: Apply queued inputs.
        for client in &mut self.clients {
            client.world.move_left = client.hold_left_frames > 0;
            client.world.move_right = client.hold_right_frames > 0;
            client.world.pressed_jump = client.jump_queued;
            client.world.pressed_shoot = client.shoot_queued;

            if client.hold_right_frames > 0 {
                client.hold_right_frames -= 1;
            }
            if client.hold_left_frames > 0 {
                client.hold_left_frames -= 1;
            }
            client.jump_queued = false;
            client.shoot_queued = false;
        }

        // Step 2: Build and send PlayerInput from each client via PacketIO.
        for client in &mut self.clients {
            let (jump, js) =
                prediction::advance_sticky(client.world.jump_sticky, client.world.pressed_jump);
            let (shoot, ss) =
                prediction::advance_sticky(client.world.shoot_sticky, client.world.pressed_shoot);
            client.world.jump_sticky = js;
            client.world.shoot_sticky = ss;

            let seq = client.world.input_seq.wrapping_add(1);
            client.world.input_seq = seq;

            let input = GamePacket::PlayerInput {
                id: client.player_id,
                level_x: client.world.current_coords.0 as u8,
                level_y: client.world.current_coords.1 as u8,
                move_left: client.world.move_left,
                move_right: client.world.move_right,
                jump,
                shoot,
                seq,
            };

            client.world.input_buffer.push(prediction::InputEntry {
                seq,
                move_left: client.world.move_left,
                move_right: client.world.move_right,
                jump,
            });

            client.world.network.send(input);
        }

        // Step 3: Drain client→server channels, feed server.apply_input.
        for rx in &self.from_clients {
            while let Ok(bytes) = rx.try_recv() {
                if let Ok(GamePacket::PlayerInput {
                    id,
                    level_x,
                    level_y,
                    move_left,
                    move_right,
                    jump,
                    shoot,
                    seq,
                }) = postcard::from_bytes::<GamePacket>(&bytes)
                {
                    self.server.apply_input(
                        id,
                        level_x,
                        level_y,
                        PlayerInput {
                            move_left,
                            move_right,
                            jump,
                            shoot,
                        },
                        seq,
                    );
                }
            }
        }

        // Step 4: Server update.
        let tick_shots = self.server.update(tick_dt);
        // Accumulate shot counts (server never stores Bullet objects — D6).
        for &(pid, dir) in &tick_shots {
            *self.total_shots.entry(pid).or_insert(0) += 1;
            self.last_shots.push((pid, dir));
        }

        // Step 5: Broadcast PlayerPos to ALL clients.
        self.last_broadcasts.clear();
        let snapshots: Vec<(u8, f32, f32, f32, u8, u8, u16)> = {
            self.server
                .players()
                .iter()
                .map(|(&pid, p)| {
                    let (lx, ly) = self.server.coords().get(&pid).copied().unwrap_or((0, 0));
                    let last_seq = self.server.max_seq().get(&pid).copied().unwrap_or(0);
                    (pid, p.pos.x, p.pos.y, p.dir, lx, ly, last_seq)
                })
                .collect()
        };

        for (c_idx, tx) in self.to_clients.iter().enumerate() {
            let handle = self.clients[c_idx].handle;
            for &(pid, px, py, pdir, lx, ly, last_seq) in &snapshots {
                let packet = GamePacket::PlayerPos {
                    id: pid,
                    x: px,
                    y: py,
                    dir: pdir,
                    level_x: lx,
                    level_y: ly,
                    last_seq,
                };
                let bytes = postcard::to_allocvec(&packet).expect("serialize PlayerPos");
                let _ = tx.send(bytes);
                self.last_broadcasts.push((handle, pid, px, py));
            }
        }

        // Step 6: Broadcast Action for shots to OTHER clients.
        self.last_actions.clear();
        for &(shooter_id, dir) in &self.last_shots {
            for client in &self.clients {
                if client.player_id == shooter_id {
                    continue;
                }
                if let Some(idx) = self.clients.iter().position(|c| c.handle == client.handle) {
                    let tx = &self.to_clients[idx];
                    let packet = GamePacket::Action {
                        id: shooter_id,
                        kind: "shoot".to_string(),
                        dir,
                    };
                    let bytes = postcard::to_allocvec(&packet).expect("serialize Action");
                    let _ = tx.send(bytes);
                    self.last_actions
                        .push((client.handle, "shoot".to_string(), dir));
                }
            }
        }

        // Step 7: Each client drains server→client channel, runs local
        // prediction, and cleans up bullets/enemies.
        // Order: reconcile → predict → transitions (matches plan spec).
        for client in &mut self.clients {
            // --- 7a: receive server packets (reconciliation) ---
            while let Some(packet) = client.world.network.try_recv() {
                handle_packet(&mut client.world, packet);
            }

            // --- 7b: local prediction ---
            let coords = client.world.current_coords;
            {
                let maybe_level = client
                    .world
                    .levels_matrix
                    .get(coords.1)
                    .and_then(|row| row.get(coords.0));
                if let Some(level) = maybe_level {
                    let local_input = PlayerInput {
                        move_left: client.world.move_left,
                        move_right: client.world.move_right,
                        jump: client.world.pressed_jump,
                        shoot: false,
                    };

                    client.world.player.update(tick_dt, level, &local_input);

                    // Spawn own bullet when pressed_shoot is true (mirrors World::update).
                    if client.world.pressed_shoot {
                        client.world.bullets.push(Bullet::new(
                            client.world.player.pos
                                + glam::Vec2::new(
                                    client.world.player.w / 2.0,
                                    client.world.player.h / 2.0,
                                ),
                            client.world.player.dir,
                            client.world.player.id,
                        ));
                    }

                    for bullet in &mut client.world.bullets {
                        bullet.update(tick_dt, level);
                    }
                    for bullet in &mut client.world.enemy_bullets {
                        bullet.update(tick_dt, level);
                    }
                    for enemy in &mut client.world.enemies {
                        enemy.update(tick_dt, level, 0);
                    }
                }
                // else: bogus coords — skip prediction.
            }

            // --- 7c: room transitions ---
            check_level_transitions(&mut client.world);

            // --- 7d: cleanup ---
            client.world.bullets.retain(|b| b.active);
            client.world.enemy_bullets.retain(|b| b.active);
            client.world.enemies.retain(|e| e.alive);
        }

        // Step 8: Advance clock.
        self.clock.advance(17);
    }

    pub fn tick_n(&mut self, n: u32) {
        for _ in 0..n {
            self.tick();
        }
    }

    pub fn hold_right(&mut self, handle: ClientHandle, frames: u32) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.hold_right_frames = frames;
        }
    }

    pub fn hold_left(&mut self, handle: ClientHandle, frames: u32) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.hold_left_frames = frames;
        }
    }

    pub fn press_jump(&mut self, handle: ClientHandle) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.jump_queued = true;
        }
    }

    pub fn press_shoot(&mut self, handle: ClientHandle) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.shoot_queued = true;
        }
    }

    pub fn release_all(&mut self, handle: ClientHandle) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.hold_left_frames = 0;
            client.hold_right_frames = 0;
            client.jump_queued = false;
            client.shoot_queued = false;
        }
    }

    pub fn set_server_player_pos(&mut self, handle: ClientHandle, x: f32, y: f32) {
        let player_id = self.player_id_for(handle);
        if let Some(player) = self.server.players_mut().get_mut(&player_id) {
            player.pos = glam::Vec2::new(x, y);
        }
    }

    pub fn set_server_player_grounded(&mut self, handle: ClientHandle, grounded: bool) {
        let player_id = self.player_id_for(handle);
        if let Some(player) = self.server.players_mut().get_mut(&player_id) {
            player.grounded = grounded;
        }
    }

    pub fn set_client_player_pos(&mut self, handle: ClientHandle, x: f32, y: f32) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.handle == handle) {
            client.world.player.pos = glam::Vec2::new(x, y);
        }
    }

    pub fn server_player_pos(&self, handle: ClientHandle) -> (f32, f32) {
        let player_id = self.player_id_for(handle);
        if let Some(player) = self.server.players().get(&player_id) {
            (player.pos.x, player.pos.y)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn server_player_coords(&self, handle: ClientHandle) -> (u8, u8) {
        let player_id = self.player_id_for(handle);
        self.server
            .coords()
            .get(&player_id)
            .copied()
            .unwrap_or((0, 0))
    }

    pub fn client_player_pos(&self, handle: ClientHandle) -> (f32, f32) {
        if let Some(client) = self.clients.iter().find(|c| c.handle == handle) {
            (client.world.player.pos.x, client.world.player.pos.y)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn client_player_coords(&self, handle: ClientHandle) -> (u8, u8) {
        if let Some(client) = self.clients.iter().find(|c| c.handle == handle) {
            (
                client.world.current_coords.0 as u8,
                client.world.current_coords.1 as u8,
            )
        } else {
            (0, 0)
        }
    }

    pub fn server_bullet_count(&self, handle: ClientHandle) -> usize {
        let player_id = self.player_id_for(handle);
        self.total_shots.get(&player_id).copied().unwrap_or(0)
    }

    pub fn client_bullet_count(&self, handle: ClientHandle) -> usize {
        if let Some(client) = self.clients.iter().find(|c| c.handle == handle) {
            client.world.bullets.len()
        } else {
            0
        }
    }

    /// Return a reference to the client with the given handle.
    pub fn client(&self, handle: ClientHandle) -> &TestClient {
        self.clients
            .iter()
            .find(|c| c.handle == handle)
            .expect("client not found")
    }

    /// Return a mutable reference to the client with the given handle.
    pub fn client_mut(&mut self, handle: ClientHandle) -> &mut TestClient {
        self.clients
            .iter_mut()
            .find(|c| c.handle == handle)
            .expect("client not found")
    }

    /// Disconnect a client: remove from server, broadcast Leave to remaining
    /// clients, and drop channels. Panics if handle is not found.
    pub fn disconnect_client(&mut self, handle: ClientHandle) {
        let player_id = self.player_id_for(handle);

        // Find the client index.
        let idx = self
            .clients
            .iter()
            .position(|c| c.handle == handle)
            .expect("client to disconnect not found");

        // Remove from server world.
        self.server.remove(player_id);

        // Broadcast Leave to all remaining clients.
        for (i, _client) in self.clients.iter().enumerate() {
            if i == idx {
                continue;
            }
            let packet = GamePacket::Leave { id: player_id };
            if let Ok(bytes) = postcard::to_allocvec(&packet) {
                let _ = self.to_clients[i].send(bytes);
            }
        }

        // Remove the client and its channels.
        self.clients.remove(idx);
        self.from_clients.remove(idx);
        self.to_clients.remove(idx);
    }

    pub fn advance_clock(&mut self, ms: u64) {
        self.clock.advance(ms);
    }

    pub fn clock_now(&self) -> Instant {
        self.clock.now()
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
