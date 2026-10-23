//! Control signals: where they come from, how they are mapped, and what they set. A control
//! route is the one place a wire from an LFO, an envelope, or a MIDI source meets a knob.

use crate::dsp::envelope::EnvPhase;
use crate::nodes::input::MidiSource;
use crate::nodes::input::MIDI_SOURCE_COUNT;
use crate::nodes::knobs::KnobId;
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::math::MathOp;

/// The most LFO nodes that drive knobs. Extra LFOs make no signal.
pub const MAX_LFOS: usize = 8;

/// The most note-emitting nodes a patch can run: MIDI inputs and arpeggiators together, counted in
/// graph order the same way `MAX_LFOS` and `MAX_ENVELOPES` are. A node past this many feeds nothing
/// downstream, the same as an LFO or envelope past its own limit.
pub const MAX_NOTE_SOURCES: usize = 8;

/// Where a control signal comes from. The indices are bytes, since every route carries a source
/// and a patch holds many routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The envelope at this index, in graph order. Each voice runs its own copy of it.
    Envelope(u8),
    /// The output of the LFO at this index, in graph order.
    Lfo(u8),
    /// A MIDI source on an input node.
    Midi(MidiSource),
    /// The macro knob at this index, which is a host parameter.
    Macro(u8),
    /// The output of the map or math node at this index in the patch's `SignalTable`. Such a node
    /// is a source like any other, so they chain to any depth, and the values of a map or the
    /// inputs of a math node can be driven through them.
    Signal(u8),
}

/// The most envelope nodes a patch can run. Each voice keeps a copy of every one, so the audio
/// thread can size them ahead of time.
pub const MAX_ENVELOPES: usize = 8;
/// One more than `MAX_ENVELOPES`: the bypass envelope that a slot without one plays through.
pub const ENV_SLOTS: usize = MAX_ENVELOPES + 1;

/// The most Sampler nodes a patch tracks a live playhead for, counted in graph order the same way
/// `MAX_ENVELOPES` and `MAX_LFOS` are. A Sampler past this many still plays; only its viewer's
/// cursor goes missing.
pub const MAX_SAMPLERS: usize = 8;

/// The most map and math nodes a patch can run, together. One past this many makes no signal.
pub const MAX_SIGNALS: usize = 24;

/// The shape of one map: where the input range is, where the output range is, and the offset that
/// pushes the result up or down. It is a plain curve, with no sources in it. The editor draws it
/// from the knob values it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapStage {
    pub in_lo: f32,
    pub in_hi: f32,
    pub out_lo: f32,
    pub out_hi: f32,
    /// From -1.0 to 1.0.
    pub offset: f32,
}

impl MapStage {
    /// Passes a value straight through.
    #[cfg(test)]
    pub const IDENTITY: MapStage = MapStage::fixed(0.0, 1.0, 0.0, 1.0);

    /// A map with no offset.
    pub const fn fixed(in_lo: f32, in_hi: f32, out_lo: f32, out_hi: f32) -> Self {
        Self {
            in_lo,
            in_hi,
            out_lo,
            out_hi,
            offset: 0.0,
        }
    }

    /// The same map with its offset set to `offset`, from -1.0 to 1.0.
    pub const fn with_offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// The output for the input `x`. It takes the input from `in_lo` to `in_hi` and sends it to
    /// the output range from `out_lo` to `out_hi`, then adds the offset. Inputs outside the input
    /// range stay at the nearest end, so the output is flat there. The result stays between 0.0
    /// and 1.0.
    pub fn apply(&self, x: f32) -> f32 {
        let span = self.in_hi - self.in_lo;
        // An empty input range is a step: below its point the output is the low end.
        let t = if span.abs() < 1e-6 {
            if x >= self.in_lo {
                1.0
            } else {
                0.0
            }
        } else {
            ((x - self.in_lo) / span).clamp(0.0, 1.0)
        };
        (self.out_lo + t * (self.out_hi - self.out_lo) + self.offset).clamp(0.0, 1.0)
    }
}

/// One value of a map: its own number, or the source that drives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapValue {
    /// The knob's value in its own units: a fraction from 0.0 to 1.0, or from -1.0 to 1.0 for the
    /// offset.
    pub value: f32,
    pub source: Option<Source>,
}

impl MapValue {
    /// A value that holds still.
    pub const fn fixed(value: f32) -> Self {
        Self {
            value,
            source: None,
        }
    }

    /// The value now. A source's level runs from 0.0 to 1.0, which a `signed` value stretches to
    /// -1.0 to 1.0, since a source has no sign.
    fn get(&self, signed: bool, levels: &ControlLevels) -> f32 {
        match self.source {
            None => self.value,
            Some(source) if signed => levels.level(source) * 2.0 - 1.0,
            Some(source) => levels.level(source),
        }
    }
}

/// A map as the audio thread runs it: the source it reads, and its values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapNode {
    /// The source wired to the map's input.
    pub input: Source,
    pub in_lo: MapValue,
    pub in_hi: MapValue,
    pub out_lo: MapValue,
    pub out_hi: MapValue,
    pub offset: MapValue,
    /// The output swing as a middle and a width, in place of `out_lo` and `out_hi`. The map's
    /// bipolar form sets it. The middle runs from 0.0 to 1.0, and the width from -1.0 to 1.0, where
    /// a negative width runs the swing backward. Half the width each side of the middle gives the
    /// two ends, which may land outside 0.0 to 1.0. The result is clamped at the end.
    pub center_range: Option<(MapValue, MapValue)>,
}

impl MapNode {
    /// The curve with every driven value read from `levels`.
    pub fn stage(&self, levels: &ControlLevels) -> MapStage {
        let (out_lo, out_hi) = match &self.center_range {
            Some((center, range)) => {
                let center = center.get(false, levels);
                let half = range.get(true, levels) / 2.0;
                (center - half, center + half)
            }
            None => (
                self.out_lo.get(false, levels),
                self.out_hi.get(false, levels),
            ),
        };
        MapStage {
            in_lo: self.in_lo.get(false, levels),
            in_hi: self.in_hi.get(false, levels),
            out_lo,
            out_hi,
            offset: self.offset.get(true, levels),
        }
    }

    /// The map's output now.
    pub fn level(&self, levels: &ControlLevels) -> f32 {
        self.stage(levels).apply(levels.level(self.input))
    }
}

/// A math node as the audio thread runs it: its operation and its two inputs. Each input is the
/// knob's own number, or the source that drives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathNode {
    pub op: MathOp,
    pub a: MapValue,
    pub b: MapValue,
}

impl MathNode {
    /// The node's output now.
    pub fn level(&self, levels: &ControlLevels) -> f32 {
        self.op
            .apply(self.a.get(false, levels), self.b.get(false, levels))
    }
}

/// A node that makes a signal out of other signals: a map or a math node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalNode {
    Map(MapNode),
    Math(MathNode),
}

impl SignalNode {
    /// The source wired to a map's input. A math node has no one input, so it gives None.
    #[cfg(test)]
    pub fn input(&self) -> Option<Source> {
        match self {
            SignalNode::Map(map) => Some(map.input),
            SignalNode::Math(_) => None,
        }
    }

    /// The node's output now.
    pub fn level(&self, levels: &ControlLevels) -> f32 {
        match self {
            SignalNode::Map(map) => map.level(levels),
            SignalNode::Math(math) => math.level(levels),
        }
    }
}

impl From<MapNode> for SignalNode {
    fn from(map: MapNode) -> Self {
        SignalNode::Map(map)
    }
}

impl From<MathNode> for SignalNode {
    fn from(math: MathNode) -> Self {
        SignalNode::Math(math)
    }
}

/// Every map and math node in the patch, in the order to run them: a node comes after every node
/// that feeds its input or drives one of its values. So each reads levels that are already worked
/// out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalTable {
    nodes: [Option<SignalNode>; MAX_SIGNALS],
}

impl SignalTable {
    /// A patch with no such nodes.
    pub const EMPTY: SignalTable = SignalTable {
        nodes: [None; MAX_SIGNALS],
    };

    /// The nodes, in running order.
    pub fn nodes(&self) -> impl Iterator<Item = &SignalNode> {
        self.nodes.iter().map_while(Option::as_ref)
    }

    /// Adds a node at the end, and returns its place. None if the table is full.
    pub fn push(&mut self, node: impl Into<SignalNode>) -> Option<usize> {
        let at = self.nodes.iter().position(Option::is_none)?;
        self.nodes[at] = Some(node.into());
        Some(at)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.nodes().count()
    }

    /// Works out every node's output into `levels.signals`. The sources in `levels` must be set
    /// first. A node reads only the ones before it, so this runs them in one pass.
    pub fn evaluate(&self, levels: &mut ControlLevels) {
        for (at, node) in self.nodes().enumerate() {
            levels.signals[at] = node.level(levels);
        }
    }
}

impl Default for SignalTable {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// A control route from a source to a knob. The knob follows the source's level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    pub source: Source,
}

impl Route {
    /// The knob's normalized value for the current levels.
    pub fn apply(&self, levels: &ControlLevels) -> f32 {
        levels.level(self.source).clamp(0.0, 1.0)
    }
}

/// The current level of every control source, from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlLevels {
    pub envelope: [f32; MAX_ENVELOPES],
    pub lfo: [f32; MAX_LFOS],
    pub midi: [f32; MIDI_SOURCE_COUNT],
    pub macros: [f32; MACRO_COUNT],
    /// The output of each map and math node in the patch's `SignalTable`, worked out from the
    /// sources above by `SignalTable::evaluate`. A voice works them out again from its own
    /// envelopes.
    pub signals: [f32; MAX_SIGNALS],
    /// The stage of the newest voice's envelope, or None while no voice runs it. The viewer on
    /// an envelope node shows it. No knob reads it.
    pub envelope_phase: [Option<EnvPhase>; MAX_ENVELOPES],
    /// Where each LFO is in its cycle, from 0.0 to 1.0. The viewer on an LFO node shows it.
    pub lfo_phase: [f32; MAX_LFOS],
    /// Where the newest voice of each Sampler is in its file, from 0.0 to 1.0, or None while no
    /// voice plays it. The viewer on a Sampler node shows it. No knob reads it.
    pub sampler_position: [Option<f32>; MAX_SAMPLERS],
}

impl ControlLevels {
    pub fn level(&self, source: Source) -> f32 {
        match source {
            Source::Envelope(index) => self.envelope[usize::from(index)],
            Source::Lfo(index) => self.lfo[usize::from(index)],
            Source::Midi(source) => self.midi[source.index()],
            Source::Macro(index) => self.macros[usize::from(index)],
            Source::Signal(index) => self.signals.get(usize::from(index)).copied().unwrap_or(0.0),
        }
    }
}

/// One knob as the audio thread sees it: the node's own value, and the control route that drives
/// it, if any.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobValue {
    /// The knob's own value, normalized from 0.0 to 1.0.
    pub value: f32,
    pub route: Option<Route>,
}

impl KnobValue {
    /// The knob's normalized value now. A driven knob follows its control source.
    pub fn normalized(&self, levels: &ControlLevels) -> f32 {
        match self.route {
            Some(route) => route.apply(levels),
            None => self.value,
        }
    }

    /// The knob's value in its own units.
    pub fn plain(&self, id: KnobId, levels: &ControlLevels) -> f32 {
        id.to_plain(self.normalized(levels))
    }
}

/// A knob a stage reads. It pairs the knob's id with the node's value and control route, so a
/// stage never repeats which knob it is reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    id: KnobId,
    knob: KnobValue,
}

impl Param {
    pub const fn new(id: KnobId, knob: KnobValue) -> Self {
        Self { id, knob }
    }

    /// A param at the knob's default value, with no control wire.
    pub fn default_of(id: KnobId) -> Self {
        Self::fixed(id, id.default_plain())
    }

    /// A param held at `plain`, in the knob's own units, with no control wire.
    pub fn fixed(id: KnobId, plain: f32) -> Self {
        Self::new(
            id,
            KnobValue {
                value: id.to_normalized(plain),
                route: None,
            },
        )
    }

    /// The value now, in the knob's own units. A driven knob follows its control source.
    pub fn plain(&self, levels: &ControlLevels) -> f32 {
        self.knob.plain(self.id, levels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_param_reads_back_its_value() {
        let levels = ControlLevels::default();
        assert_eq!(Param::fixed(KnobId::Octave, 1.0).plain(&levels), 1.0);
        assert!((Param::fixed(KnobId::Detune, 40.0).plain(&levels) - 40.0).abs() < 1e-4);
    }

    #[test]
    fn a_default_param_reads_the_knobs_default() {
        let levels = ControlLevels::default();
        let unison = Param::default_of(KnobId::Unison).plain(&levels);
        assert_eq!(unison, KnobId::Unison.default_plain());
    }
}
