//! Compressor: a stereo-linked feed-forward compressor, with a multiband mode that gives the
//! upward and downward squash of an OTT.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The compressor's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorParams {
    /// The level above which the signal is turned down, in dB, from -60.0 to 0.0.
    pub threshold_db: f32,
    /// How much the signal above the threshold is turned down, from 1.0 (none) to 20.0.
    pub ratio: f32,
    /// How fast the gain falls when the signal rises past the threshold, in ms, from 0.1 to 100.0.
    pub attack_ms: f32,
    /// How fast the gain comes back, in ms, from 10.0 to 1000.0.
    pub release_ms: f32,
    /// Gain added after compression, in dB, from 0.0 to 24.0.
    pub makeup_db: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet). A mix between them is parallel compression.
    pub mix: f32,
    /// True for the multiband mode.
    pub multiband: bool,
    /// Multiband only. How hard the upward and downward compression work, from 0.0 to 1.0.
    pub depth: f32,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 0.0,
            mix: 1.0,
            multiband: false,
            depth: 1.0,
        }
    }
}

pub struct Compressor {
    sample_rate: f32,
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &CompressorParams) -> Frame {
        input
    }
}
