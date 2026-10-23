//! Pitch math.

/// The octave, coarse and fine transpose knobs, combined into one sum of semitones. Each octave is
/// twelve semitones, and each semitone is a hundred cents.
pub fn semitones_of(octave: f32, coarse: f32, fine: f32) -> f32 {
    octave * 12.0 + coarse + fine / 100.0
}

/// The frequency ratio for a transpose given as a sum of semitones. A ratio of 2.0 is up an
/// octave.
pub fn ratio_of_semitones(semitones: f32) -> f32 {
    (semitones / 12.0).exp2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transpose_ratio(octave: f32, coarse: f32, fine: f32) -> f32 {
        ratio_of_semitones(semitones_of(octave, coarse, fine))
    }

    /// Zero on every knob is unity, each octave doubles, and twelve semitones make an octave.
    #[test]
    fn transpose_steps_map_to_ratios() {
        assert_eq!(transpose_ratio(0.0, 0.0, 0.0), 1.0);
        assert!((transpose_ratio(1.0, 0.0, 0.0) - 2.0).abs() < 1e-6);
        assert!((transpose_ratio(0.0, 12.0, 0.0) - 2.0).abs() < 1e-6);
        assert!((transpose_ratio(0.0, 0.0, -100.0) - transpose_ratio(0.0, -1.0, 0.0)).abs() < 1e-6);
    }
}
