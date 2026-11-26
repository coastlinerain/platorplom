use crate::world::game::World;
use game::entities::player::Player;

pub fn update_remote_bullets_position(world: &mut World, dt: f32) {
    let current_room = world.current_coords;
    let level = &world.levels_matrix[current_room.1][current_room.0];

    world.remote_bullets.retain_mut(|bullet| {
        if world.other_players.contains_key(&bullet.owner_id) {
            bullet.update(dt, level);
            bullet.active
        } else {
            false
        }
    });
}
/// Applies a server-authoritative respawn (`PlayerDied` naming our own id).
/// Rebuilds the local player from the canonical spawn so the next `PlayerPos`
/// reconciles cleanly; never sends a packet.
pub fn respawn_local(world: &mut World) {
    println!("You died! Returning to start...");
    world.current_coords = (0, 0);
    // Match the server's `Player::new` respawn exactly (pos/vel/grounded/dir).
    let id = world.player.id;
    world.player = Player::new(id);
    // Snap the interpolation source to the respawn position so the next
    // `draw(alpha)` renders at the spawn point instead of lerping from a
    // stale pre-death position (defensive — the prev-state store in
    // `World::update` already runs after this, but keep the invariant local).
    world.prev_player_pos = world.player.pos;
    world.prev_player_dir = world.player.dir;
    // Clear bullet tracking on death/respawn
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();
    // Reset clock sync on death
    world.client_tick = 0;
    world.server_tick_offset = 0;
    world.clock_offset_ms = 0;
    world.last_server_tick = 0;
    world.last_server_time_ms = 0;
    world.last_acked_server_tick = 0;
    load_current_level(world);
}

fn load_current_level(world: &mut World) {
    world.bullets.clear();
    world.remote_bullets.clear();
    world.spawned_bullet_ids.clear();
    world.confirmed_bullets.clear();
    // Reset clock sync on room transition
    world.client_tick = 0;
    world.server_tick_offset = 0;
    world.clock_offset_ms = 0;
    world.last_server_tick = 0;
    world.last_server_time_ms = 0;
    world.last_acked_server_tick = 0;
    // Reset sticky edges to avoid carry-over from previous room
    world.jump_sticky = 0;
    world.shoot_sticky = 0;

    println!(
        "Level {},{} loaded",
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
        // Snap the interpolation source to the post-teleport entry position so
        // `draw(alpha)` renders at the new-room spawn instead of lerping from
        // the old-room position for one frame (the level is already drawn at
        // the new coords).
        world.prev_player_pos = world.player.pos;
        world.prev_player_dir = world.player.dir;
    }
}
