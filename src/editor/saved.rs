//! The graph as the project saves it, and the migrations that keep old saves loading.

use crate::dsp::curve::Curve;
use crate::dsp::svf::FilterType;
use crate::dsp::wavetable::Waveform;
use crate::nodes::connection::Connection;
use crate::nodes::group::{GroupData, GroupPin};
use crate::nodes::knobs::KnobId;
use crate::nodes::{FlodeNode, NodeKind, Port, NODE_WIDTH};
use nih_plug_iced::Point;
use serde::{Deserialize, Serialize};

/// The version of the save format this build writes. A graph without one is from before
/// oscillators took their notes from a MIDI input node. Version 3 added groups. A save from before
/// it has none, and loads the same way.
pub(super) const SAVE_VERSION: u32 = 3;

/// The first version whose oscillators take their notes from a MIDI input node.
pub(super) const NOTES_VERSION: u32 = 1;

/// Saves from before this version have an Octave knob that spans -2 to 2. It spans -4 to 4 now.
pub(super) const OCTAVE_SPAN_VERSION: u32 = 2;

/// Rescales the Octave knob of a save from before it spanned -4 to 4, so each oscillator keeps
/// its pitch. The knob is stored from 0 to 1, so the old position moves toward the middle.
pub(super) fn migrate_octave_span(nodes: &mut [FlodeNode]) {
    for node in nodes {
        let old = node.value(KnobId::Octave);
        node.set_value(KnobId::Octave, 0.5 + (old - 0.5) / 2.0);
    }
}

/// The node graph as saved with the project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedGraph {
    /// Which save format this is. Missing in the oldest saves, which read as 0.
    #[serde(default)]
    pub(super) version: u32,
    /// The preset this graph was last saved as or loaded from, if any. Shown on the Presets row.
    #[serde(default)]
    pub(super) preset_name: Option<String>,
    /// The category of that preset. Empty if it has none.
    #[serde(default)]
    pub(super) preset_category: String,
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
    /// The points of an LFO's custom wave. Empty means the default curve, which is what saves from
    /// before the wave existed have.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) points: Vec<[f32; 2]>,
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
    /// A group node's graph. Missing on every other kind of node, and in saves from before groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) group: Option<SavedGroup>,
    /// The pins that take a signal, on a group node or a group's output edge node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) inputs: Vec<GroupPin>,
    /// The pins that send a signal, on a group node or a group's input edge node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) outputs: Vec<GroupPin>,
    /// The names given to a macro node's four knobs. Empty when none was renamed, and in saves
    /// from before macros could be renamed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) names: Vec<String>,
}

/// The graph a group node holds, as saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedGroup {
    pub(super) name: String,
    pub(super) nodes: Vec<SavedNode>,
    pub(super) links: Vec<Connection>,
}

/// The version of the group file format this build writes. A file with a higher one is from a
/// newer build and is refused.
pub const GROUP_FILE_VERSION: u32 = 1;

/// A group saved on its own, to share between patches. It holds the group node as the patch
/// saves it, so the graph inside, the pins, and the face knob values all come back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedGroupFile {
    #[serde(default)]
    version: u32,
    /// The group's name. The file's own name wins when the file loads.
    pub name: String,
    node: SavedNode,
}

impl SavedGroupFile {
    /// The file for group node `node`, saved under `name`. The node's place, fold, and bypass are
    /// left out, since the copy that loads starts fresh.
    pub fn new(node: &FlodeNode, name: &str) -> Self {
        let mut saved = SavedNode::new(node);
        saved.x = 0.0;
        saved.y = 0.0;
        saved.collapsed = false;
        saved.bypassed = false;
        if let Some(group) = saved.group.as_mut() {
            group.name = name.to_string();
        }
        Self {
            version: GROUP_FILE_VERSION,
            name: name.to_string(),
            node: saved,
        }
    }

    /// Sets the group's name, in the file and in the group.
    pub fn rename(&mut self, name: &str) {
        self.name = name.to_string();
        if let Some(group) = self.node.group.as_mut() {
            group.name = name.to_string();
        }
    }

    /// Builds the group node the file holds, or says why it can't. A file is refused if it is
    /// from a newer build, does not hold a group, holds an Output or Macro node at any depth, or
    /// has a wire that names a pin or node that is not there.
    pub fn into_node(&self) -> Result<FlodeNode, String> {
        if self.version > GROUP_FILE_VERSION {
            return Err("it was saved by a newer Flode".to_string());
        }
        let saved = match (&self.node.kind, &self.node.group) {
            (NodeKind::Group, Some(saved)) => saved,
            _ => return Err("it does not hold a group".to_string()),
        };
        let node = self.node.into_node();
        let data = node
            .group
            .as_ref()
            .ok_or_else(|| "it does not hold a group".to_string())?;
        check_group(saved, data)?;
        Ok(node)
    }
}

/// Checks a group as loaded against what the file said. The load drops a wire that has no pin to
/// end on, so a count that differs means the file was broken.
fn check_group(saved: &SavedGroup, data: &GroupData) -> Result<(), String> {
    if saved.links.len() != data.links.len() {
        return Err(format!("{} has a wire with no pin", data.name));
    }
    if data.find(NodeKind::GroupInput).is_none() || data.find(NodeKind::GroupOutput).is_none() {
        return Err(format!("{} is missing its input or output node", data.name));
    }
    for (index, node) in data.nodes.iter().enumerate() {
        match node.kind {
            NodeKind::Output | NodeKind::Macro => {
                return Err(format!("{} cannot hold a {} node", data.name, node.title()));
            }
            NodeKind::Group => {
                let inner = node.group.as_ref();
                let saved_inner = saved.nodes.get(index).and_then(|n| n.group.as_ref());
                match (saved_inner, inner) {
                    (Some(saved_inner), Some(inner)) => check_group(saved_inner, inner)?,
                    _ => return Err(format!("{} has a group with no graph", data.name)),
                }
            }
            _ => {}
        }
    }
    Ok(())
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
            // A group node's knob ids only name its face knobs, whose values the pins save.
            knobs: if node.kind == NodeKind::Group {
                Vec::new()
            } else {
                KnobId::ALL.iter().map(|&id| (id, node.value(id))).collect()
            },
            points: node.points.clone(),
            waveform: None,
            filter_type: None,
            out_min: None,
            out_max: None,
            group: node.group.as_ref().map(|group| SavedGroup {
                name: group.name.clone(),
                nodes: group.nodes.iter().map(SavedNode::new).collect(),
                links: group.links.clone(),
            }),
            inputs: node.inputs.clone(),
            outputs: node.outputs.clone(),
            names: node.names.clone(),
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
        // Points that do not make a valid curve load as the default.
        if Curve::from_points(&self.points).is_some() {
            node.points = self.points.clone();
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
        node.inputs = self.inputs.clone();
        node.outputs = self.outputs.clone();
        if self.kind == NodeKind::Macro {
            for (index, name) in self.names.iter().enumerate() {
                node.set_macro_name(index, name);
            }
        }
        if self.kind == NodeKind::Group {
            if let Some(saved) = &self.group {
                // The graph inside loads like any other, so a group in a group does too.
                let nodes: Vec<FlodeNode> = saved.nodes.iter().map(SavedNode::into_node).collect();
                let links = saved
                    .links
                    .iter()
                    .copied()
                    .filter(|link| link.is_valid(&nodes))
                    .collect();
                let data = GroupData {
                    name: saved.name.clone(),
                    nodes,
                    links,
                };
                node.group = Some(Box::new(data));
                // The face pins are a copy of the edge nodes' pins, so the edge nodes decide the
                // names and signals. The saved face keeps the knob values, and a save from before
                // face knobs takes them from the knobs inside.
                super::group::sync_face(&mut node);
            }
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
