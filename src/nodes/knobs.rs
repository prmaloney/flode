//! Knobs. A knob is one adjustable value on a node. Every node owns its knob values, so two nodes
//! of the same kind do not share them. This module says what each knob is called, what range it
//! covers, and how its value reads. It does not hold any values.

use super::lfo::{self, DIVISION_COUNT};
use nih_plug::prelude::FloatRange;
use serde::{Deserialize, Serialize};

/// A gain knob's minimum, in dB. At this value the gain is exactly silent.
pub const GAIN_MIN_DB: f32 = crate::dsp::gain::SILENT_DB;

/// The most unison voices an oscillator can run.
pub const MAX_UNISON: usize = 7;

/// The kinds of knob. A node uses the ones its rows list. Wires name a knob by this id, so the
/// names are part of the saved graph format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnobId {
    Gain,
    Attack,
    Decay,
    Sustain,
    Release,
    Cutoff,
    Resonance,
    Unison,
    Detune,
    PhaseRandom,
    Glide,
    Slot1,
    Slot2,
    Slot3,
    Slot4,
    /// The map's output range, the low and high end.
    MapMin,
    MapMax,
    /// The map's input range, the low and high end.
    MapInMin,
    MapInMax,
    /// A map's offset, from -1 to 1. It pushes the whole curve up or down.
    MapOffset,
    /// A map's output swing in its bipolar form: the middle of the swing, from 0 to 1, and how wide
    /// it is, from -1 to 1. A negative width runs the swing backward, so the map inverts.
    MapCenter,
    MapRange,
    /// The two inputs of a math node, from 0 to 1.
    MathA,
    MathB,
    LfoRate,
    LfoSync,
    /// How strongly an oscillator's warp bends its waveform, from 0 to 1.
    WarpAmount,
    /// An oscillator's own gain in dB, before its output slot's gain and the filter.
    OscGain,
    /// An oscillator's coarse transpose in semitones, from -12 to 12.
    Coarse,
    /// An oscillator's fine transpose in cents, from -100 to 100.
    Fine,
    /// An oscillator's transpose in octaves, from -4 to 4.
    Octave,
    /// A mix node's blend, from 0.0 (just signal A) to 1.0 (fully the chosen combination).
    Mix,
    /// An oscillator's place in the stereo field, from -1 (left) to 1 (right).
    Pan,
    /// How far an oscillator's unison copies spread across the stereo field, from 0 to 1. Effects
    /// that move the left and right apart share it.
    Spread,
    /// How hard a distortion pushes its input, from 0 to 1.
    Drive,
    /// A tilt from dark (0) to bright (1), for a distortion's output.
    Tone,
    /// A compressor's threshold in dB.
    Threshold,
    /// A compressor's ratio, from 1 to 20.
    Ratio,
    /// A compressor's attack in milliseconds.
    CompAttack,
    /// A compressor's release in milliseconds.
    CompRelease,
    /// A compressor's makeup gain in dB.
    Makeup,
    /// How far a modulation effect sweeps, or how hard the multiband compressor works, from 0 to 1.
    Depth,
    /// A reverb's decay time in seconds.
    ReverbDecay,
    /// A reverb's room size, from 0 to 1.
    Size,
    /// How much high end a reverb or delay tail loses, from 0 to 1.
    Damp,
    /// A delay's time in milliseconds.
    DelayTime,
    /// How much of an effect's output goes back into its input, from 0 to 0.95.
    Feedback,
    /// A phaser's centre frequency in Hz.
    Center,
    /// A flanger's base delay in milliseconds.
    FlangerDelay,
    /// An equalizer band's frequency in Hz. Band 1 is the lowest. The graph on the node sets it.
    EqFreq1,
    EqFreq2,
    EqFreq3,
    EqFreq4,
    /// An equalizer band's gain in dB. The graph on the node sets it.
    EqGain1,
    EqGain2,
    EqGain3,
    EqGain4,
    /// An equalizer band's Q.
    EqQ1,
    EqQ2,
    EqQ3,
    EqQ4,
    /// The four macro knobs, from 0 to 1. Each one is a host parameter, so the DAW can automate it.
    Macro1,
    Macro2,
    Macro3,
    Macro4,
}

/// Envelope times in seconds. Skewed so most of the knob travel covers short times.
fn time_range() -> FloatRange {
    FloatRange::Skewed {
        min: 0.001,
        max: 5.0,
        factor: FloatRange::skew_factor(-2.0),
    }
}

impl KnobId {
    /// Number of knob kinds, for sizing per-knob arrays.
    pub const COUNT: usize = 65;

    /// The macro knobs, in order. The first is macro 1.
    pub const MACROS: [KnobId; 4] = [
        KnobId::Macro1,
        KnobId::Macro2,
        KnobId::Macro3,
        KnobId::Macro4,
    ];

    /// Which macro this knob is, from 0, or None for any other knob.
    pub fn macro_index(self) -> Option<usize> {
        Self::MACROS.iter().position(|&id| id == self)
    }

    pub const ALL: [KnobId; KnobId::COUNT] = [
        KnobId::Gain,
        KnobId::Attack,
        KnobId::Decay,
        KnobId::Sustain,
        KnobId::Release,
        KnobId::Cutoff,
        KnobId::Resonance,
        KnobId::Unison,
        KnobId::Detune,
        KnobId::PhaseRandom,
        KnobId::Glide,
        KnobId::Slot1,
        KnobId::Slot2,
        KnobId::Slot3,
        KnobId::Slot4,
        KnobId::MapMin,
        KnobId::MapMax,
        KnobId::MapInMin,
        KnobId::MapInMax,
        KnobId::MapOffset,
        KnobId::MapCenter,
        KnobId::MapRange,
        KnobId::MathA,
        KnobId::MathB,
        KnobId::LfoRate,
        KnobId::LfoSync,
        KnobId::WarpAmount,
        KnobId::OscGain,
        KnobId::Coarse,
        KnobId::Fine,
        KnobId::Octave,
        KnobId::Mix,
        KnobId::Pan,
        KnobId::Spread,
        KnobId::Drive,
        KnobId::Tone,
        KnobId::Threshold,
        KnobId::Ratio,
        KnobId::CompAttack,
        KnobId::CompRelease,
        KnobId::Makeup,
        KnobId::Depth,
        KnobId::ReverbDecay,
        KnobId::Size,
        KnobId::Damp,
        KnobId::DelayTime,
        KnobId::Feedback,
        KnobId::Center,
        KnobId::FlangerDelay,
        KnobId::EqFreq1,
        KnobId::EqFreq2,
        KnobId::EqFreq3,
        KnobId::EqFreq4,
        KnobId::EqGain1,
        KnobId::EqGain2,
        KnobId::EqGain3,
        KnobId::EqGain4,
        KnobId::EqQ1,
        KnobId::EqQ2,
        KnobId::EqQ3,
        KnobId::EqQ4,
        KnobId::Macro1,
        KnobId::Macro2,
        KnobId::Macro3,
        KnobId::Macro4,
    ];

    /// The name shown beside the knob.
    pub fn name(self) -> &'static str {
        match self {
            KnobId::Gain => "Gain",
            KnobId::Attack => "Attack",
            KnobId::Decay => "Decay",
            KnobId::Sustain => "Sustain",
            KnobId::Release => "Release",
            KnobId::Cutoff => "Cutoff",
            KnobId::Resonance => "Resonance",
            KnobId::Unison => "Unison",
            KnobId::Detune => "Detune",
            KnobId::PhaseRandom => "Phase",
            KnobId::Glide => "Glide",
            KnobId::Slot1 => "Slot 1",
            KnobId::Slot2 => "Slot 2",
            KnobId::Slot3 => "Slot 3",
            KnobId::Slot4 => "Slot 4",
            KnobId::MapMin => "Out min",
            KnobId::MapMax => "Out max",
            KnobId::MapInMin => "In min",
            KnobId::MapInMax => "In max",
            KnobId::MapOffset => "Offset",
            KnobId::MapCenter => "Center",
            KnobId::MapRange => "Range",
            KnobId::MathA => "A",
            KnobId::MathB => "B",
            KnobId::LfoRate => "Rate",
            KnobId::LfoSync => "Sync",
            KnobId::WarpAmount => "Warp",
            KnobId::OscGain => "Level",
            KnobId::Coarse => "Crs",
            KnobId::Fine => "Fin",
            KnobId::Octave => "Oct",
            KnobId::Mix => "Mix",
            KnobId::Pan => "Pan",
            KnobId::Spread => "Spread",
            KnobId::Drive => "Drive",
            KnobId::Tone => "Tone",
            KnobId::Threshold => "Thresh",
            KnobId::Ratio => "Ratio",
            KnobId::CompAttack => "Attack",
            KnobId::CompRelease => "Release",
            KnobId::Makeup => "Makeup",
            KnobId::Depth => "Depth",
            KnobId::ReverbDecay => "Decay",
            KnobId::Size => "Size",
            KnobId::Damp => "Damp",
            KnobId::DelayTime => "Time",
            KnobId::Feedback => "Feedback",
            KnobId::Center => "Freq",
            KnobId::FlangerDelay => "Delay",
            KnobId::EqFreq1 => "Freq 1",
            KnobId::EqFreq2 => "Freq 2",
            KnobId::EqFreq3 => "Freq 3",
            KnobId::EqFreq4 => "Freq 4",
            KnobId::EqGain1 => "Gain 1",
            KnobId::EqGain2 => "Gain 2",
            KnobId::EqGain3 => "Gain 3",
            KnobId::EqGain4 => "Gain 4",
            KnobId::EqQ1 => "Q 1",
            KnobId::EqQ2 => "Q 2",
            KnobId::EqQ3 => "Q 3",
            KnobId::EqQ4 => "Q 4",
            KnobId::Macro1 => "Macro 1",
            KnobId::Macro2 => "Macro 2",
            KnobId::Macro3 => "Macro 3",
            KnobId::Macro4 => "Macro 4",
        }
    }

    fn range(self) -> FloatRange {
        match self {
            KnobId::Gain
            | KnobId::Slot1
            | KnobId::Slot2
            | KnobId::Slot3
            | KnobId::Slot4
            | KnobId::OscGain => FloatRange::Linear {
                min: GAIN_MIN_DB,
                max: 0.0,
            },
            KnobId::Attack | KnobId::Decay | KnobId::Release => time_range(),
            KnobId::Glide => FloatRange::Skewed {
                min: 0.0,
                max: 2.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::Sustain | KnobId::Resonance | KnobId::PhaseRandom => {
                FloatRange::Linear { min: 0.0, max: 1.0 }
            }
            KnobId::Cutoff => FloatRange::Skewed {
                min: 20.0,
                max: 20000.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::Unison => FloatRange::Linear {
                min: 1.0,
                max: MAX_UNISON as f32,
            },
            KnobId::Detune => FloatRange::Linear {
                min: 0.0,
                max: 100.0,
            },
            // A map's range is a fraction of its target's range.
            KnobId::MapMin | KnobId::MapMax | KnobId::MapInMin | KnobId::MapInMax => {
                FloatRange::Linear { min: 0.0, max: 1.0 }
            }
            KnobId::MapOffset | KnobId::MapRange => FloatRange::Linear {
                min: -1.0,
                max: 1.0,
            },
            KnobId::MapCenter => FloatRange::Linear { min: 0.0, max: 1.0 },
            KnobId::WarpAmount | KnobId::MathA | KnobId::MathB => {
                FloatRange::Linear { min: 0.0, max: 1.0 }
            }
            KnobId::Coarse => FloatRange::Linear {
                min: -12.0,
                max: 12.0,
            },
            KnobId::Fine => FloatRange::Linear {
                min: -100.0,
                max: 100.0,
            },
            KnobId::Octave => FloatRange::Linear {
                min: -4.0,
                max: 4.0,
            },
            KnobId::Mix
            | KnobId::Spread
            | KnobId::Drive
            | KnobId::Tone
            | KnobId::Depth
            | KnobId::Size
            | KnobId::Damp
            | KnobId::Macro1
            | KnobId::Macro2
            | KnobId::Macro3
            | KnobId::Macro4 => FloatRange::Linear { min: 0.0, max: 1.0 },
            KnobId::Pan => FloatRange::Linear {
                min: -1.0,
                max: 1.0,
            },
            KnobId::Threshold => FloatRange::Linear {
                min: -60.0,
                max: 0.0,
            },
            KnobId::Ratio => FloatRange::Skewed {
                min: 1.0,
                max: 20.0,
                factor: FloatRange::skew_factor(-1.0),
            },
            KnobId::CompAttack => FloatRange::Skewed {
                min: 0.1,
                max: 100.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::CompRelease => FloatRange::Skewed {
                min: 10.0,
                max: 1000.0,
                factor: FloatRange::skew_factor(-1.5),
            },
            KnobId::Makeup => FloatRange::Linear {
                min: 0.0,
                max: 24.0,
            },
            KnobId::ReverbDecay => FloatRange::Skewed {
                min: 0.1,
                max: 20.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::DelayTime => FloatRange::Skewed {
                min: 1.0,
                max: 2000.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::Feedback => FloatRange::Linear {
                min: 0.0,
                max: 0.95,
            },
            KnobId::Center => FloatRange::Skewed {
                min: 100.0,
                max: 8000.0,
                factor: FloatRange::skew_factor(-2.0),
            },
            KnobId::FlangerDelay => FloatRange::Skewed {
                min: 0.1,
                max: 10.0,
                factor: FloatRange::skew_factor(-1.5),
            },
            KnobId::EqFreq1 | KnobId::EqFreq2 | KnobId::EqFreq3 | KnobId::EqFreq4 => {
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                }
            }
            KnobId::EqGain1 | KnobId::EqGain2 | KnobId::EqGain3 | KnobId::EqGain4 => {
                FloatRange::Linear {
                    min: -18.0,
                    max: 18.0,
                }
            }
            KnobId::EqQ1 | KnobId::EqQ2 | KnobId::EqQ3 | KnobId::EqQ4 => FloatRange::Skewed {
                min: 0.1,
                max: 10.0,
                factor: FloatRange::skew_factor(-1.0),
            },
            // Zero is free running. Each step past it is one of the note divisions.
            KnobId::LfoSync => FloatRange::Linear {
                min: 0.0,
                max: DIVISION_COUNT as f32,
            },
            KnobId::LfoRate => FloatRange::Skewed {
                min: 0.05,
                max: 200.0,
                factor: FloatRange::skew_factor(-2.0),
            },
        }
    }

    /// The value a new node starts with, in the knob's own units.
    pub fn default_plain(self) -> f32 {
        match self {
            KnobId::Gain => -10.0,
            KnobId::Slot1 | KnobId::Slot2 | KnobId::Slot3 | KnobId::Slot4 | KnobId::OscGain => 0.0,
            KnobId::Attack => 0.001,
            KnobId::Decay => 1.0,
            KnobId::Sustain => 1.0,
            KnobId::Release => 0.015,
            KnobId::Cutoff => 8000.0,
            KnobId::Resonance => 0.1,
            KnobId::Unison => 1.0,
            KnobId::Detune => 10.0,
            KnobId::PhaseRandom => 1.0,
            KnobId::Glide => 0.0,
            KnobId::MapMin | KnobId::MapInMin => 0.0,
            KnobId::MapMax | KnobId::MapInMax => 1.0,
            KnobId::MapOffset => 0.0,
            // The full swing, from 0 to 1, as a center and a width.
            KnobId::MapCenter => 0.5,
            KnobId::MapRange => 1.0,
            KnobId::MathA | KnobId::MathB => 0.5,
            KnobId::LfoRate => 1.0,
            KnobId::LfoSync => 0.0,
            KnobId::WarpAmount => 0.0,
            KnobId::Coarse | KnobId::Fine | KnobId::Octave => 0.0,
            KnobId::Mix => 0.5,
            KnobId::Macro1 | KnobId::Macro2 | KnobId::Macro3 | KnobId::Macro4 => 0.0,
            KnobId::Spread => 0.75,
            KnobId::Pan => 0.0,
            KnobId::Drive => 0.3,
            KnobId::Tone => 0.5,
            KnobId::Threshold => -18.0,
            KnobId::Ratio => 4.0,
            KnobId::CompAttack => 10.0,
            KnobId::CompRelease => 100.0,
            KnobId::Makeup => 0.0,
            KnobId::Depth => 0.5,
            KnobId::ReverbDecay => 2.0,
            KnobId::Size => 0.5,
            KnobId::Damp => 0.5,
            KnobId::DelayTime => 350.0,
            KnobId::Feedback => 0.4,
            KnobId::Center => 1000.0,
            KnobId::FlangerDelay => 2.0,
            KnobId::EqFreq1 => 100.0,
            KnobId::EqFreq2 => 500.0,
            KnobId::EqFreq3 => 2500.0,
            KnobId::EqFreq4 => 8000.0,
            KnobId::EqGain1 | KnobId::EqGain2 | KnobId::EqGain3 | KnobId::EqGain4 => 0.0,
            KnobId::EqQ1 | KnobId::EqQ4 => 0.71,
            KnobId::EqQ2 | KnobId::EqQ3 => 1.0,
        }
    }

    pub fn default_normalized(self) -> f32 {
        self.to_normalized(self.default_plain())
    }

    /// The knob's value in its own units for a normalized value from 0.0 to 1.0.
    pub fn to_plain(self, normalized: f32) -> f32 {
        let plain = self.range().unnormalize(normalized);
        match self {
            KnobId::Unison | KnobId::LfoSync | KnobId::Coarse | KnobId::Octave => plain.round(),
            _ => plain,
        }
    }

    pub fn to_normalized(self, plain: f32) -> f32 {
        self.range().normalize(plain)
    }

    /// Rounds a normalized value to the knob's step, if it has one. Unison moves in whole voices.
    pub fn snap(self, normalized: f32) -> f32 {
        match self {
            KnobId::Unison | KnobId::LfoSync | KnobId::Coarse | KnobId::Octave => {
                self.to_normalized(self.to_plain(normalized))
            }
            _ => normalized.clamp(0.0, 1.0),
        }
    }

    /// How many of the number a person types are one plain unit of this knob. A knob the node
    /// shows as a percentage of a 0 to 1 value is typed as that percentage, so 75 means 0.75.
    fn entry_scale(self) -> f32 {
        match self {
            KnobId::MapMin
            | KnobId::MapMax
            | KnobId::MapInMin
            | KnobId::MapInMax
            | KnobId::MapOffset
            | KnobId::MapCenter
            | KnobId::MapRange
            | KnobId::MathA
            | KnobId::MathB
            | KnobId::WarpAmount
            | KnobId::Mix
            | KnobId::Pan
            | KnobId::Spread
            | KnobId::Drive
            | KnobId::Tone
            | KnobId::Depth
            | KnobId::Size
            | KnobId::Damp
            | KnobId::Feedback
            | KnobId::Macro1
            | KnobId::Macro2
            | KnobId::Macro3
            | KnobId::Macro4 => 100.0,
            _ => 1.0,
        }
    }

    /// The number to show in a box for typing over a knob's value, in the units the node shows,
    /// without the unit's name.
    pub fn to_entry(self, normalized: f32) -> f32 {
        self.to_plain(normalized) * self.entry_scale()
    }

    /// The normalized value for a number typed in those units. It stops at the ends of the
    /// knob's range.
    pub fn from_entry(self, number: f32) -> f32 {
        self.snap(self.to_normalized(number / self.entry_scale()))
    }

    /// The value as the node shows it, with its unit.
    pub fn format(self, normalized: f32) -> String {
        let plain = self.to_plain(normalized);
        match self {
            KnobId::Gain
            | KnobId::Slot1
            | KnobId::Slot2
            | KnobId::Slot3
            | KnobId::Slot4
            | KnobId::OscGain => {
                if plain <= GAIN_MIN_DB {
                    "-inf".to_string()
                } else {
                    format!("{plain:.1} dB")
                }
            }
            KnobId::Attack | KnobId::Decay | KnobId::Release => format!("{plain:.3} s"),
            KnobId::Glide => format!("{plain:.2} s"),
            KnobId::Sustain | KnobId::Resonance | KnobId::PhaseRandom => format!("{plain:.2}"),
            KnobId::Cutoff => format!("{plain:.0} Hz"),
            KnobId::Unison => format!("{plain:.0}"),
            KnobId::Detune => format!("{plain:.1} ct"),
            KnobId::MapMin | KnobId::MapMax | KnobId::MapInMin | KnobId::MapInMax => {
                format!("{:.0}%", plain * 100.0)
            }
            KnobId::WarpAmount | KnobId::MathA | KnobId::MathB => {
                format!("{:.0}%", plain * 100.0)
            }
            KnobId::MapOffset => format!("{:+.0}%", plain * 100.0),
            KnobId::MapCenter | KnobId::MapRange => format!("{:.0}%", plain * 100.0),
            KnobId::Coarse => format!("{:+} st", plain.round() as i32),
            KnobId::Fine => format!("{plain:+.0} ct"),
            KnobId::Octave => format!("{:+} oct", plain.round() as i32),
            KnobId::Mix
            | KnobId::Macro1
            | KnobId::Macro2
            | KnobId::Macro3
            | KnobId::Macro4
            | KnobId::Spread
            | KnobId::Drive
            | KnobId::Tone
            | KnobId::Depth
            | KnobId::Size
            | KnobId::Damp
            | KnobId::Feedback => format!("{:.0}%", plain * 100.0),
            KnobId::Threshold | KnobId::Makeup => format!("{plain:.1} dB"),
            KnobId::Ratio => format!("{plain:.1}:1"),
            KnobId::CompAttack => format!("{plain:.1} ms"),
            KnobId::CompRelease | KnobId::DelayTime => format!("{plain:.0} ms"),
            KnobId::FlangerDelay => format!("{plain:.2} ms"),
            KnobId::ReverbDecay => format!("{plain:.2} s"),
            KnobId::Center => format!("{plain:.0} Hz"),
            KnobId::EqFreq1 | KnobId::EqFreq2 | KnobId::EqFreq3 | KnobId::EqFreq4 => {
                format!("{plain:.0} Hz")
            }
            KnobId::EqGain1 | KnobId::EqGain2 | KnobId::EqGain3 | KnobId::EqGain4 => {
                format!("{plain:+.1} dB")
            }
            KnobId::EqQ1 | KnobId::EqQ2 | KnobId::EqQ3 | KnobId::EqQ4 => format!("{plain:.2}"),
            KnobId::Pan => match (plain * 100.0).round() as i32 {
                0 => "C".to_string(),
                side if side < 0 => format!("L {}", -side),
                side => format!("R {side}"),
            },
            KnobId::LfoRate => format!("{plain:.2} Hz"),
            KnobId::LfoSync => match plain.round() as usize {
                0 => "Free".to_string(),
                step => lfo::DIVISIONS[(step - 1).min(DIVISION_COUNT - 1)]
                    .0
                    .to_string(),
            },
        }
    }
}

/// A number as the entry box shows it, with trailing zeros dropped. Large numbers get fewer
/// decimals, so the noise left by a trip through a skewed range does not show: a cutoff of 8000
/// reads "8000" and not "8000.001".
pub fn entry_text(number: f32) -> String {
    let decimals = match number.abs() {
        n if n >= 1000.0 => 0,
        n if n >= 100.0 => 1,
        n if n >= 10.0 => 2,
        _ => 3,
    };
    let text = format!("{number:.decimals$}");
    // Zeros only come off after a decimal point, or 8000 would read as 8.
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text.as_str()
    };
    if text.is_empty() || text == "-" || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// The number a person typed in the entry box. "inf" and "-inf" are the ends of a knob that runs
/// to silence, and anything else must be an ordinary finite number.
pub fn parse_entry(text: &str) -> Option<f32> {
    let text = text.trim();
    match text.to_ascii_lowercase().as_str() {
        "inf" | "+inf" => return Some(f32::INFINITY),
        "-inf" => return Some(f32::NEG_INFINITY),
        _ => {}
    }
    text.parse::<f32>().ok().filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every knob's default survives a trip through the normalized range.
    #[test]
    fn defaults_round_trip() {
        for id in KnobId::ALL {
            let plain = id.to_plain(id.default_normalized());
            let tolerance = id.default_plain().abs().max(1.0) * 1e-4;
            assert!(
                (plain - id.default_plain()).abs() < tolerance,
                "{id:?}: {plain} vs {}",
                id.default_plain()
            );
        }
    }

    #[test]
    fn unison_moves_in_whole_voices() {
        let snapped = KnobId::Unison.snap(0.4);
        assert_eq!(KnobId::Unison.to_plain(snapped).fract(), 0.0);
    }

    #[test]
    fn gain_reads_minus_inf_at_the_floor() {
        assert_eq!(KnobId::Gain.format(0.0), "-inf");
        assert_eq!(KnobId::Gain.format(1.0), "0.0 dB");
    }

    /// Sync steps through Free and then each note division, and reads each one's name.
    #[test]
    fn sync_steps_through_free_and_the_divisions() {
        assert_eq!(KnobId::LfoSync.format(0.0), "Free");
        let step = |n: usize| KnobId::LfoSync.to_normalized(n as f32);
        assert_eq!(KnobId::LfoSync.format(step(3)), "1/4");
        assert_eq!(KnobId::LfoSync.format(step(8)), "1/8 triplet");
        assert_eq!(
            KnobId::LfoSync.to_plain(KnobId::LfoSync.snap(0.5)).fract(),
            0.0
        );
    }

    /// A quarter note is one LFO cycle per beat, and a whole note is a cycle every four beats.
    #[test]
    fn divisions_match_their_beat_lengths() {
        assert_eq!(crate::nodes::lfo::cycles_per_beat(3), 1.0);
        assert_eq!(crate::nodes::lfo::cycles_per_beat(1), 0.25);
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    /// The Rate knob reaches audio-rate noise steps, and each rate reads back in Hz.
    #[test]
    fn rate_reaches_two_hundred_hertz() {
        let top = KnobId::LfoRate.to_plain(1.0);
        assert!((top - 200.0).abs() < 1e-3, "{top}");
        assert_eq!(KnobId::LfoRate.format(1.0), "200.00 Hz");
    }
}

#[cfg(test)]
mod pan_tests {
    use super::*;

    /// The pan reads as a side and a percentage, and the centre is the knob's default.
    #[test]
    fn pan_reads_as_a_side() {
        assert_eq!(KnobId::Pan.format(KnobId::Pan.to_normalized(0.0)), "C");
        assert_eq!(KnobId::Pan.format(0.0), "L 100");
        assert_eq!(KnobId::Pan.format(1.0), "R 100");
        assert_eq!(KnobId::Pan.format(KnobId::Pan.to_normalized(-0.25)), "L 25");
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    /// A typed number is in the units the node shows, so every knob's own display value reads
    /// back as the same value.
    #[test]
    fn every_knob_reads_back_what_it_shows() {
        for id in KnobId::ALL {
            let normalized = id.default_normalized();
            let number = id.to_entry(normalized);
            let back = id.from_entry(number);
            assert!(
                (back - normalized).abs() < 1e-4,
                "{id:?}: {normalized} -> {number} -> {back}"
            );
        }
    }

    #[test]
    fn percent_knobs_are_typed_as_percent() {
        assert_eq!(KnobId::Mix.from_entry(75.0), 0.75);
        assert_eq!(KnobId::Mix.to_entry(0.5), 50.0);
        assert!((KnobId::Pan.from_entry(-25.0) - KnobId::Pan.to_normalized(-0.25)).abs() < 1e-6);
        assert_eq!(KnobId::Macro1.from_entry(100.0), 1.0);
    }

    /// Units are the ones on the label: Hz, dB, seconds, milliseconds.
    #[test]
    fn other_knobs_are_typed_in_their_own_units() {
        assert!((KnobId::Cutoff.to_plain(KnobId::Cutoff.from_entry(440.0)) - 440.0).abs() < 0.1);
        assert!(
            (KnobId::Threshold.to_plain(KnobId::Threshold.from_entry(-24.0)) + 24.0).abs() < 1e-3
        );
        assert!(
            (KnobId::CompAttack.to_plain(KnobId::CompAttack.from_entry(5.0)) - 5.0).abs() < 1e-3
        );
        assert!((KnobId::Attack.to_plain(KnobId::Attack.from_entry(0.25)) - 0.25).abs() < 1e-4);
    }

    /// A number past the ends stops at them, and a stepped knob lands on a step.
    #[test]
    fn typed_numbers_stop_at_the_ends_and_snap() {
        assert_eq!(KnobId::Cutoff.from_entry(1.0e9), 1.0);
        assert_eq!(KnobId::Cutoff.from_entry(-5.0), 0.0);
        assert_eq!(KnobId::Mix.from_entry(250.0), 1.0);
        let unison = KnobId::Unison.from_entry(3.4);
        assert_eq!(KnobId::Unison.to_plain(unison), 3.0);
        assert_eq!(KnobId::Gain.from_entry(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn entry_text_is_short() {
        assert_eq!(entry_text(8000.0), "8000");
        assert_eq!(entry_text(8000.001), "8000");
        assert_eq!(entry_text(440.04), "440");
        assert_eq!(entry_text(440.5), "440.5");
        assert_eq!(entry_text(12.345), "12.35");
        assert_eq!(entry_text(0.5), "0.5");
        assert_eq!(entry_text(-12.25), "-12.25");
        assert_eq!(entry_text(0.001), "0.001");
        assert_eq!(entry_text(0.0), "0");
        assert_eq!(entry_text(8000.0), "8000");
        assert_eq!(entry_text(20000.0), "20000");
        assert_eq!(entry_text(100.0), "100");
        assert_eq!(entry_text(-1000.0), "-1000");
        assert_eq!(entry_text(-0.0001), "0");
    }

    #[test]
    fn typed_text_parses_or_is_refused() {
        assert_eq!(parse_entry("440"), Some(440.0));
        assert_eq!(parse_entry(" -3.5 "), Some(-3.5));
        assert_eq!(parse_entry("+2"), Some(2.0));
        assert_eq!(parse_entry("-inf"), Some(f32::NEG_INFINITY));
        assert_eq!(parse_entry("INF"), Some(f32::INFINITY));
        for bad in ["", "-", ".", "1.2.3", "abc", "nan", "1e999"] {
            assert_eq!(parse_entry(bad), None, "{bad:?}");
        }
    }
}
