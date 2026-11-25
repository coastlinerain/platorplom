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
