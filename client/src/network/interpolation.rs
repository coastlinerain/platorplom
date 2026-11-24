use game::entities::nemesis::Nemesis;
use std::time::Instant;

/// Per-remote-player interpolation state (client-only, not in `game`).
///
/// `Nemesis` holds the raw positions (`interp_snapshots`); this struct holds the
/// arrival timestamps needed to compute the lerp factor.
#[derive(Debug)]
pub struct RemoteInterp {
    /// When each snapshot arrived. Parallels `nemesis.interp_snapshots`:
    /// `timestamps[0]` corresponds to the older snapshot (if any),
    /// `timestamps[1]` to the newer.
    pub timestamps: [Option<Instant>; 2],
}

impl RemoteInterp {
    pub fn new() -> Self {
        todo!("implementer: RemoteInterp::new")
    }

    /// Push a new snapshot into the Nemesis and record its arrival time.
    ///
    /// Internally calls `nemesis.push_interp_snapshot(x, y, dir)` and shifts
    /// timestamps to keep the 2 most recent entries in sync.
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32) {
        todo!("implementer: RemoteInterp::push")
    }

    /// Compute interpolated `(x, y, dir)` from the Nemesis snapshot buffer and
    /// the recorded timestamps.
    ///
    /// - **0 snapshots**: returns `None`.
    /// - **1 snapshot**: returns that snapshot's `(x, y, dir)` (snap).
    /// - **2 snapshots**: lerps position between older and newer based on
    ///   elapsed time since the older timestamp, clamped to [0, 1] so that
    ///   elapsed > interval snaps to the newer position. `dir` always snaps to
    ///   the newer snapshot.
    /// - **Stale guard**: if the newer snapshot's timestamp is >200 ms old,
    ///   snaps to the newer position (treats the remote as disconnected).
    pub fn interpolate(&self, nemesis: &Nemesis) -> Option<(f32, f32, f32)> {
        todo!("implementer: RemoteInterp::interpolate")
    }

    /// Same as `interpolate` but accepts an explicit `now` for testability.
    /// `interpolate` delegates to this with `Instant::now()`.
    pub fn interpolate_at(&self, nemesis: &Nemesis, now: Instant) -> Option<(f32, f32, f32)> {
        todo!("implementer: RemoteInterp::interpolate_at")
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteInterp;
    use game::entities::nemesis::Nemesis;
    use std::time::{Duration, Instant};

    /// Helper: create a Nemesis with the new interpolation fields.
    /// Returns a Nemesis at position (x, y).
    fn make_nemesis(id: u8, x: f32, y: f32) -> Nemesis {
        let mut n = Nemesis::new(id, glam::Vec2::new(x, y));
        n.interp_snapshots = [None, None];
        n.interp_len = 0;
        n
    }

    // -----------------------------------------------------------------------
    // Group D — Entity interpolation tests
    // -----------------------------------------------------------------------

    /// AC 18 (empty case) — no snapshots → interpolate returns None.
    #[test]
    fn interpolate_empty_returns_none() {
        let interp = RemoteInterp::new();
        let nemesis = make_nemesis(1, 0.0, 0.0);

        let now = Instant::now();
        let result = interp.interpolate_at(&nemesis, now);

        assert!(result.is_none(), "empty interp should return None");
    }

    /// AC 18 (single case) — one snapshot → snap to it.
    #[test]
    fn interpolate_single_snapshot_returns_it() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        interp.push(&mut nemesis, 100.0, 200.0, 1.0);

        // Use a now that is close to the push time (the timestamp was recorded
        // during push, so we can't match it exactly — but with 1 snapshot the
        // result should be the snapshot regardless of elapsed).
        let now = Instant::now();
        let result = interp.interpolate_at(&nemesis, now);

        assert!(result.is_some(), "single snapshot should return Some");
        let (x, y, dir) = result.unwrap();
        assert_eq!(x, 100.0);
        assert_eq!(y, 200.0);
        assert_eq!(dir, 1.0);
    }

    /// AC 19 — two snapshots, lerp at 50% of the interval.
    #[test]
    fn interpolate_two_snapshots_lerps_midpoint() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();

        // Push snapshot A at t0.
        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        // Overwrite recorded timestamps for deterministic testing.
        // The interp module needs a way to set timestamps directly or
        // we rely on the timestamps being recorded at push time.
        // For the test, we push and then manually set timestamps.
        interp.timestamps[0] = Some(t0);

        let t1 = t0 + Duration::from_millis(100);

        // Push snapshot B at t1.
        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);

        // Sample at 50ms after t0 (midpoint of 100ms interval).
        let now = t0 + Duration::from_millis(50);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(result.is_some(), "two snapshots should return Some");
        let (x, y, dir) = result.unwrap();

        // Position is lerp(A, B, 0.5): 100 + (200-100)*0.5 = 150.
        let epsilon = 0.5;
        assert!((x - 150.0).abs() < epsilon, "expected x ≈ 150.0, got {x}");
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
        // dir always snaps to the newest snapshot.
        assert_eq!(dir, -1.0);
    }

    /// AC 20 — elapsed > interval clamps to 1.0 (no overshoot extrapolation).
    #[test]
    fn interpolate_clamps_to_most_recent() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();

        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);

        let t1 = t0 + Duration::from_millis(50);
        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);

        // Sample at 200ms after t1 (well past the 50ms interval).
        let now = t1 + Duration::from_millis(200);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();

        // Should snap to the newer snapshot (200, 100, -1.0).
        let epsilon = 0.5;
        assert!(
            (x - 200.0).abs() < epsilon,
            "expected x ≈ 200.0 (clamped), got {x}"
        );
        assert!(
            (y - 100.0).abs() < epsilon,
            "expected y ≈ 100.0 (clamped), got {y}"
        );
        assert_eq!(dir, -1.0);
    }

    /// AC 20 (stale case) — a single snapshot older than 200ms still returns it.
    #[test]
    fn interpolate_stale_guard_snaps_to_latest() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();

        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);

        // Sample at 300ms after t0 (>200ms stale threshold).
        let now = t0 + Duration::from_millis(300);
        let result = interp.interpolate_at(&nemesis, now);

        // With 1 snapshot, stale guard should still return the snapshot
        // (treating >200ms as valid but "snapped").
        assert!(
            result.is_some(),
            "even stale, 1 snapshot should return Some"
        );
        let (x, y, dir) = result.unwrap();
        let epsilon = 0.5;
        assert!((x - 100.0).abs() < epsilon, "expected x ≈ 100.0, got {x}");
        assert_eq!(dir, 1.0);
    }

    /// AC 39 (edge case) — stale guard boundary test with 2 snapshots.
    /// At >200ms after the newest snapshot, interpolation still returns a
    /// position (snaps rather than interpolating, does NOT return None).
    #[test]
    fn interpolate_stale_guard_boundary() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();

        // Push snapshot A at t0.
        interp.push(&mut nemesis, 0.0, 0.0, 1.0);
        interp.timestamps[0] = Some(t0);

        // Push snapshot B at t0 + 50ms.
        let t1 = t0 + Duration::from_millis(50);
        interp.push(&mut nemesis, 10.0, 0.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);

        // Exactly 200ms after B (t1 + 200ms = t0 + 250ms).
        // With 2 snapshots where the newest is >200ms old, the stale guard
        // fires and snaps to the newest position.
        let now = t1 + Duration::from_millis(200);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(
            result.is_some(),
            "stale guard should still return a position"
        );
        let (x, _y, dir) = result.unwrap();

        // Snaps to B's position (10.0, 0.0, -1.0) rather than interpolating.
        let epsilon = 0.5;
        assert!(
            (x - 10.0).abs() < epsilon,
            "stale guard should snap to newest position; expected x≈10.0, got {x}"
        );
        assert_eq!(dir, -1.0);

        // Just over the threshold: 201ms after B.
        let now2 = t1 + Duration::from_millis(201);
        let result2 = interp.interpolate_at(&nemesis, now2);
        assert!(result2.is_some());
        let (x2, _, _) = result2.unwrap();
        assert!(
            (x2 - 10.0).abs() < epsilon,
            "201ms stale should also snap; got {x2}"
        );
    }

    /// AC 17 — pushing a third snapshot evicts the oldest, keeping only the
    /// two most recent.
    #[test]
    fn interpolate_evicts_old_snapshots() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        // Push A, B, C.
        interp.push(&mut nemesis, 0.0, 0.0, 1.0); // A
        interp.push(&mut nemesis, 10.0, 10.0, 1.0); // B
        interp.push(&mut nemesis, 20.0, 20.0, -1.0); // C

        // Verify only B and C remain in nemesis.interp_snapshots.
        assert_eq!(nemesis.interp_len, 2);
        assert_eq!(nemesis.interp_snapshots[0], Some((10.0, 10.0, 1.0)));
        assert_eq!(nemesis.interp_snapshots[1], Some((20.0, 20.0, -1.0)));

        // Verify only B and C timestamps remain (positions match).
        assert!(interp.timestamps[0].is_some());
        assert!(interp.timestamps[1].is_some());
    }
}
