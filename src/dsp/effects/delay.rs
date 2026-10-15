//! Delay: a stereo delay with feedback, damping in the feedback path, and a ping pong mode that
//! bounces the repeats between the left and right.
//!
//! This is a stub. It passes the input through until the real effect replaces it.
#![allow(dead_code)]

use crate::dsp::frame::Frame;

/// The delay's controls, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayParams {
    /// The time between repeats in ms, from 1.0 to 2000.0. The caller has already turned a synced
    /// note division into milliseconds, so this is always the time to use.
    pub time_ms: f32,
    /// How much of the output goes back in, from 0.0 to 0.95.
    pub feedback: f32,
    /// How much high end each repeat loses, from 0.0 (bright) to 1.0 (dark).
    pub damp: f32,
    /// Dry/wet, from 0.0 (all dry) to 1.0 (all wet).
    pub mix: f32,
    /// True to send each repeat to the opposite side from the last one.
    pub ping_pong: bool,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            time_ms: 350.0,
            feedback: 0.4,
            damp: 0.5,
            mix: 0.3,
            ping_pong: false,
        }
    }
}

pub struct Delay {
    sample_rate: f32,
}

impl Delay {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    pub fn reset(&mut self) {}

    pub fn process(&mut self, input: Frame, _params: &DelayParams) -> Frame {
        input
    }
}
