use game::config::TICK_HZ;
use game::entities::nemesis::Nemesis;
use std::time::Instant;

/// Per-remote-player interpolation state (client-only, not in `game`).
///
/// `Nemesis` holds the raw positions (`interp_snapshots`); this struct holds the
/// arrival timestamps needed to compute the lerp factor.
#[derive(Debug)]
pub struct RemoteInterp {
    /// When each snapshot arrived. Parallels `nemesis.interp_snapshots`:
    /// `timestamps[0..4]` corresponds to snapshots in order (oldest to newest).
    pub timestamps: [Option<Instant>; 5],
    /// The server tick from the most recent PlayerPos packet.
    pub last_known_server_tick: u64,
    /// The local Instant when the most recent PlayerPos packet arrived.
    pub last_known_arrival: Instant,
}

impl RemoteInterp {
    pub fn new() -> Self {
        Self {
            timestamps: [None, None, None, None, None],
            last_known_server_tick: 0,
            last_known_arrival: Instant::now(),
        }
    }

    /// Push a new snapshot into the Nemesis and record its arrival time.
    ///
    /// Internally calls `nemesis.push_interp_snapshot(x, y, dir)` and shifts
    /// timestamps to match the 5-entry ring buffer eviction.
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32) {
        let was_full = nemesis.interp_len == 5;
        nemesis.push_interp_snapshot(x, y, dir);

        let now = Instant::now();
        if was_full {
            // Shift timestamps left by 1: [1]→[0], [2]→[1], [3]→[2], [4]→[3], new at [4]
            self.timestamps[0] = self.timestamps[1];
            self.timestamps[1] = self.timestamps[2];
            self.timestamps[2] = self.timestamps[3];
            self.timestamps[3] = self.timestamps[4];
            self.timestamps[4] = Some(now);
        } else {
            // Fill next empty slot
            self.timestamps[nemesis.interp_len as usize - 1] = Some(now);
        }
        // Update the anchor for tick-based interpolation
        self.last_known_server_tick = nemesis.interp_len as u64; // placeholder, will be set by caller
        self.last_known_arrival = now;
    }

    /// Set the anchor server tick and arrival time (called after push with the actual server_tick from packet).
    pub fn set_anchor(&mut self, server_tick: u64) {
        self.last_known_server_tick = server_tick;
        self.last_known_arrival = Instant::now();
    }

    /// Interpolate at a specific target render tick.
    ///
    /// - `target_tick`: The server tick we want to render for (server_tick_estimate - INTERPOLATION_DELAY_TICKS).
    /// - `last_known_server_tick`: The server_tick from the most recent PlayerPos.
    /// - `last_known_arrival`: The Instant when that PlayerPos arrived.
    ///
    /// Returns `Some((x, y, dir))` with interpolated position, or `None` if no snapshots.
    pub fn interpolate_at_tick(
        &self,
        nemesis: &Nemesis,
        target_tick: u64,
        last_known_server_tick: u64,
        last_known_arrival: Instant,
    ) -> Option<(f32, f32, f32)> {
        // Collect valid snapshots and their timestamps
        let mut snapshots = Vec::new();
        let mut snapshot_times = Vec::new();

        for i in 0..nemesis.interp_len as usize {
            if let Some(snap) = nemesis.interp_snapshots[i]
                && let Some(ts) = self.timestamps[i]
            {
                snapshots.push(snap);
                snapshot_times.push(ts);
            }
        }

        let count = snapshots.len();
        if count == 0 {
            return None;
        }
        if count == 1 {
            let (x, y, dir) = snapshots[0];
            return Some((x, y, dir));
        }

        // Convert target_tick to expected arrival time:
        // target_arrival = last_known_arrival + (target_tick - last_known_server_tick) * TICK_MS
        // where TICK_MS = 1000 / TICK_HZ (approx 16.67ms at 60Hz)
        let tick_duration_ms = 1000.0 / TICK_HZ as f32;
        let tick_diff = target_tick as i64 - last_known_server_tick as i64;
        let target_arrival_offset_ms = tick_diff as f32 * tick_duration_ms;

        let target_arrival = if target_arrival_offset_ms >= 0.0 {
            last_known_arrival + std::time::Duration::from_millis(target_arrival_offset_ms as u64)
        } else {
            // target_tick is before last_known_server_tick (shouldn't happen normally, but handle)
            last_known_arrival
                .checked_sub(std::time::Duration::from_millis(
                    (-target_arrival_offset_ms) as u64,
                ))
                .unwrap_or(last_known_arrival)
        };

        // Stale guard: if newest snapshot > 200ms old relative to target_arrival, snap to newest
        if let Some(newest_ts) = snapshot_times.last()
            && target_arrival
                .checked_duration_since(*newest_ts)
                .is_some_and(|d| d.as_millis() > 200)
        {
            let (x, y, dir) = snapshots[count - 1];
            return Some((x, y, dir));
        }

        // Find two snapshots whose arrival times bracket target_arrival
        // We want i such that snapshot_times[i] <= target_arrival <= snapshot_times[i+1]
        let mut bracket_idx = None;
        for i in 0..count - 1 {
            if snapshot_times[i] <= target_arrival && target_arrival <= snapshot_times[i + 1] {
                bracket_idx = Some(i);
                break;
            }
        }

        match bracket_idx {
            Some(i) => {
                // Interpolate between snapshots[i] and snapshots[i+1]
                let (ox, oy, _odir) = snapshots[i];
                let (nx, ny, ndir) = snapshots[i + 1];
                let older_ts = snapshot_times[i];
                let newer_ts = snapshot_times[i + 1];

                let interval = newer_ts.duration_since(older_ts);
                let elapsed = target_arrival.duration_since(older_ts);

                let t = if interval.as_secs_f32() > 0.0 {
                    (elapsed.as_secs_f32() / interval.as_secs_f32()).clamp(0.0, 1.0)
                } else {
                    1.0
                };

                let x = ox + (nx - ox) * t;
                let y = oy + (ny - oy) * t;
                Some((x, y, ndir)) // Direction snaps to newer
            }
            None => {
                // Target is outside the range of buffered snapshots
                if target_arrival < snapshot_times[0] {
                    // Before oldest: snap to oldest
                    let (x, y, dir) = snapshots[0];
                    Some((x, y, dir))
                } else {
                    // After newest: snap to newest
                    let (x, y, dir) = snapshots[count - 1];
                    Some((x, y, dir))
                }
            }
        }
    }

    /// Legacy interpolation method (wall-clock based) for backward compatibility.
    /// Delegates to `interpolate_at_tick` with current estimates.
    pub fn interpolate(&self, nemesis: &Nemesis) -> Option<(f32, f32, f32)> {
        // Use current time as target arrival, with server_tick_estimate
        // This is kept for backward compatibility but the new tick-based method is preferred.
        self.interpolate_at(nemesis, Instant::now())
    }

    /// Same as `interpolate` but accepts an explicit `now` for testability.
    /// `interpolate` delegates to this with `Instant::now()`.
    pub fn interpolate_at(&self, nemesis: &Nemesis, now: Instant) -> Option<(f32, f32, f32)> {
        match (nemesis.interp_snapshots[0], nemesis.interp_snapshots[1]) {
            (None, None) => None,
            (Some(pos), None) | (None, Some(pos)) => Some(pos),
            (Some(older), Some(newer)) => {
                let (ox, oy, _odir) = older;
                let (nx, ny, ndir) = newer;

                // Stale guard: if the newer snapshot's timestamp is > 200 ms in
                // the past, snap to the newest position.
                if let Some(newer_ts) = self.timestamps[1]
                    && now
                        .checked_duration_since(newer_ts)
                        .is_some_and(|d| d.as_millis() > 200)
                {
                    return Some((nx, ny, ndir));
                }

                // Compute lerp factor t = elapsed / interval, clamped to [0, 1].
                let elapsed_ms = self
                    .timestamps
                    .iter()
                    .zip(&[older, newer])
                    .filter_map(|(ts, _pos)| *ts)
                    .collect::<Vec<_>>();
                if elapsed_ms.len() < 2 {
                    return Some((nx, ny, ndir));
                }

                let interval = elapsed_ms[1]
                    .checked_duration_since(elapsed_ms[0])
                    .unwrap_or_default();
                let elapsed = now
                    .checked_duration_since(elapsed_ms[0])
                    .unwrap_or_default();

                let t = if interval.as_secs_f32() > 0.0 {
                    (elapsed.as_secs_f32() / interval.as_secs_f32()).clamp(0.0, 1.0)
                } else {
                    1.0
                };

                let x = ox + (nx - ox) * t;
                let y = oy + (ny - oy) * t;
                Some((x, y, ndir))
            }
        }
    }
}

impl Default for RemoteInterp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteInterp;
    use game::entities::nemesis::Nemesis;
    use std::time::{Duration, Instant};

    /// Helper: create a Nemesis with the new 5-entry interpolation fields.
    /// Returns a Nemesis at position (x, y).
    fn make_nemesis(id: u8, x: f32, y: f32) -> Nemesis {
        let mut n = Nemesis::new(id, glam::Vec2::new(x, y));
        n.interp_snapshots = [None, None, None, None, None];
        n.interp_len = 0;
        n
    }

    // -----------------------------------------------------------------------
    // Group D — Entity interpolation tests (updated for N=5)
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

        let epsilon = 0.5;
        assert!((x - 150.0).abs() < epsilon, "expected x ≈ 150.0, got {x}");
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
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

        let now = t1 + Duration::from_millis(200);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();

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

        let now = t0 + Duration::from_millis(300);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(
            result.is_some(),
            "even stale, 1 snapshot should return Some"
        );
        let (x, _y, dir) = result.unwrap();
        let epsilon = 0.5;
        assert!((x - 100.0).abs() < epsilon, "expected x ≈ 100.0, got {x}");
        assert_eq!(dir, 1.0);
    }

    /// AC 39 (edge case) — stale guard boundary test with 2 snapshots.
    #[test]
    fn interpolate_stale_guard_boundary() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();

        interp.push(&mut nemesis, 0.0, 0.0, 1.0);
        interp.timestamps[0] = Some(t0);

        let t1 = t0 + Duration::from_millis(50);
        interp.push(&mut nemesis, 10.0, 0.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);

        let now = t1 + Duration::from_millis(200);
        let result = interp.interpolate_at(&nemesis, now);

        assert!(
            result.is_some(),
            "stale guard should still return a position"
        );
        let (x, _y, dir) = result.unwrap();

        let epsilon = 0.5;
        assert!(
            (x - 10.0).abs() < epsilon,
            "stale guard should snap to newest position; expected x≈10.0, got {x}"
        );
        assert_eq!(dir, -1.0);

        let now2 = t1 + Duration::from_millis(201);
        let result2 = interp.interpolate_at(&nemesis, now2);
        assert!(result2.is_some());
        let (x2, _, _) = result2.unwrap();
        assert!(
            (x2 - 10.0).abs() < epsilon,
            "201ms stale should also snap; got {x2}"
        );
    }

    /// AC 17 — pushing a 6th snapshot evicts the oldest, keeping only the
    /// five most recent.
    #[test]
    fn interpolate_evicts_old_snapshots() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        // Push A, B, C, D, E, F (6 pushes)
        interp.push(&mut nemesis, 0.0, 0.0, 1.0); // A
        interp.push(&mut nemesis, 10.0, 10.0, 1.0); // B
        interp.push(&mut nemesis, 20.0, 20.0, -1.0); // C
        interp.push(&mut nemesis, 30.0, 30.0, 1.0); // D
        interp.push(&mut nemesis, 40.0, 40.0, -1.0); // E
        interp.push(&mut nemesis, 50.0, 50.0, 1.0); // F

        // Verify only B, C, D, E, F remain (5 entries, A evicted)
        assert_eq!(nemesis.interp_len, 5);
        assert_eq!(nemesis.interp_snapshots[0], Some((10.0, 10.0, 1.0))); // B
        assert_eq!(nemesis.interp_snapshots[1], Some((20.0, 20.0, -1.0))); // C
        assert_eq!(nemesis.interp_snapshots[2], Some((30.0, 30.0, 1.0))); // D
        assert_eq!(nemesis.interp_snapshots[3], Some((40.0, 40.0, -1.0))); // E
        assert_eq!(nemesis.interp_snapshots[4], Some((50.0, 50.0, 1.0))); // F

        // Verify timestamps shifted correctly
        assert!(interp.timestamps[0].is_some());
        assert!(interp.timestamps[1].is_some());
        assert!(interp.timestamps[2].is_some());
        assert!(interp.timestamps[3].is_some());
        assert!(interp.timestamps[4].is_some());
    }

    // -----------------------------------------------------------------------
    // New tests for interpolate_at_tick (tick-based interpolation)
    // -----------------------------------------------------------------------

    /// UT3 — interpolate_at_tick: 2 snapshots, target at oldest tick → position from oldest, dir from newest.
    #[test]
    fn interpolate_at_tick_lerps_midpoint() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();
        let server_tick_0 = 1000u64;

        // Push snapshot A at server_tick 1000
        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);
        interp.set_anchor(server_tick_0);

        let t1 = t0 + Duration::from_millis(16); // ~1 tick at 60Hz
        let server_tick_1 = 1001u64;

        // Push snapshot B at server_tick 1001
        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);
        interp.set_anchor(server_tick_1);

        // Target tick = 1000 (exactly at snapshot A's tick)
        // target_arrival = t1 + (1000-1001)*16.67 = t0
        let result = interp.interpolate_at_tick(&nemesis, 1000, server_tick_1, t1);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();
        let epsilon = 0.5;
        // At target_tick=1000, position should be at snapshot A (100, 100)
        // Direction always snaps to newer snapshot (B), so dir = -1.0
        assert!((x - 100.0).abs() < epsilon, "expected x ≈ 100.0, got {x}");
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
        assert_eq!(dir, -1.0, "direction should snap to newer snapshot");
    }

    /// UT4 — interpolate_at_tick: target before oldest → snap to oldest.
    #[test]
    fn interpolate_at_tick_before_oldest_snaps_to_oldest() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();
        let server_tick_0 = 1000u64;

        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);
        interp.set_anchor(server_tick_0);

        let t1 = t0 + Duration::from_millis(16);
        let server_tick_1 = 1001u64;

        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);
        interp.set_anchor(server_tick_1);

        // Target tick = 999 (before oldest snapshot at 1000)
        let result = interp.interpolate_at_tick(&nemesis, 999, server_tick_1, t1);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();
        let epsilon = 0.5;
        assert!(
            (x - 100.0).abs() < epsilon,
            "expected x ≈ 100.0 (oldest), got {x}"
        );
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
        assert_eq!(dir, 1.0);
    }

    /// UT5 — interpolate_at_tick: target after newest → snap to newest (clamp).
    #[test]
    fn interpolate_at_tick_after_newest_snaps_to_newest() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();
        let server_tick_0 = 1000u64;

        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);
        interp.set_anchor(server_tick_0);

        let t1 = t0 + Duration::from_millis(16);
        let server_tick_1 = 1001u64;

        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);
        interp.set_anchor(server_tick_1);

        // Target tick = 1003 (after newest snapshot at 1001)
        let result = interp.interpolate_at_tick(&nemesis, 1003, server_tick_1, t1);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();
        let epsilon = 0.5;
        assert!(
            (x - 200.0).abs() < epsilon,
            "expected x ≈ 200.0 (newest), got {x}"
        );
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
        assert_eq!(dir, -1.0);
    }

    /// UT6 — interpolate_at_tick: stale guard (>200ms) → snap to newest.
    #[test]
    fn interpolate_at_tick_stale_guard_snaps_to_newest() {
        let mut interp = RemoteInterp::new();
        let mut nemesis = make_nemesis(1, 0.0, 0.0);

        let t0 = Instant::now();
        let server_tick_0 = 1000u64;

        interp.push(&mut nemesis, 100.0, 100.0, 1.0);
        interp.timestamps[0] = Some(t0);
        interp.set_anchor(server_tick_0);

        let t1 = t0 + Duration::from_millis(16);
        let server_tick_1 = 1001u64;

        interp.push(&mut nemesis, 200.0, 100.0, -1.0);
        interp.timestamps[0] = Some(t0);
        interp.timestamps[1] = Some(t1);
        interp.set_anchor(server_tick_1);

        // Target tick far in the future such that target_arrival is >200ms after newest timestamp
        // target_arrival = t1 + (target_tick - 1001) * 16.67
        // We want target_arrival - t1 > 200ms => (target_tick - 1001) * 16.67 > 200
        // => target_tick - 1001 > 12 => target_tick > 1013
        let result = interp.interpolate_at_tick(&nemesis, 1020, server_tick_1, t1);

        assert!(result.is_some());
        let (x, y, dir) = result.unwrap();
        let epsilon = 0.5;
        // Should snap to newest (200, 100, -1.0) due to stale guard
        assert!(
            (x - 200.0).abs() < epsilon,
            "expected x ≈ 200.0 (stale snap), got {x}"
        );
        assert!((y - 100.0).abs() < epsilon, "expected y ≈ 100.0, got {y}");
        assert_eq!(dir, -1.0);
    }
}
