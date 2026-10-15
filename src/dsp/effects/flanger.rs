//! Flanger: a very short modulated delay with feedback, mixed with the dry signal so the comb
//! filter sweeps.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The flanger's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangerParams {
    /// The speed of the sweep in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    pub rate_hz: f32,
    /// How far the delay swings around its base, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// The middle of the delay in ms, from 0.1 to 10.0.
    pub delay_ms: f32,
    /// How far apart the left and right sweeps sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for FlangerParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            feedback: 0.4,
            delay_ms: 2.0,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

pub struct Flanger {
    sample_rate: f32,
}

impl Flanger {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &FlangerParams) -> Frame {
        input
    }
}
