//! The map node. It rescales a control signal. The input range says which part of the incoming
//! signal to read, and the output range says where that part lands. Inputs outside the input range
//! hold at the nearest end. The curve on the node shows the mapping and where the live signal is.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::routing::MapStage;

pub struct Map;

impl NodeBehavior for Map {
    fn title(&self) -> &'static str {
        "Map"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::input(Port::Shape).with_output(Port::Shape),
            Row::control(Control::Curve),
        ]
    }
}

/// The map's stage: its input range and its output range, from the node's four values.
pub fn stage(node: &FlodeNode) -> MapStage {
    MapStage {
        in_lo: node.plain(KnobId::MapInMin),
        in_hi: node.plain(KnobId::MapInMax),
        out_lo: node.plain(KnobId::MapMin),
        out_hi: node.plain(KnobId::MapMax),
    }
}
