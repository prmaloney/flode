//! Gain conversion.

use nih_plug::util;

/// A level at or below this many dB is exactly silent.
pub const SILENT_DB: f32 = -30.0;

/// The linear gain for a level in dB. A level at the floor is exactly silent.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        util::db_to_gain_fast(db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_silent_and_zero_db_is_unity() {
        assert_eq!(db_to_linear(SILENT_DB), 0.0);
        assert_eq!(db_to_linear(-60.0), 0.0);
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0) - 0.501).abs() < 0.01);
    }
}
