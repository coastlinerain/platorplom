pub const TILE_SIZE: f32 = 40.0;
pub const GRAVEDAD: f32 = 20.0;
pub const SALTO_FUERZA: f32 = -1000.0;
pub const VELOCIDAD: f32 = 400.0;
pub const BULLET_SPEED: f32 = 400.0;
pub const MAP_WIDTH: usize = 20;
pub const MAP_HEIGHT: usize = 12;
pub const TICK_HZ: u32 = 60;

/// Approximate client render frame rate. Distinct from `TICK_HZ` (the fixed
/// simulation rate): reconciliation replay and interpolation use `TICK_HZ`, not
/// this. Named `RENDER_HZ` to reflect the display-refresh semantics. Currently
/// unused (documented for future render-rate-aware code); must track the
/// typical display refresh rate if it is ever consumed.
pub const RENDER_HZ: u32 = 60;

/// Number of server ticks to delay rendering behind the estimated server tick.
/// This provides headroom for the jitter buffer to absorb network variance.
/// At 60Hz, 5 ticks = ~83ms delay.
pub const INTERPOLATION_DELAY_TICKS: u8 = 5;

/// Maximum number of server ticks a client can be behind before forcing
/// a reconciliation (staleness heuristic).
pub const STALENESS_THRESHOLD_TICKS: u64 = 5;

/// Maximum simulation steps per render frame (spiral of death protection).
/// At 60Hz, 5 steps = ~83ms of sim catch-up per frame (safety clamp, not a
/// render floor).
pub const MAX_ACCUMULATED_FRAMES: u32 = 5;

/// Headless sleep precision: busy-wait for the last N milliseconds of each
/// tick interval (the remainder is covered by `std::thread::sleep`).
pub const HEADLESS_BUSY_WAIT_MS: u64 = 1;

/// Client connect timeout: give up waiting for `JoinResponse` after this long.
pub const CONNECT_TIMEOUT_MS: u64 = 5000;

/// Disconnect watchdog: fire `ConnectionLost` if no `PlayerPos` for this long.
pub const DISCONNECT_TIMEOUT_MS: u64 = 1000;

/// Server: minimum ticks between room-change responses for one player (an
/// accepted change or a rejection arms it). ~100 ms at 60 Hz. Tunable if door
/// ping-pong becomes a real problem.
pub const ROOM_CHANGE_COOLDOWN_TICKS: u8 = 6;

/// Client: ticks to wait after a rejected request before retrying.
pub const ROOM_CHANGE_RETRY_TICKS: u8 = 6;

/// Server-side only: extra px added to a door's geometric trigger on the
/// approach edge before `can_enter` validates. Covers one prediction tick
/// (~VELOCIDAD / TICK_HZ ≈ 6.667 px) with margin. The client's `door_at`
/// detection does not use this.
pub const DOOR_LEAD_TOLERANCE_PX: f32 = 8.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// Box 1 (AC1): fixed-timestep constants required by the accumulator
    /// pattern and the headless hybrid sleep + busy-wait loop.
    #[test]
    fn fixed_timestep_constants() {
        assert_eq!(MAX_ACCUMULATED_FRAMES, 5);
        assert_eq!(HEADLESS_BUSY_WAIT_MS, 1);
    }

    /// Spec 3 fix (m2): the room-change cooldown is deliberately short (~100 ms
    /// at 60 Hz) so an honest client can reverse quickly while a modified one
    /// still gets at most one `RoomChangeResponse` per window.
    #[test]
    fn room_change_cooldown_is_six_ticks() {
        assert_eq!(ROOM_CHANGE_COOLDOWN_TICKS, 6);
    }
}
