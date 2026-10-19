//! Effects. Each one takes a stereo frame at a time and returns a stereo frame. An effect keeps
//! its own memory, so one instance serves one place in the signal chain.
//!
//! Every effect module follows one shape:
//!
//! - `<Name>Params`: the plain-valued controls, already read from the knobs for this sample.
//! - `<Name>::new(sample_rate)`: builds the effect. It may allocate, since it runs before the audio
//!   thread does.
//! - `<Name>::reset()`: forgets the tail and any filter memory.
//! - `<Name>::process(input, &params) -> Frame`: one sample. It must not allocate or block.
//!
//! Params change once per audio block when a knob is moved by the host, and every sample when a
//! control wire drives a knob. An effect smooths the params that would click if they jumped.

pub mod chorus;
pub mod compressor;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod flanger;
pub mod phaser;
pub mod reverb;
