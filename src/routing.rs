//! What the audio thread reads from the node graph. The editor publishes a snapshot on every
//! graph change, and the audio thread only loads from it. Everything is an atomic, so the audio
//! thread never locks or allocates. The audio thread also publishes the live control levels, so
//! the editor can show them.

use crate::dsp::noise::NoiseType;
use crate::dsp::warp::WarpMode;
use crate::dsp::wavetable::Waveform;
use crate::editor::graph::Graph;
use crate::nodes::filter::FilterType;
use crate::nodes::input::MIDI_SOURCE_COUNT;
use crate::nodes::knobs::KnobId;
use crate::nodes::mix::MixMode;
use crate::nodes::output::MAX_SLOTS;
use crate::patch::control::{
    ControlLevels, KnobValue, MapStage, Route, Source, MAX_ENVELOPES, MAX_LFOS, MAX_MAPS,
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

/// An envelope node's settings, as the audio thread reads them. Each voice's copy of the envelope
/// is set up from these once per block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeSettings {
    pub active: bool,
    pub attack: KnobValue,
    pub decay: KnobValue,
    pub sustain: KnobValue,
    pub release: KnobValue,
}

impl EnvelopeSettings {
    pub const OFF: EnvelopeSettings = EnvelopeSettings {
        active: false,
        attack: KnobValue::ZERO,
        decay: KnobValue::ZERO,
        sustain: KnobValue::ZERO,
        release: KnobValue::ZERO,
    };
}

/// The chain behind one output slot, with the knob values of the nodes in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaneChain {
    pub active: bool,
    pub waveform: Waveform,
    /// The noise type, when the waveform is noise.
    pub noise: NoiseType,
    /// The warp mode the oscillator applies.
    pub warp: WarpMode,
    /// The index of the envelope in the chain, if one is. Its voice copy shapes the amplitude.
    pub envelope: Option<usize>,
    pub filter: Option<FilterType>,
    /// Set when a mix node stands in for the single oscillator above. Its two branches replace
    /// `waveform`, `noise`, `warp`, `envelope`, and the oscillator knobs above.
    pub mix: Option<MixChain>,
    /// Indexed by `KnobId as usize`. Holds the knobs of the chain's nodes and the slot's gain.
    pub knobs: [KnobValue; KnobId::COUNT],
}

impl LaneChain {
    /// A slot with nothing wired in. It makes no sound.
    pub const SILENT: LaneChain = LaneChain {
        active: false,
        waveform: Waveform::Sine,
        noise: NoiseType::White,
        warp: WarpMode::None,
        envelope: None,
        filter: None,
        mix: None,
        knobs: [KnobValue::ZERO; KnobId::COUNT],
    };

    pub fn knob(&self, id: KnobId) -> KnobValue {
        self.knobs[id as usize]
    }
}

/// One of a mix node's two sources: an oscillator, with its own amplitude envelope and knobs. It
/// takes the place of a lane's single oscillator when a mix node sits in the chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Branch {
    /// False when nothing is wired to this input, so it contributes silence.
    pub active: bool,
    pub waveform: Waveform,
    pub noise: NoiseType,
    pub warp: WarpMode,
    pub envelope: Option<usize>,
    pub knobs: [KnobValue; KnobId::COUNT],
}

impl Branch {
    pub const SILENT: Branch = Branch {
        active: false,
        waveform: Waveform::Sine,
        noise: NoiseType::White,
        warp: WarpMode::None,
        envelope: None,
        knobs: [KnobValue::ZERO; KnobId::COUNT],
    };
}

/// A mix node's settings: how it combines its two branches, and the branches themselves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixChain {
    pub mode: MixMode,
    pub mix: KnobValue,
    pub a: Branch,
    pub b: Branch,
}

/// The output node's settings that apply to every slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Globals {
    pub mono: bool,
    /// Glide on every mono note, not only while another note is still held.
    pub glide_always: bool,
    pub gain: KnobValue,
    pub glide: KnobValue,
}

impl Globals {
    pub const SILENT: Globals = Globals {
        mono: false,
        glide_always: false,
        gain: KnobValue::ZERO,
        glide: KnobValue::ZERO,
    };
}

/// One LFO's settings. An LFO that isn't in the graph is inactive and makes no signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LfoSettings {
    pub active: bool,
    pub waveform: Waveform,
    /// The rate knob, in cycles per second. A control wire can drive it.
    pub rate: KnobValue,
    /// The Sync knob. Zero is free running, and each step past it is a note division.
    pub sync: KnobValue,
}

impl LfoSettings {
    pub const OFF: LfoSettings = LfoSettings {
        active: false,
        waveform: Waveform::Sine,
        rate: KnobValue::ZERO,
        sync: KnobValue::ZERO,
    };
}

/// A knob's published value and route, as atomics. A knob with no route stores zeros and
/// `driven` false.
struct KnobRoute {
    value: AtomicU32,
    source: AtomicU8,
    /// Each map stage's four values, in order.
    stages: [AtomicU32; MAX_MAPS * 4],
    count: AtomicU8,
    driven: AtomicBool,
}

impl KnobRoute {
    fn new() -> Self {
        Self {
            value: AtomicU32::new(0),
            source: AtomicU8::new(0),
            stages: std::array::from_fn(|_| AtomicU32::new(0)),
            count: AtomicU8::new(0),
            driven: AtomicBool::new(false),
        }
    }

    fn store(&self, knob: KnobValue) {
        self.value.store(knob.value.to_bits(), Ordering::Relaxed);
        let route = knob.route.unwrap_or(Route {
            source: Source::Envelope(0),
            stages: [MapStage::IDENTITY; MAX_MAPS],
            count: 0,
        });
        self.source.store(route.source.to_u8(), Ordering::Relaxed);
        for (i, stage) in route.stages.iter().enumerate() {
            let words = [stage.in_lo, stage.in_hi, stage.out_lo, stage.out_hi];
            for (j, word) in words.iter().enumerate() {
                self.stages[i * 4 + j].store(word.to_bits(), Ordering::Relaxed);
            }
        }
        self.count.store(route.count as u8, Ordering::Relaxed);
        self.driven.store(knob.route.is_some(), Ordering::Relaxed);
    }

    fn load(&self) -> KnobValue {
        let word = |i: usize| f32::from_bits(self.stages[i].load(Ordering::Relaxed));
        KnobValue {
            value: f32::from_bits(self.value.load(Ordering::Relaxed)),
            route: self.driven.load(Ordering::Relaxed).then(|| Route {
                source: Source::from_u8(self.source.load(Ordering::Relaxed)),
                stages: std::array::from_fn(|i| MapStage {
                    in_lo: word(i * 4),
                    in_hi: word(i * 4 + 1),
                    out_lo: word(i * 4 + 2),
                    out_hi: word(i * 4 + 3),
                }),
                count: self.count.load(Ordering::Relaxed) as usize,
            }),
        }
    }
}

/// One branch of a mix node, as atomics.
struct BranchRoute {
    active: AtomicBool,
    waveform: AtomicU8,
    noise: AtomicU8,
    warp: AtomicU8,
    /// The envelope index in the branch, plus one. Zero for none.
    envelope: AtomicU8,
    knobs: [KnobRoute; KnobId::COUNT],
}

impl BranchRoute {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            waveform: AtomicU8::new(Waveform::default().to_u8()),
            noise: AtomicU8::new(NoiseType::default().to_u8()),
            warp: AtomicU8::new(WarpMode::default().to_u8()),
            envelope: AtomicU8::new(0),
            knobs: std::array::from_fn(|_| KnobRoute::new()),
        }
    }

    fn store(&self, branch: &Branch) {
        self.active.store(branch.active, Ordering::Relaxed);
        self.waveform
            .store(branch.waveform.to_u8(), Ordering::Relaxed);
        self.noise.store(branch.noise.to_u8(), Ordering::Relaxed);
        self.warp.store(branch.warp.to_u8(), Ordering::Relaxed);
        self.envelope.store(
            branch.envelope.map_or(0, |index| index as u8 + 1),
            Ordering::Relaxed,
        );
        for (knob, value) in self.knobs.iter().zip(branch.knobs) {
            knob.store(value);
        }
    }

    fn load(&self) -> Branch {
        Branch {
            active: self.active.load(Ordering::Relaxed),
            waveform: Waveform::from_u8(self.waveform.load(Ordering::Relaxed)),
            noise: NoiseType::from_u8(self.noise.load(Ordering::Relaxed)),
            warp: WarpMode::from_u8(self.warp.load(Ordering::Relaxed)),
            envelope: match self.envelope.load(Ordering::Relaxed) {
                0 => None,
                n => Some(n as usize - 1),
            },
            knobs: std::array::from_fn(|i| self.knobs[i].load()),
        }
    }
}

/// A mix node's settings, as atomics.
struct MixRoute {
    active: AtomicBool,
    mode: AtomicU8,
    mix: KnobRoute,
    a: BranchRoute,
    b: BranchRoute,
}

impl MixRoute {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            mode: AtomicU8::new(0),
            mix: KnobRoute::new(),
            a: BranchRoute::new(),
            b: BranchRoute::new(),
        }
    }

    fn store(&self, mix: &MixChain) {
        self.active.store(true, Ordering::Relaxed);
        self.mode.store(mix.mode.to_u8(), Ordering::Relaxed);
        self.mix.store(mix.mix);
        self.a.store(&mix.a);
        self.b.store(&mix.b);
    }

    fn clear(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    fn load(&self) -> Option<MixChain> {
        self.active.load(Ordering::Relaxed).then(|| MixChain {
            mode: MixMode::from_u8(self.mode.load(Ordering::Relaxed)),
            mix: self.mix.load(),
            a: self.a.load(),
            b: self.b.load(),
        })
    }
}

/// One output slot's chain, as atomics the audio thread loads.
struct LaneRoute {
    active: AtomicBool,
    waveform: AtomicU8,
    noise: AtomicU8,
    warp: AtomicU8,
    /// The envelope index in the chain, plus one. Zero for none.
    envelope: AtomicU8,
    /// Zero for no filter. Otherwise one more than the filter type.
    filter: AtomicU8,
    mix: MixRoute,
    knobs: [KnobRoute; KnobId::COUNT],
}

/// One envelope's settings, as atomics.
struct EnvelopeRoute {
    active: AtomicBool,
    attack: KnobRoute,
    decay: KnobRoute,
    sustain: KnobRoute,
    release: KnobRoute,
}

impl EnvelopeRoute {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            attack: KnobRoute::new(),
            decay: KnobRoute::new(),
            sustain: KnobRoute::new(),
            release: KnobRoute::new(),
        }
    }
}

/// One LFO's settings, as atomics.
struct LfoRoute {
    active: AtomicBool,
    waveform: AtomicU8,
    rate: KnobRoute,
    sync: KnobRoute,
}

pub struct AudioRouting {
    lanes: [LaneRoute; MAX_SLOTS],
    lfos: [LfoRoute; MAX_LFOS],
    mono: AtomicBool,
    glide_always: AtomicBool,
    gain: KnobRoute,
    glide: KnobRoute,
    /// True while any knob is driven, so the editor knows to redraw each frame.
    any_driven: AtomicBool,
    /// The live control levels, published by the audio thread once per block.
    envelope_levels: [AtomicU32; MAX_ENVELOPES],
    envelopes: [EnvelopeRoute; MAX_ENVELOPES],
    lfo_levels: [AtomicU32; MAX_LFOS],
    midi_levels: [AtomicU32; MIDI_SOURCE_COUNT],
    /// The notes with a voice sounding, as two 64-bit words. The audio thread publishes it.
    active_notes: [AtomicU64; 2],
}

impl AudioRouting {
    pub fn new() -> Self {
        Self {
            lanes: std::array::from_fn(|_| LaneRoute {
                active: AtomicBool::new(false),
                waveform: AtomicU8::new(Waveform::default().to_u8()),
                noise: AtomicU8::new(NoiseType::default().to_u8()),
                warp: AtomicU8::new(WarpMode::default().to_u8()),
                envelope: AtomicU8::new(0),
                filter: AtomicU8::new(0),
                mix: MixRoute::new(),
                knobs: std::array::from_fn(|_| KnobRoute::new()),
            }),
            lfos: std::array::from_fn(|_| LfoRoute {
                active: AtomicBool::new(false),
                waveform: AtomicU8::new(0),
                rate: KnobRoute::new(),
                sync: KnobRoute::new(),
            }),
            mono: AtomicBool::new(false),
            glide_always: AtomicBool::new(false),
            gain: KnobRoute::new(),
            glide: KnobRoute::new(),
            any_driven: AtomicBool::new(false),
            envelope_levels: std::array::from_fn(|_| AtomicU32::new(0)),
            envelopes: std::array::from_fn(|_| EnvelopeRoute::new()),
            lfo_levels: std::array::from_fn(|_| AtomicU32::new(0)),
            midi_levels: std::array::from_fn(|_| AtomicU32::new(0)),
            active_notes: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub fn publish(&self, graph: &Graph) {
        for (route, chain) in self.lanes.iter().zip(graph.lane_chains()) {
            route.active.store(chain.active, Ordering::Relaxed);
            route
                .waveform
                .store(chain.waveform.to_u8(), Ordering::Relaxed);
            route.noise.store(chain.noise.to_u8(), Ordering::Relaxed);
            route.warp.store(chain.warp.to_u8(), Ordering::Relaxed);
            route.envelope.store(
                chain.envelope.map_or(0, |index| index as u8 + 1),
                Ordering::Relaxed,
            );
            route.filter.store(
                chain.filter.map_or(0, |filter| filter.to_u8() + 1),
                Ordering::Relaxed,
            );
            match &chain.mix {
                Some(mix) => route.mix.store(mix),
                None => route.mix.clear(),
            }
            for (knob, value) in route.knobs.iter().zip(chain.knobs) {
                knob.store(value);
            }
        }
        for (route, settings) in self.envelopes.iter().zip(graph.envelope_settings()) {
            route.active.store(settings.active, Ordering::Relaxed);
            route.attack.store(settings.attack);
            route.decay.store(settings.decay);
            route.sustain.store(settings.sustain);
            route.release.store(settings.release);
        }
        for (route, settings) in self.lfos.iter().zip(graph.lfo_settings()) {
            route.active.store(settings.active, Ordering::Relaxed);
            route
                .waveform
                .store(settings.waveform.to_u8(), Ordering::Relaxed);
            route.rate.store(settings.rate);
            route.sync.store(settings.sync);
        }
        let globals = graph.globals();
        self.mono.store(globals.mono, Ordering::Relaxed);
        self.glide_always
            .store(globals.glide_always, Ordering::Relaxed);
        self.gain.store(globals.gain);
        self.glide.store(globals.glide);
        self.any_driven.store(graph.any_driven(), Ordering::Relaxed);
    }

    /// The chain of one output slot, as the editor last published it.
    pub fn lane(&self, index: usize) -> LaneChain {
        let route = &self.lanes[index];
        let filter = match route.filter.load(Ordering::Relaxed) {
            0 => None,
            n => Some(FilterType::from_u8(n - 1)),
        };
        LaneChain {
            active: route.active.load(Ordering::Relaxed),
            waveform: Waveform::from_u8(route.waveform.load(Ordering::Relaxed)),
            noise: NoiseType::from_u8(route.noise.load(Ordering::Relaxed)),
            warp: WarpMode::from_u8(route.warp.load(Ordering::Relaxed)),
            envelope: match route.envelope.load(Ordering::Relaxed) {
                0 => None,
                n => Some(n as usize - 1),
            },
            filter,
            mix: route.mix.load(),
            knobs: std::array::from_fn(|i| route.knobs[i].load()),
        }
    }

    /// The settings of one envelope, as the editor last published them.
    pub fn envelope(&self, index: usize) -> EnvelopeSettings {
        let route = &self.envelopes[index];
        EnvelopeSettings {
            active: route.active.load(Ordering::Relaxed),
            attack: route.attack.load(),
            decay: route.decay.load(),
            sustain: route.sustain.load(),
            release: route.release.load(),
        }
    }

    /// The settings of one LFO, as the editor last published them.
    pub fn lfo(&self, index: usize) -> LfoSettings {
        let route = &self.lfos[index];
        LfoSettings {
            active: route.active.load(Ordering::Relaxed),
            waveform: Waveform::from_u8(route.waveform.load(Ordering::Relaxed)),
            rate: route.rate.load(),
            sync: route.sync.load(),
        }
    }

    pub fn globals(&self) -> Globals {
        Globals {
            mono: self.mono.load(Ordering::Relaxed),
            glide_always: self.glide_always.load(Ordering::Relaxed),
            gain: self.gain.load(),
            glide: self.glide.load(),
        }
    }

    /// True while any knob is driven by a control wire.
    pub fn any_driven(&self) -> bool {
        self.any_driven.load(Ordering::Relaxed)
    }

    /// Called by the audio thread once per block.
    pub fn set_levels(&self, levels: ControlLevels) {
        for (store, level) in self.envelope_levels.iter().zip(levels.envelope) {
            store.store(level.to_bits(), Ordering::Relaxed);
        }
        for (store, level) in self.lfo_levels.iter().zip(levels.lfo) {
            store.store(level.to_bits(), Ordering::Relaxed);
        }
        for (store, level) in self.midi_levels.iter().zip(levels.midi) {
            store.store(level.to_bits(), Ordering::Relaxed);
        }
    }

    /// Called by the audio thread once per block.
    pub fn set_active_notes(&self, notes: [u64; 2]) {
        for (store, word) in self.active_notes.iter().zip(notes) {
            store.store(word, Ordering::Relaxed);
        }
    }

    /// The notes with a voice sounding, as the audio thread last published them.
    pub fn active_notes(&self) -> [u64; 2] {
        std::array::from_fn(|i| self.active_notes[i].load(Ordering::Relaxed))
    }

    /// The control levels as the audio thread last published them.
    pub fn levels(&self) -> ControlLevels {
        ControlLevels {
            envelope: std::array::from_fn(|i| {
                f32::from_bits(self.envelope_levels[i].load(Ordering::Relaxed))
            }),
            lfo: std::array::from_fn(|i| {
                f32::from_bits(self.lfo_levels[i].load(Ordering::Relaxed))
            }),
            midi: std::array::from_fn(|i| {
                f32::from_bits(self.midi_levels[i].load(Ordering::Relaxed))
            }),
        }
    }
}
