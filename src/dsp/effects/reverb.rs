//! Reverb: a basic stereo algorithmic reverb with a decay time, a room size, damping, and a
//! dry/wet mix.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The reverb's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    /// How long the tail takes to fall by 60 dB, in seconds, from 0.1 to 20.0.
    pub decay_s: f32,
    /// The size of the room, from 0.0 (small) to 1.0 (large). It scales the delay lengths.
    pub size: f32,
    /// How much high end the tail loses on each pass, from 0.0 (bright) to 1.0 (dark).
    pub damp: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            decay_s: 2.0,
            size: 0.5,
            damp: 0.5,
            mix: 0.3,
        }
    }
}

pub struct Reverb {
    sample_rate: f32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &ReverbParams) -> Frame {
        input
    }
}
