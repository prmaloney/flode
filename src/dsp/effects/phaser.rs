//! Phaser: a chain of all-pass filters whose frequency sweeps, summed with the dry signal so the
//! phase shift cuts notches.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The phaser's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaserParams {
    /// The speed of the sweep in Hz, from 0.05 to 200.0 (the knob is shared with the LFO).
    pub rate_hz: f32,
    /// How far the sweep reaches around the centre, from 0.0 to 1.0.
    pub depth: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// The middle of the sweep in Hz, from 100.0 to 8000.0.
    pub center_hz: f32,
    /// The number of all-pass stages: 4, 6, 8, or 12. Each pair makes one notch.
    pub stages: usize,
    /// How far apart the left and right sweeps sit, from 0.0 (identical) to 1.0 (opposite).
    pub spread: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
}

impl Default for PhaserParams {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            depth: 0.5,
            feedback: 0.4,
            center_hz: 1000.0,
            stages: 4,
            spread: 0.75,
            mix: 0.5,
        }
    }
}

pub struct Phaser {
    sample_rate: f32,
}

impl Phaser {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &PhaserParams) -> Frame {
        input
    }
}
