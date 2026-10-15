//! The compressor node. The Multiband switch turns it into an OTT style three band compressor,
//! which shows its Depth knob.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Compressor;

impl NodeBehavior for Compressor {
    fn title(&self) -> &'static str {
        "Compressor"
    }

    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::Threshold),
            Row::driven_knob(KnobId::Ratio),
            Row::driven_knob(KnobId::CompAttack),
            Row::driven_knob(KnobId::CompRelease),
            Row::driven_knob(KnobId::Makeup),
            Row::driven_knob(KnobId::Mix),
        ];
        // Depth only means something in the multiband mode. It has no pin, so a saved wire never
        // depends on whether the row is showing.
        if multiband(node) {
            rows.push(Row::plain_knob(KnobId::Depth));
        }
        rows.push(Row::control(Control::Switch(0)));
        rows
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Multiband"]
    }

    /// A compressor starts fully wet, and the multiband mode starts at full depth.
    fn defaults(&self) -> &'static [(KnobId, f32)] {
        &[(KnobId::Mix, 1.0), (KnobId::Depth, 1.0)]
    }
}

/// True when the compressor splits the signal into bands.
pub fn multiband(node: &FlodeNode) -> bool {
    node.switch(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Compressor, Point::new(0.0, 0.0))
    }

    /// Depth shows only in the multiband mode, and never has a pin.
    #[test]
    fn depth_shows_only_in_the_multiband_mode() {
        let mut node = node();
        let depth = |node: &FlodeNode| {
            node.rows()
                .iter()
                .any(|row| row.knob == Some(KnobId::Depth))
        };
        assert!(!depth(&node));
        node.set_switch(0, true);
        assert!(multiband(&node));
        assert!(depth(&node));
        assert!(!node
            .input_ports()
            .contains(&crate::nodes::Port::Knob(KnobId::Depth)));
    }

    #[test]
    fn a_new_compressor_starts_wet_and_deep() {
        let node = node();
        assert_eq!(node.plain(KnobId::Mix), 1.0);
        assert_eq!(node.plain(KnobId::Depth), 1.0);
    }
}
