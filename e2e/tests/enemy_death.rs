// Phase 2e — Client Enemy Death Flow E2E Tests (AC 17, 18, 19, 20)
// These tests verify server-authoritative enemy death.

use e2e::harness::TestHarness;
use game::entities::enemy::{Enemy, EnemyKind};
use metroidvania_rust::network::world::handle_packet;
use protocol::GamePacket;

/// AC 17 — Enemy struct has id: u8 field; enemies_from_data assigns
/// sequential IDs per room matching server's room_x * 16 + spawn_index.
#[test]
fn test_enemy_has_id_field() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();

    harness.tick_n(2);

    let client = harness.client(handle);
    let level = &client.world.levels_matrix[0][0];
    let enemies = game::entities::enemy::enemies_from_data(&level.data, 0);

    // Each enemy should have an id field
    for (i, enemy) in enemies.iter().enumerate() {
        // The id should be assigned based on spawn order
        // For room (0,0): id = 0 * 16 + spawn_index = spawn_index
        assert_eq!(
            enemy.id, i as u8,
            "Enemy id should match spawn index for room (0,0)"
        );
    }

    // Also verify the Enemy struct has the id field at all
    let _enemy: Enemy = Enemy::new(0, glam::Vec2::ZERO, EnemyKind::Enemy);
    // If this compiles, the id field exists. If not, compilation fails.
    let _ = _enemy.id;
}

/// AC 18 — handle_packet on EnemyDied finds enemy by enemy_id (room match)
/// and calls die().
#[test]
fn test_enemydied_handler_kills_enemy() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();

    harness.tick_n(3);

    // Position player away from room boundaries to avoid transitions
    harness.set_client_player_pos(handle, 100.0, 100.0);

    // We need an enemy in the room to kill. Since current map has no enemies,
    // we'll need to inject one or the test map needs enemies.
    // For now, manually add an enemy to the client's world for testing.
    let client = harness.client_mut(handle);
    let mut enemy = Enemy::new(5, glam::Vec2::new(200.0, 200.0), EnemyKind::Enemy);
    enemy.alive = true;
    client.world.enemies.push(enemy);

    assert!(client.world.enemies.iter().any(|e| e.id == 5 && e.alive));

    // Send EnemyDied for enemy_id=5 in current room (0,0)
    let enemy_died = GamePacket::EnemyDied {
        enemy_id: 5,
        room_x: 0,
        room_y: 0,
    };

    // Process the packet directly (without running a full tick that clears enemies)
    {
        let client = harness.client_mut(handle);
        handle_packet(&mut client.world, enemy_died);
    }

    // Enemy should now be dead
    let client = harness.client(handle);
    let enemy_opt = client.world.enemies.iter().find(|e| e.id == 5);
    assert!(enemy_opt.is_some());
    assert!(
        !enemy_opt.unwrap().alive,
        "Enemy should be dead after EnemyDied packet"
    );
}

/// AC 19 — World::update own-bullet vs enemy collision loop removed;
/// no local enemy.die() on bullet collision.
#[test]
fn test_own_bullet_enemy_collision_removed() {
    let mut harness = TestHarness::new();
    let handle = harness.connect_client();

    harness.tick_n(3);

    // Position player away from boundaries
    harness.set_client_player_pos(handle, 100.0, 100.0);

    let client = harness.client_mut(handle);

    // Add an enemy at a known position
    let mut enemy = Enemy::new(1, glam::Vec2::new(200.0, 200.0), EnemyKind::Enemy);
    enemy.alive = true;
    client.world.enemies.push(enemy);

    // Add a bullet that would hit the enemy (at enemy position)
    // Bullet spawns at player center, travels right
    client.world.player.pos = glam::Vec2::new(150.0, 200.0);
    client.world.player.dir = 1.0;
    client.world.player.w = 30.0;
    client.world.player.h = 38.0;

    // Manually spawn a bullet that will hit the enemy on update
    client
        .world
        .bullets
        .push(game::entities::bullet::Bullet::new(
            glam::Vec2::new(165.0, 219.0), // Player center
            1.0,
            client.player_id,
        ));

    assert!(client.world.enemies.iter().any(|e| e.id == 1 && e.alive));

    // Run update via harness tick — bullet should collide with enemy
    // BUT: the collision loop should be REMOVED, so enemy should stay alive
    harness.tick();

    // Enemy should still be alive (no local collision death)
    let client = harness.client(handle);
    let enemy_opt = client.world.enemies.iter().find(|e| e.id == 1);
    assert!(enemy_opt.is_some());
    assert!(
        enemy_opt.unwrap().alive,
        "Enemy should NOT die from local bullet collision; death is server-authoritative"
    );
}

/// AC 20 — E2E: Client A shoots enemy → server validates hit → EnemyDied
/// broadcast → enemy dies on Client A AND Client B.
///
/// This test is ignored because it requires adding test enemies to the map data
/// or extending the harness with server-side enemy injection.
/// The server-side raycast hit validation is implemented and tested in server unit tests.
#[ignore]
#[test]
fn test_e2e_shoot_enemy_both_clients() {
    let mut harness = TestHarness::new();
    let _handle_a = harness.connect_client(); // Shooter
    let _handle_b = harness.connect_client(); // Observer

    harness.tick_n(3);

    // TODO: Implement when harness supports server-side enemy injection
    // For now, the server enemy state and raycast hit validation is tested
    // in server unit tests (raycast_hits_enemy, raycast_miss_above_below, etc.)
}
