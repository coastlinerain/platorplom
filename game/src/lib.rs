pub mod config;
pub mod entities;
pub mod fixed_timestep;
pub mod level;
pub mod maps;
pub mod rect;
pub mod traits;
pub mod types;

/// Wrapping-aware sequence comparison. Returns `true` when `a` is newer than `b`
/// by 1..32767 (inclusive), `false` otherwise (including equal and older).
/// The threshold of 32767 (half the u16 range) correctly distinguishes "newer"
/// from "far behind due to wraparound."
///
pub fn is_newer_seq(a: u16, b: u16) -> bool {
    let diff = a.wrapping_sub(b);
    diff > 0 && diff < 32768
}

#[cfg(test)]
mod is_newer_seq_tests {
    /// AC 10 — `a` ahead of `b` by a small positive delta is newer.
    #[test]
    fn is_newer_seq_ahead() {
        assert!(super::is_newer_seq(10, 5));
    }

    /// Equal sequences are not newer.
    #[test]
    fn is_newer_seq_equal() {
        assert!(!super::is_newer_seq(5, 5));
    }

    /// A smaller seq is not newer than a larger one.
    #[test]
    fn is_newer_seq_behind() {
        assert!(!super::is_newer_seq(5, 10));
    }

    /// Wraparound: 0 is newer than u16::MAX (the counter just wrapped).
    /// u16::MAX is NOT newer than 0.
    /// seq 1 is newer than 65535 (within the 32767 threshold).
    #[test]
    fn is_newer_seq_wraparound() {
        assert!(super::is_newer_seq(0, u16::MAX));
        assert!(!super::is_newer_seq(u16::MAX, 0));
        assert!(super::is_newer_seq(1, 65535));
    }

    /// A difference > 32767 means the smaller value is actually newer after
    /// wraparound, so the larger value is NOT newer.
    #[test]
    fn is_newer_seq_beyond_threshold() {
        assert!(!super::is_newer_seq(40000, 0));
    }
}
