//! Control signals: where they come from, how they are mapped, and what they set. A control
//! route is the one place a wire from an LFO, an envelope, or a MIDI source meets a knob.

use crate::nodes::input::{MidiSource, MIDI_SOURCE_COUNT};
use crate::nodes::knobs::KnobId;

/// The most LFO nodes that drive knobs. Extra LFOs make no signal.
pub const MAX_LFOS: usize = 2;

/// Where a control signal comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The envelope at this index, in graph order. Each voice runs its own copy of it.
    Envelope(usize),
    /// The output of the LFO at this index, in graph order.
    Lfo(usize),
    /// A MIDI source on an input node.
    Midi(MidiSource),
}

/// The most envelope nodes a patch can run. Each voice keeps a copy of every one, so the audio
/// thread can size them ahead of time.
pub const MAX_ENVELOPES: usize = 8;
/// One more than `MAX_ENVELOPES`: the bypass envelope that a slot without one plays through.
pub const ENV_SLOTS: usize = MAX_ENVELOPES + 1;

/// The most map nodes one control route can pass through.
pub const MAX_MAPS: usize = 4;

/// One map on a route. It takes the input from `in_lo` to `in_hi` and sends it to the output
/// range from `out_lo` to `out_hi`. Inputs outside the input range stay at the nearest end, so
/// the output is flat there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapStage {
    pub in_lo: f32,
    pub in_hi: f32,
    pub out_lo: f32,
    pub out_hi: f32,
}

impl MapStage {
    /// Passes a value straight through.
    pub const IDENTITY: MapStage = MapStage {
        in_lo: 0.0,
        in_hi: 1.0,
        out_lo: 0.0,
        out_hi: 1.0,
    };

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
        self.out_lo + t * (self.out_hi - self.out_lo)
    }
}

/// A control route from a source to a knob. The source's level passes through each map in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    pub source: Source,
    pub stages: [MapStage; MAX_MAPS],
    /// How many of `stages` are used.
    pub count: usize,
}

impl Route {
    /// The knob's normalized value for the current levels.
    pub fn apply(&self, levels: &ControlLevels) -> f32 {
        let mut x = levels.level(self.source);
        for stage in &self.stages[..self.count] {
            x = stage.apply(x);
        }
        x.clamp(0.0, 1.0)
    }
}

/// The current level of every control source, from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlLevels {
    pub envelope: [f32; MAX_ENVELOPES],
    pub lfo: [f32; MAX_LFOS],
    pub midi: [f32; MIDI_SOURCE_COUNT],
}

impl ControlLevels {
    pub fn level(&self, source: Source) -> f32 {
        match source {
            Source::Envelope(index) => self.envelope[index],
            Source::Lfo(index) => self.lfo[index],
            Source::Midi(source) => self.midi[source.index()],
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
