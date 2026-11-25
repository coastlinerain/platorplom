pub const TILE_SIZE: f32 = 40.0;
pub const GRAVEDAD: f32 = 20.0;
pub const SALTO_FUERZA: f32 = -1000.0;
pub const VELOCIDAD: f32 = 400.0;
pub const BULLET_SPEED: f32 = 400.0;
pub const MAP_WIDTH: usize = 20;
pub const MAP_HEIGHT: usize = 12;
pub const TICK_HZ: u32 = 60;

/// Approximate client frame rate, used for replay dt during reconciliation and
/// interpolation interval clamping. Must match the typical display refresh rate.
pub const CLIENT_HZ: u32 = 60;

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
}
