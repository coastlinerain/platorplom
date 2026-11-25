use crate::world::game::World;
use protocol::GamePacket;

pub fn update_enemy_bullets_position(world: &mut World, dt: f32) {
    let current_room = world.current_coords;
    let level = &world.levels_matrix[current_room.1][current_room.0];

    world.enemy_bullets.retain_mut(|bullet| {
        if world.other_players.contains_key(&bullet.owner_id) {
            bullet.update(dt, level);
            bullet.active
        } else {
            false
        }
    });
}
pub fn check_player_death(world: &mut World) {
    let mut died = false;

    for bullet in &mut world.enemy_bullets {
        if bullet.active && bullet.check_collision(&world.player) {
            bullet.active = false;
            died = true;
            break;
        }
    }

    if died {
        handle_death(world);
    }
}
fn handle_death(world: &mut World) {
    if let Some(my_id) = world.id {
        world.network.send(GamePacket::Hit { entity_id: my_id });
    }

    println!("You died! Returning to start...");
    world.current_coords = (0, 0);
    world.player.pos = glam::Vec2::new(100.0, 100.0);
    // Clear bullet tracking on death/respawn
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();
    load_current_level(world);
}

fn load_current_level(world: &mut World) {
    world.enemies.clear();
    world.bullets.clear();
    world.enemy_bullets.clear();
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();

    // Request authoritative enemy state from server
    let (rx, ry) = world.current_coords;
    let req = GamePacket::EnemyStateRequest {
        room_x: rx as u8,
        room_y: ry as u8,
    };
    world.network.send(req);

    println!(
        "Level {},{} load requested (EnemyStateRequest sent)",
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
    }
}
