//! Chorus: several short modulated delays mixed with the dry signal, spread across the stereo
//! field.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The chorus's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChorusParams {
    /// The speed of the modulation in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    /// Values above about 10 Hz are an extreme effect, not a musical one, and must stay stable.
    pub rate_hz: f32,
    /// How far the delay swings, from 0.0 to 1.0.
    pub depth: f32,
    /// How far apart the left and right modulation sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

pub struct Chorus {
    sample_rate: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &ChorusParams) -> Frame {
        input
    }
}
