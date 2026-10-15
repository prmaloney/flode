use flode::{Flode, STANDALONE};
use nih_plug::prelude::*;
use std::sync::atomic::Ordering;

fn main() {
    // Only the standalone app routes the computer keyboard to the synth.
    STANDALONE.store(true, Ordering::Relaxed);
    nih_export_standalone::<Flode>();
}
