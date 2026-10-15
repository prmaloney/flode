//! Bell Lead. A starting stub, to be replaced.

use super::{Builder, SOURCES};
use crate::nodes::NodeKind;
use crate::presets::Preset;

pub(super) fn build() -> Preset {
    let mut b = Builder::new();
    let input = b.voice_input(true, 0.03);
    let osc = b.add(NodeKind::Oscillator, SOURCES);
    b.notes(input, osc);
    let output = b.output(-10.0);
    b.slot(osc, 0, output);
    b.finish("Bell Lead")
}
