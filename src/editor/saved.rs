//! The graph as the project saves it, and the migrations that keep old saves loading.

use crate::dsp::svf::FilterType;
use crate::dsp::wavetable::Waveform;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{FlodeNode, NodeKind, Port, NODE_WIDTH};
use nih_plug_iced::Point;
use serde::{Deserialize, Serialize};

/// The version of the save format this build writes. A graph without one is from before
/// oscillators took their notes from a MIDI input node.
pub(super) const SAVE_VERSION: u32 = 1;

/// The node graph as saved with the project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedGraph {
    /// Which save format this is. Missing in the oldest saves, which read as 0.
    #[serde(default)]
    pub(super) version: u32,
    pub(super) nodes: Vec<SavedNode>,
    /// Audio connections as (output node index, input node index). Graphs saved before ports
    /// existed use this. New saves leave it empty.
    #[serde(default)]
    pub(super) connections: Vec<(usize, usize)>,
    #[serde(default)]
    pub(super) links: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedNode {
    pub(super) kind: NodeKind,
    pub(super) x: f32,
    pub(super) y: f32,
    /// Missing in graphs saved before collapsing existed. Those load expanded.
    #[serde(default)]
    pub(super) collapsed: bool,
    /// Missing in graphs saved before nodes could be switched off. Those load switched on.
    #[serde(default)]
    pub(super) bypassed: bool,
    /// Which sections of the node are folded, by section number. Missing in graphs saved before
    /// sections existed. Those load with each kind's own starting folds.
    #[serde(default)]
    pub(super) sections_collapsed: Vec<bool>,
    /// The dropdown rows, for oscillators, filters, and LFOs. Each one is a dropdown in order.
    #[serde(default)]
    pub(super) choices: Vec<usize>,
    /// Graphs saved before there were several dropdowns kept one, which loads as `choices[0]`.
    #[serde(default, skip_serializing)]
    pub(super) choice: usize,
    /// The switches, for the output node's mono and always-glide.
    #[serde(default)]
    pub(super) switches: Vec<bool>,
    /// Graphs saved before there were switches kept one, for mono. It loads into `switches`.
    #[serde(default, skip_serializing)]
    pub(super) switched: bool,
    /// The node's knob values, normalized. A knob that is missing keeps its default. Graphs saved
    /// before knobs lived on nodes have none, because those values were host parameters.
    #[serde(default)]
    pub(super) knobs: Vec<(KnobId, f32)>,
    /// Fields from before the dropdown row and the knobs moved onto every node. They are read
    /// when loading and never written.
    #[serde(default, skip_serializing)]
    pub(super) waveform: Option<Waveform>,
    #[serde(default, skip_serializing)]
    pub(super) filter_type: Option<FilterType>,
    #[serde(default, skip_serializing)]
    pub(super) out_min: Option<f32>,
    #[serde(default, skip_serializing)]
    pub(super) out_max: Option<f32>,
}

impl SavedNode {
    pub(super) fn new(node: &FlodeNode) -> Self {
        Self {
            kind: node.kind,
            x: node.position.x,
            y: node.position.y,
            collapsed: node.collapsed,
            bypassed: node.bypassed,
            sections_collapsed: node.sections_collapsed.clone(),
            choices: node.choices.to_vec(),
            choice: 0,
            switches: node.switches.to_vec(),
            switched: false,
            knobs: KnobId::ALL.iter().map(|&id| (id, node.value(id))).collect(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
        }
    }

    pub(super) fn into_node(&self) -> FlodeNode {
        let mut node = FlodeNode::new(self.kind, Point::new(self.x, self.y));
        node.collapsed = self.collapsed;
        node.bypassed = self.bypassed;
        for (section, &collapsed) in self.sections_collapsed.iter().enumerate() {
            if section < node.sections_collapsed.len() {
                node.sections_collapsed[section] = collapsed;
            }
        }
        node.choices = self.choices.clone();
        if self.choices.is_empty() {
            node.choices.push(self.choice);
        }
        node.switches = self.switches.clone();
        if self.switched {
            node.set_switch(0, true);
        }
        match self.kind {
            NodeKind::Oscillator => {
                if let Some(waveform) = self.waveform {
                    if node.choices.is_empty() {
                        node.choices.push(0);
                    }
                    node.choices[0] = waveform.index();
                }
            }
            NodeKind::Filter => {
                if let Some(filter_type) = self.filter_type {
                    if node.choices.is_empty() {
                        node.choices.push(0);
                    }
                    node.choices[0] = filter_type.index();
                }
            }
            NodeKind::Map => {
                if let Some(min) = self.out_min {
                    node.set_value(KnobId::MapMin, KnobId::MapMin.to_normalized(min));
                }
                if let Some(max) = self.out_max {
                    node.set_value(KnobId::MapMax, KnobId::MapMax.to_normalized(max));
                }
            }
            _ => {}
        }
        for &(id, value) in &self.knobs {
            node.set_value(id, value);
        }
        node
    }
}

/// Brings a save from before the MIDI input node up to date. Every oscillator is wired to a MIDI
/// input, which is the first one in the graph or a new one left of the other nodes. That input
/// takes the glide and mono settings the output node used to hold, so the patch sounds the same.
pub(super) fn migrate_notes(nodes: &mut Vec<FlodeNode>, links: &mut Vec<Connection>) {
    let oscillators: Vec<usize> = (0..nodes.len())
        .filter(|&index| nodes[index].kind == NodeKind::Oscillator)
        .collect();
    if oscillators.is_empty() {
        return;
    }
    let input = match nodes.iter().position(|n| n.kind == NodeKind::Input) {
        Some(index) => index,
        None => {
            let left = nodes
                .iter()
                .map(|n| n.position.x)
                .fold(f32::INFINITY, f32::min);
            let top = nodes
                .iter()
                .map(|n| n.position.y)
                .fold(f32::INFINITY, f32::min);
            let x = (left - NODE_WIDTH - 48.0).max(0.0);
            nodes.push(FlodeNode::new(NodeKind::Input, Point::new(x, top)));
            nodes.len() - 1
        }
    };
    if let Some(output) = nodes.iter().position(|n| n.kind == NodeKind::Output) {
        let glide = nodes[output].value(KnobId::Glide);
        let (mono, always) = (nodes[output].switch(0), nodes[output].switch(1));
        nodes[input].set_value(KnobId::Glide, glide);
        nodes[input].set_switch(0, mono);
        nodes[input].set_switch(1, always);
        nodes[output].switches.clear();
    }
    for oscillator in oscillators {
        if !links
            .iter()
            .any(|c| c.to == oscillator && c.input == Port::Notes)
        {
            links.push(Connection {
                from: input,
                out: Port::Notes,
                to: oscillator,
                input: Port::Notes,
            });
        }
    }
}

/// Rewires old saves whose envelopes sat in the sound chain. The audio goes straight from the
/// envelope's input to its output, and the envelope drives the oscillator's gain instead, so the
/// sound stays the same.
pub(super) fn migrate_envelope_audio(
    nodes: &[FlodeNode],
    links: Vec<Connection>,
) -> Vec<Connection> {
    let is_envelope = |index: usize| {
        nodes
            .get(index)
            .is_some_and(|n| n.kind == NodeKind::Envelope)
    };
    let mut result = Vec::new();
    let mut audio = Vec::new();
    for link in links {
        let touches_envelope_audio = (is_envelope(link.to) && link.input.is_audio())
            || (is_envelope(link.from) && link.out == Port::Audio);
        if touches_envelope_audio {
            audio.push(link);
        } else {
            result.push(link);
        }
    }
    for envelope in (0..nodes.len()).filter(|&index| is_envelope(index)) {
        let Some(input) = audio
            .iter()
            .find(|c| c.to == envelope && c.input.is_audio())
            .copied()
        else {
            continue;
        };
        for output in audio
            .iter()
            .filter(|c| c.from == envelope && c.out == Port::Audio)
        {
            result.push(Connection {
                from: input.from,
                out: input.out,
                to: output.to,
                input: output.input,
            });
        }
        // The oscillator's envelope input is the amplitude, so the envelope drives that.
        let amp = Port::Shape;
        if nodes[input.from].kind == NodeKind::Oscillator
            && !result.iter().any(|c| c.to == input.from && c.input == amp)
        {
            result.push(Connection {
                from: envelope,
                out: Port::Shape,
                to: input.from,
                input: amp,
            });
        }
    }
    result
}
