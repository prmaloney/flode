//! A delay line with a fractional read. Reading it at a delay that moves with a signal is phase
//! modulation, which is how the mix stage does FM on a signal that is already rendered.

/// Samples a delay line holds. It sets the longest delay, and so how deep an FM can swing at a
/// low pitch.
pub const DELAY_CAPACITY: usize = 4096;

/// A circular buffer of the most recent samples.
pub struct DelayLine {
    buffer: Box<[f32]>,
    /// The index the next sample goes to.
    write: usize,
}

impl DelayLine {
    /// A line of `DELAY_CAPACITY` samples. It allocates, so it is built before the audio thread
    /// runs.
    pub fn new() -> Self {
        Self::with_capacity(DELAY_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity >= 8, "a delay line needs room for interpolation");
        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            write: 0,
        }
    }

    /// The longest delay `read` can reach.
    pub fn max_delay(&self) -> f32 {
        (self.buffer.len() - 3) as f32
    }

    /// Forgets everything, so a new note starts from silence.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write = 0;
    }

    /// Adds a sample. It is the newest, at delay 0.
    pub fn push(&mut self, sample: f32) {
        self.buffer[self.write] = sample;
        self.write = (self.write + 1) % self.buffer.len();
    }

    /// The signal `delay` samples ago, interpolated between samples with a cubic. A delay of 1.0
    /// is the sample before the newest. Delays outside the line are clamped to its ends, and the
    /// shortest is one sample, because the interpolation reads one sample newer.
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let delay = delay.clamp(1.0, self.max_delay());
        let whole = delay as usize;
        let t = delay - whole as f32;
        let newest = self.write + len - 1;
        let at = |back: usize| self.buffer[(newest - back) % len];
        let (newer, near, far, farther) = (at(whole - 1), at(whole), at(whole + 1), at(whole + 2));

        // A Catmull-Rom curve through `near` and `far`, shaped by their neighbors.
        let c1 = 0.5 * (far - newer);
        let c2 = newer - 2.5 * near + 2.0 * far - 0.5 * farther;
        let c3 = 0.5 * (farther - newer) + 1.5 * (near - far);
        ((c3 * t + c2) * t + c1) * t + near
    }
}

impl Default for DelayLine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn filled(samples: &[f32]) -> DelayLine {
        let mut line = DelayLine::with_capacity(64);
        for &s in samples {
            line.push(s);
        }
        line
    }

    /// A whole-sample delay returns the exact sample from that long ago.
    #[test]
    fn a_whole_delay_returns_the_exact_sample() {
        let line = filled(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(line.read(1.0), 4.0);
        assert_eq!(line.read(2.0), 3.0);
        assert_eq!(line.read(4.0), 1.0);
    }

    /// Between two samples, a ramp reads back as the ramp.
    #[test]
    fn a_ramp_reads_back_linearly_between_samples() {
        let line = filled(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert!((line.read(2.5) - 3.5).abs() < 1e-5);
        assert!((line.read(3.25) - 2.75).abs() < 1e-5);
    }

    /// A slow sine read at a fractional delay matches the sine shifted by that delay.
    #[test]
    fn a_fractional_delay_shifts_a_sine_accurately() {
        let freq = 0.02;
        let mut line = DelayLine::with_capacity(256);
        for n in 0..200 {
            line.push((TAU * freq * n as f32).sin());
        }
        for delay in [3.3, 10.7, 40.5] {
            let expected = (TAU * freq * (199.0 - delay)).sin();
            assert!((line.read(delay) - expected).abs() < 2e-3, "delay {delay}");
        }
    }

    /// The write index wraps, and reads still find the right samples.
    #[test]
    fn reads_survive_the_buffer_wrapping() {
        let mut line = DelayLine::with_capacity(16);
        for n in 0..100 {
            line.push(n as f32);
        }
        assert_eq!(line.read(1.0), 98.0);
        assert_eq!(line.read(5.0), 94.0);
    }

    /// Delays past either end are clamped instead of reading garbage.
    #[test]
    fn out_of_range_delays_are_clamped() {
        let line = filled(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(line.read(-5.0), line.read(1.0));
        assert_eq!(line.read(1e9), line.read(line.max_delay()));
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut line = filled(&[1.0, 2.0, 3.0]);
        line.clear();
        assert_eq!(line.read(1.0), 0.0);
        assert_eq!(line.read(2.0), 0.0);
    }
}
