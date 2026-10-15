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
    /// An oscillator's transpose in octaves, from -2 to 2.
    Octave,
    /// A mix node's blend, from 0.0 (just signal A) to 1.0 (fully the chosen combination).
    Mix,
    /// An oscillator's place in the stereo field, from -1 (left) to 1 (right).
    Pan,
    /// How far an oscillator's unison copies spread across the stereo field, from 0 to 1.
    Spread,
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
    pub const COUNT: usize = 29;

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
            KnobId::WarpAmount => FloatRange::Linear { min: 0.0, max: 1.0 },
            KnobId::Coarse => FloatRange::Linear {
                min: -12.0,
                max: 12.0,
            },
            KnobId::Fine => FloatRange::Linear {
                min: -100.0,
                max: 100.0,
            },
            KnobId::Octave => FloatRange::Linear {
                min: -2.0,
                max: 2.0,
            },
            KnobId::Mix | KnobId::Spread => FloatRange::Linear { min: 0.0, max: 1.0 },
            KnobId::Pan => FloatRange::Linear {
                min: -1.0,
                max: 1.0,
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
            KnobId::LfoRate => 1.0,
            KnobId::LfoSync => 0.0,
            KnobId::WarpAmount => 0.0,
            KnobId::Coarse | KnobId::Fine | KnobId::Octave => 0.0,
            KnobId::Mix => 0.5,
            KnobId::Spread => 0.75,
            KnobId::Pan => 0.0,
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
            KnobId::WarpAmount => format!("{:.0}%", plain * 100.0),
            KnobId::Coarse => format!("{:+} st", plain.round() as i32),
            KnobId::Fine => format!("{plain:+.0} ct"),
            KnobId::Octave => format!("{:+} oct", plain.round() as i32),
            KnobId::Mix | KnobId::Spread => format!("{:.0}%", plain * 100.0),
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
