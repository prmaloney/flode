//! Pure signal processing. Nothing in here knows about nodes, wires, or the editor, so each
//! module can be read and tested on its own.

pub mod envelope;
pub mod mix;
pub mod noise;
pub mod pitch;
pub mod rng;
pub mod svf;
pub mod warp;
pub mod wavetable;
