//! The noise sources: white, pink, brown, and a Geiger crackle.

use super::rng::Rng;
use serde::{Deserialize, Serialize};

/// The kinds of noise. Each one is a different spectrum, from flat to dark, or a crackle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum NoiseType {
    /// Equal energy at every frequency.
    #[default]
    White = 0,
    /// Equal energy per octave, so it sounds even across the range.
    Pink = 1,
    /// Random walk, which sounds dark and rumbling.
    Brown = 2,
    /// Random clicks that decay quickly, like a counter.
    Geiger = 3,
}

impl NoiseType {
    pub const ALL: [NoiseType; 4] = [
        NoiseType::White,
        NoiseType::Pink,
        NoiseType::Brown,
        NoiseType::Geiger,
    ];

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Unknown values fall back to white.
    pub fn from_u8(value: u8) -> Self {
        Self::ALL.get(value as usize).copied().unwrap_or_default()
    }
}

/// What one voice's noise remembers between samples. Each voice has its own, so voices don't
/// share a random walk or a click.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoiseState {
    /// The pink filter's stages.
    pink: [f32; 7],
    /// The brown walk's position.
    brown: f32,
    /// The current click's level, which decays.
    click: f32,
}

impl NoiseState {
    /// The next sample of a noise, from -1.0 to 1.0.
    pub fn sample(&mut self, kind: NoiseType, rng: &mut Rng) -> f32 {
        let white = rng.signed();
        match kind {
            NoiseType::White => white,
            // Paul Kellet's refinement of pink noise: white through seven leaky stages.
            NoiseType::Pink => {
                let p = &mut self.pink;
                p[0] = 0.99886 * p[0] + white * 0.055_517_9;
                p[1] = 0.99332 * p[1] + white * 0.075_075_9;
                p[2] = 0.969_00 * p[2] + white * 0.153_852_0;
                p[3] = 0.866_50 * p[3] + white * 0.310_485_6;
                p[4] = 0.550_00 * p[4] + white * 0.532_952_2;
                p[5] = -0.761_6 * p[5] - white * 0.016_898_0;
                let sum: f32 = p.iter().sum::<f32>() + white * 0.5362;
                p[6] = white * 0.115_926;
                sum * 0.11
            }
            // A leaky random walk. It drifts slowly, so it stays between the ends.
            NoiseType::Brown => {
                self.brown = (self.brown + 0.02 * white) / 1.02;
                (self.brown * 3.5).clamp(-1.0, 1.0)
            }
            // About 60 clicks a second at 44.1 kHz. Each one jumps to a random level and decays.
            NoiseType::Geiger => {
                if rng.unit() < 0.0014 {
                    self.click = white;
                }
                self.click *= 0.996;
                self.click
            }
        }
    }
}
