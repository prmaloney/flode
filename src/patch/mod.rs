//! What the editor hands the audio thread: the patch, as plain data the engine can run.

pub mod control;
#[allow(dead_code)] // Used once the engine runs the stage arena.
pub mod stage;
