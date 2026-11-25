//! Fixed-timestep accumulator shared by the client GUI loop and the e2e tests.
//!
//! The accumulator pattern (spec D1): simulation runs at a fixed tick rate
//! (`TICK_HZ`), independent of the variable render rate. Each render frame
//! accumulates the real elapsed `frame_dt`, clamps it to `max_frames * tick_dt`
//! (spiral-of-death protection), runs 0..=max_frames fixed steps, and exposes an
//! interpolation `alpha` so the renderer can lerp between ticks.

/// Accumulate → clamp → step → alpha, in one place.
///
/// Both the production GUI loop (`client/src/main.rs`) and the e2e accumulator
/// test (`e2e/tests/gui_accumulator.rs`) drive their simulation through this
/// struct so the timing logic has a single source of truth (QA finding m2).
pub struct FixedTimestep {
    accumulator: f64,
    tick_dt: f64,
    max_accumulator: f64,
    max_frames: u32,
}

impl FixedTimestep {
    /// Construct from a tick rate and a max-frames clamp. `max_accumulator` is
    /// derived as `max_frames as f64 * tick_dt` (matching production).
    pub fn new(tick_hz: u32, max_frames: u32) -> Self {
        let tick_dt = 1.0 / tick_hz as f64;
        let max_accumulator = max_frames as f64 * tick_dt;
        Self {
            accumulator: 0.0,
            tick_dt,
            max_accumulator,
            max_frames,
        }
    }

    /// Construct with an explicit `max_accumulator`, decoupled from `max_frames`.
    ///
    /// Production always ties the two (`max_accumulator = max_frames * tick_dt`),
    /// which makes the `steps < max_frames` guard unreachable because the clamp
    /// already bounds the accumulator to `max_frames` ticks. This constructor
    /// exists so a test can make the guard the binding constraint and prove it
    /// actually caps the step count.
    pub fn with_max_accumulator(tick_hz: u32, max_frames: u32, max_accumulator: f64) -> Self {
        let tick_dt = 1.0 / tick_hz as f64;
        Self {
            accumulator: 0.0,
            tick_dt,
            max_accumulator,
            max_frames,
        }
    }

    /// Accumulate `frame_dt`, clamp, then run 0..=max_frames fixed steps.
    /// Returns the number of steps executed this frame.
    pub fn advance(&mut self, frame_dt: f64) -> u32 {
        self.accumulator += frame_dt;
        if self.accumulator > self.max_accumulator {
            self.accumulator = self.max_accumulator;
        }

        let mut steps = 0u32;
        while self.accumulator >= self.tick_dt && steps < self.max_frames {
            self.accumulator -= self.tick_dt;
            steps += 1;
        }
        steps
    }

    /// Interpolation alpha in [0, 1) for rendering between ticks.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.tick_dt) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TICK_HZ;

    /// AC9 (clamp): a huge `frame_dt` is clamped to `max_frames` ticks and
    /// advances exactly `max_frames` steps, leaving a well-formed `alpha`.
    #[test]
    fn advance_clamps_huge_frame_dt() {
        let mut ts = FixedTimestep::new(TICK_HZ, 5);
        let steps = ts.advance(1.0);
        assert_eq!(steps, 5);
        assert!((0.0..1.0).contains(&ts.alpha()));
    }

    /// The `max_frames` guard (not the clamp) is the binding constraint when the
    /// accumulator is decoupled: a 1s frame against a 10s `max_accumulator`
    /// would drain ~60 ticks without the guard, but must run exactly
    /// `max_frames`.
    ///
    /// This test FAILS if the `steps < max_frames` guard is removed from
    /// `advance`, because the clamp never engages here (1.0 < 10.0) and the
    /// `while` loop would otherwise drain the full accumulator.
    #[test]
    fn advance_max_frames_guard_caps_steps() {
        let mut ts = FixedTimestep::with_max_accumulator(TICK_HZ, 2, 10.0);
        let steps = ts.advance(1.0);
        assert_eq!(steps, 2);
    }
}
