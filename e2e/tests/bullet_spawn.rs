// Phase 2e — Client Bullet Spawn Flow E2E Tests (AC 13, 14, 15, 16)
// These tests verify the client waits for BulletSpawn before spawning bullets.

use e2e::harness::TestHarness;
use game::entities::bullet::Bullet;
use metroidvania_rust::network::world::handle_packet;
use protocol::GamePacket;

/// Helper to get bullet direction from velocity
fn bullet_dir(bullet: &Bullet) -> f32 {
    if bullet.vel.x > 0.0 { 1.0 } else { -1.0 }
}

/// AC 13 — World::update does NOT spawn bullet when pressed_shoot=true
/// (the local spawn block is removed). Bullet comes from server's BulletSpawn.
#[test]
fn test_no_local_bullet_on_pressed_shoot() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();

    // Position player for a clean shot
    harness.set_server_player_pos(handle, 100.0, 100.0);
    harness.set_server_player_grounded(handle, true);
    harness.set_client_player_pos(handle, 100.0, 100.0);

    // Queue a shoot input
    harness.press_shoot(handle);
    harness.tick();

    // After one tick, the client SHOULD have a bullet from the server's BulletSpawn
    // (not from local spawn). The local spawn on pressed_shoot is removed.
    let client = harness.client(handle);
    assert_eq!(
        client.world.bullets.len(),
        1,
        "Client should have bullet from server's BulletSpawn after one tick"
    );
    let bullet = &client.world.bullets[0];
    // Verify it's a server-authoritative bullet (not a locally-spawned one at a different position)
    // The exact position depends on server timing, but it should be near player center
    // and have the correct direction.
    assert_eq!(bullet_dir(bullet), 1.0);
    assert_eq!(bullet.owner_id, client.player_id);
}

/// AC 14 — handle_packet on BulletSpawn with owner_id == world.id spawns
/// Bullet in world.bullets at server (x,y) with server dir.
#[test]
fn test_bulletspawn_handler_spawns_own_bullet() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();
    let client_id = harness.client(handle).player_id;

    // Advance a few ticks to establish connection
    harness.tick_n(3);

    // Manually inject a BulletSpawn packet for our player
    let bullet_spawn = GamePacket::BulletSpawn {
        bullet_id: 42,
        x: 150.0,
        y: 200.0,
        dir: 1.0,
        owner_id: client_id,
        seq: 10,
    };

    // Process the packet directly (without running local prediction)
    {
        let client = harness.client_mut(handle);
        handle_packet(&mut client.world, bullet_spawn);
    }

    // Verify bullet was spawned at server-authoritative position
    let client = harness.client(handle);
    assert_eq!(client.world.bullets.len(), 1);
    let bullet = &client.world.bullets[0];
    assert_eq!(bullet.pos.x, 150.0);
    assert_eq!(bullet.pos.y, 200.0);
    assert_eq!(bullet_dir(bullet), 1.0);
    assert_eq!(bullet.owner_id, client_id);
}

/// AC 14 — handle_packet on BulletSpawn with owner_id != world.id spawns
/// Bullet in world.enemy_bullets.
#[test]
fn test_bulletspawn_handler_spawns_remote_bullet() {
    let mut harness = TestHarness::new();
    let handle_a = harness.connect_client(); // Player 1
    let handle_b = harness.connect_client(); // Player 2

    harness.tick_n(3);

    let client_a_id = harness.client(handle_a).player_id;
    let _client_b_id = harness.client(handle_b).player_id;

    // Inject BulletSpawn from Player A's perspective (owner_id = A)
    // but process it on Client B
    let bullet_spawn = GamePacket::BulletSpawn {
        bullet_id: 99,
        x: 300.0,
        y: 400.0,
        dir: -1.0,
        owner_id: client_a_id,
        seq: 20,
    };

    // Process the packet directly (without running local prediction)
    {
        let client = harness.client_mut(handle_b);
        handle_packet(&mut client.world, bullet_spawn);
    }

    // Client B should have the bullet in enemy_bullets (not bullets)
    let client_b = harness.client(handle_b);
    assert_eq!(client_b.world.bullets.len(), 0);
    assert_eq!(client_b.world.enemy_bullets.len(), 1);
    let bullet = &client_b.world.enemy_bullets[0];
    assert_eq!(bullet.pos.x, 300.0);
    assert_eq!(bullet.pos.y, 400.0);
    assert_eq!(bullet_dir(bullet), -1.0);
    assert_eq!(bullet.owner_id, client_a_id);
}

/// AC 15 — Duplicate BulletSpawn with same bullet_id ignored
/// (client tracks spawned_bullet_ids HashSet).
#[test]
fn test_bulletspawn_dedupe() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();
    let client_id = harness.client(handle).player_id;

    harness.tick_n(3);

    // Send first time
    let bullet_spawn1 = GamePacket::BulletSpawn {
        bullet_id: 777,
        x: 100.0,
        y: 100.0,
        dir: 1.0,
        owner_id: client_id,
        seq: 5,
    };
    harness.inject_server_packet(handle, bullet_spawn1);
    harness.tick();

    let client = harness.client(handle);
    assert_eq!(client.world.bullets.len(), 1);

    // Send duplicate with same bullet_id
    let bullet_spawn2 = GamePacket::BulletSpawn {
        bullet_id: 777,
        x: 100.0,
        y: 100.0,
        dir: 1.0,
        owner_id: client_id,
        seq: 5,
    };
    harness.inject_server_packet(handle, bullet_spawn2);
    harness.tick();

    // Should still have only 1 bullet (duplicate ignored)
    let client = harness.client(handle);
    assert_eq!(
        client.world.bullets.len(),
        1,
        "Duplicate BulletSpawn with same bullet_id should be ignored"
    );
}

/// AC 16 — E2E: Client A presses Z → server broadcasts BulletSpawn →
/// Client A and Client B both have bullet at same server-authoritative position.
#[test]
fn test_e2e_bulletspawn_both_clients() {
    let mut harness = TestHarness::new();
    let handle_a = harness.connect_client(); // Shooter
    let handle_b = harness.connect_client(); // Observer

    let player_a_id = harness.client(handle_a).player_id;

    // Position player A at known location
    harness.set_server_player_pos(handle_a, 100.0, 100.0);
    harness.set_server_player_grounded(handle_a, true);
    harness.set_client_player_pos(handle_a, 100.0, 100.0);

    harness.tick_n(3);

    // Player A shoots
    harness.press_shoot(handle_a);
    harness.tick();

    // Server processes the shot and broadcasts BulletSpawn to all clients
    // Both clients should receive it and spawn bullets

    let client_a = harness.client(handle_a);
    let client_b = harness.client(handle_b);

    // Both should have exactly 1 bullet at the same position
    // (server-authoritative spawn position)
    assert_eq!(client_a.world.bullets.len(), 1);
    assert_eq!(client_b.world.enemy_bullets.len(), 1);

    let bullet_a = &client_a.world.bullets[0];
    let bullet_b = &client_b.world.enemy_bullets[0];

    // Positions should match (server-authoritative)
    assert_eq!(bullet_a.pos.x, bullet_b.pos.x);
    assert_eq!(bullet_a.pos.y, bullet_b.pos.y);
    assert_eq!(bullet_dir(bullet_a), bullet_dir(bullet_b));
    assert_eq!(bullet_a.owner_id, player_a_id);
    assert_eq!(bullet_b.owner_id, player_a_id);
}
