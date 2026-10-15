//! The random number generator the audio thread uses. It never allocates.

/// A xorshift generator. One value is 32 bits of state, so voices and LFOs can each own one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng(u32);

impl Rng {
    /// A generator from a seed. Zero is a dead state for xorshift, so it becomes one.
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    /// The next number from 0.0 up to 1.0.
    pub fn unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x as f32 / 4_294_967_296.0
    }

    /// The next number from -1.0 up to 1.0.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(0x2545_F491)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_stay_in_range() {
        let mut rng = Rng::default();
        for _ in 0..10_000 {
            let x = rng.unit();
            assert!((0.0..1.0).contains(&x));
            assert!((-1.0..1.0).contains(&rng.signed()));
        }
    }

    #[test]
    fn a_zero_seed_still_runs() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.unit(), rng.unit());
    }

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!(a.unit(), b.unit());
        assert_eq!(a.unit(), b.unit());
    }
}
