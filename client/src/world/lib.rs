use crate::world::game::World;
use game::entities::enemy::enemies_from_data;
use macroquad::prelude::*;
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
        let death_msg = GamePacket::Hit { entity_id: my_id }; // O GamePacket::Death
        let bytes = postcard::to_allocvec(&death_msg).unwrap();

        let _ = world.network.sender.send(laminar::Packet::reliable_ordered(
            world.network.server_addr,
            bytes,
            None,
        ));
    }

    println!("¡Has muerto! Volviendo al inicio...");
    world.current_coords = (0, 0);
    world.player.pos = vec2(100.0, 100.0);
    load_current_level(world);
}

fn load_current_level(world: &mut World) {
    world.enemies.clear();
    world.bullets.clear();

    let level = &world.levels_matrix[world.current_coords.1][world.current_coords.0];
    world.enemies = enemies_from_data(&level.data);

    println!(
        "Nivel {},{} cargado con {} enemigos",
        world.current_coords.0,
        world.current_coords.1,
        world.enemies.len()
    );
}
pub fn check_level_transitions(world: &mut World) {
    let (cx, cy) = world.current_coords;
    let level_w = world.levels_matrix[cy][cx].pixel_width();
    let level_h = world.levels_matrix[cy][cx].pixel_height();
    let p = &mut world.player;

    let mut changed = false;

    // --- SALIDA DERECHA ---
    if p.pos.x + p.w > level_w - 5.0 {
        // Un pequeño margen de 5px
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
            "Cambiado a nivel: {},{}",
            world.current_coords.0, world.current_coords.1
        );
        load_current_level(world);
    }
}
