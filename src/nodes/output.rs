//! The output node. Each connected slot plays its own chain and has its own gain, and an empty
//! slot pin sits below the last used one. The master gain applies to the whole output.

use super::{FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Output;

/// The most output slots. Each connected slot plays its own chain and has its own gain.
pub const MAX_SLOTS: usize = 4;

/// The gain knob of each output slot.
const SLOT_KNOBS: [KnobId; MAX_SLOTS] =
    [KnobId::Slot1, KnobId::Slot2, KnobId::Slot3, KnobId::Slot4];

/// The gain knob of an output slot.
pub fn slot_knob(slot: usize) -> KnobId {
    SLOT_KNOBS[slot]
}

impl NodeBehavior for Output {
    fn title(&self) -> &'static str {
        "Output"
    }

    /// A pin for each shown slot, with a gain knob beside each connected one. The master gain is
    /// last.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let mut rows = Vec::new();
        for slot in 0..node.slots {
            let mut row = Row::input(Port::Slot(slot));
            if slot < node.used {
                row.knob = Some(slot_knob(slot));
            }
            rows.push(row);
        }
        rows.push(Row::driven_knob(KnobId::Gain));
        rows
    }

    fn audio_in(&self) -> bool {
        true
    }
}
