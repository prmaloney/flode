//! Distortion: a waveshaper with a choice of curve, a drive, a tone control, and a dry/wet mix.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The curve a distortion applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistortionKind {
    /// A smooth saturation, like a `tanh`. It rounds peaks and adds odd harmonics.
    #[default]
    Soft,
    /// A hard clip at full scale. It adds odd harmonics that fall off slowly.
    Hard,
    /// An asymmetric curve, like a tube stage. It adds even and odd harmonics.
    Tube,
    /// A wavefolder. Peaks past full scale fold back instead of clipping.
    Fold,
    /// Bit depth and sample rate reduction. Drive sets how coarse it gets.
    Crush,
}

impl DistortionKind {
    /// Every kind, in the order the dropdown lists them.
    pub const ALL: [DistortionKind; 5] = [
        DistortionKind::Soft,
        DistortionKind::Hard,
        DistortionKind::Tube,
        DistortionKind::Fold,
        DistortionKind::Crush,
    ];
}

/// The distortion's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistortionParams {
    pub kind: DistortionKind,
    /// How hard the input is pushed, from 0.0 to 1.0. At 0.0 the effect is nearly clean. At 1.0 it
    /// adds about 40 dB of gain before the curve. For `Crush` it sets the coarseness instead.
    pub drive: f32,
    /// A tilt on the output, from 0.0 (dark) to 1.0 (bright). 0.5 leaves the tone alone.
    pub tone: f32,
    /// The output level in dB, from -60.0 to 0.0, applied to the wet signal.
    pub level_db: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for DistortionParams {
    fn default() -> Self {
        Self {
            kind: DistortionKind::Soft,
            drive: 0.3,
            tone: 0.5,
            level_db: 0.0,
            mix: 1.0,
        }
    }
}

pub struct Distortion {
    sample_rate: f32,
}

impl Distortion {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &DistortionParams) -> Frame {
        input
    }
}
