//! The oscillator node. It makes the sound: a waveform with unison, detune, and phase randomness.

use super::viewer::{sample, Live, Trace};
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};
use crate::dsp::noise::NoiseType;
use crate::dsp::warp::{warp_phase, warp_value, WarpMode};
use crate::dsp::wavetable::Waveform;

/// How many cycles the viewer draws.
const TRACE_CYCLES: f32 = 1.0;

pub struct Oscillator;

/// The numbers of the oscillator's sections, as `Row::section` and `sections` use them.
const SECTION_PITCH: usize = 0;
const SECTION_UNISON: usize = 1;
const SECTION_WARP: usize = 2;

/// The waveform dropdown's rows. The LFO uses the same list.
pub const WAVEFORM_LABELS: [&str; 5] = ["Sine", "Saw", "Square", "Triangle", "Noise"];

/// The noise dropdown's rows. It shows only under the noise waveform.
pub const NOISE_LABELS: [&str; 4] = ["White", "Pink", "Brown", "Geiger"];

impl NodeBehavior for Oscillator {
    fn title(&self) -> &'static str {
        "Oscillator"
    }

    /// The rows in the order they show. The waveform and noise dropdowns lead. The shape row has
    /// the envelope input and the audio output, so both pins stay in the same place whatever is
    /// folded. Level and pan follow, and the rest is in three sections that fold: pitch, unison,
    /// and warp. The warp dropdown sits directly above the warp amount.
    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        // The notes pin sits on the wave dropdown's row, at the very top. The dropdown starts
        // clear of the pin, so the row stays one dropdown tall. With no wire here the oscillator
        // is silent.
        let mut rows = vec![Row {
            input: Some(Port::Notes),
            ..Row::control(Control::Dropdown(0))
        }];
        if waveform(node) == Waveform::Noise {
            rows.push(Row::control(Control::Dropdown(2)));
        }
        // A wavetable wired to the pin on the viewer's row plays in place of the waveform, at the
        // table's own Position. The pin shares the row so the node stays as tall as it was.
        rows.push(Row {
            input: Some(Port::TableIn),
            ..Row::control(Control::Viewer)
        });
        // The envelope input shapes this oscillator's amplitude, one copy per voice.
        rows.push(Row::input(Port::Shape).with_output(Port::Audio));
        rows.push(Row::driven_knob(KnobId::OscGain));
        rows.push(Row::driven_knob(KnobId::Pan));

        // The transpose knobs shift the oscillator's pitch. A control wire can drive each one.
        rows.push(Row::section(SECTION_PITCH));
        rows.push(Row::driven_knob(KnobId::Coarse));
        rows.push(Row::driven_knob(KnobId::Fine));
        rows.push(Row::driven_knob(KnobId::Octave));

        rows.push(Row::section(SECTION_UNISON));
        rows.push(Row::driven_knob(KnobId::Unison));
        rows.push(Row::driven_knob(KnobId::Detune));
        rows.push(Row::driven_knob(KnobId::Spread));
        rows.push(Row::driven_knob(KnobId::PhaseRandom));

        rows.push(Row::section(SECTION_WARP));
        // The audio pin sits on the warp dropdown's row. The PM and PD warps read it, and the
        // others leave it alone. The dropdown starts clear of the pin, as it does for the notes.
        rows.push(Row {
            input: Some(Port::AudioIn(0)),
            ..Row::control(Control::Dropdown(1))
        });
        rows.push(Row::driven_knob(KnobId::WarpAmount));
        rows
    }

    /// Pitch and warp start folded, since they are touched less than the unison.
    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: "Pitch",
                collapsed: true,
            },
            Section {
                title: "Unison",
                collapsed: false,
            },
            Section {
                title: "Warp",
                collapsed: true,
            },
        ]
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::Mute
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Wave", "Warp", "Noise"]
    }

    /// The waveform, the warp mode, and under the noise waveform, the noise type.
    fn dropdowns(&self, node: &FlodeNode) -> Vec<&'static [&'static str]> {
        let mut dropdowns: Vec<&'static [&'static str]> = vec![&WAVEFORM_LABELS, &WARP_LABELS];
        if waveform(node) == Waveform::Noise {
            dropdowns.push(&NOISE_LABELS);
        }
        dropdowns
    }

    /// One cycle of the chosen waveform, through the warp at its live amount. Noise has no
    /// cycle, so it draws a fixed jagged line that does not change between frames.
    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let wave = waveform(node);
        let points = if wave == Waveform::Noise {
            sample(samples, noise_shape)
        } else {
            let mode = warp_mode(node);
            let amount = live.value(KnobId::WarpAmount);
            sample(samples, |x| {
                let phase = (x * TRACE_CYCLES).fract();
                let phase = warp_phase(mode, phase, amount);
                let value = warp_value(mode, wave.naive(phase), amount);
                value * 0.5 + 0.5
            })
        };
        Some(Trace {
            points,
            ..Trace::default()
        })
    }
}

/// A fixed jagged line for the noise waveform, from 0.0 to 1.0. It hashes the position, so the
/// same position gives the same height every frame.
fn noise_shape(x: f32) -> f32 {
    let step = (x * 95.0).round() as u32;
    let mut h = step.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    0.1 + 0.8 * (h & 0xFFFF) as f32 / 65535.0
}

/// The waveform an oscillator node plays.
pub fn waveform(node: &FlodeNode) -> Waveform {
    Waveform::ALL[node.choice(0).min(Waveform::ALL.len() - 1)]
}

/// The noise type an oscillator node plays under the noise waveform.
pub fn noise_type(node: &FlodeNode) -> NoiseType {
    NoiseType::ALL[node.choice(2).min(NoiseType::ALL.len() - 1)]
}

/// The warp mode an oscillator node applies.
pub fn warp_mode(node: &FlodeNode) -> WarpMode {
    WarpMode::ALL
        .get(node.choice(1))
        .copied()
        .unwrap_or_default()
}

/// The warp dropdown's rows.
pub const WARP_LABELS: [&str; 10] = [
    "None", "Bend", "Sync", "PWM", "Asym", "Flip", "Mirror", "Quantize", "PM", "PD",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn oscillator() -> FlodeNode {
        FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 0.0))
    }

    fn trace_of(node: &FlodeNode, live: &Live) -> Vec<f32> {
        node.trace(live)
            .expect("the oscillator has a viewer")
            .points
    }

    /// Every waveform gives one point per sample, inside the box.
    #[test]
    fn every_waveform_traces_samples_points_in_range() {
        let mut node = oscillator();
        for choice in 0..WAVEFORM_LABELS.len() {
            node.choose(0, choice);
            let trace = Oscillator
                .trace(&node, &Live::of(&node), 64)
                .expect("a trace");
            assert_eq!(trace.points.len(), 64, "{choice}");
            assert!(trace.points.iter().all(|p| (0.0..=1.0).contains(p)));
        }
    }

    /// The node has a viewer row and a trace.
    #[test]
    fn the_oscillator_has_a_viewer() {
        let node = oscillator();
        assert!(node.has_viewer());
        assert!(node.trace(&Live::of(&node)).is_some());
    }

    /// A sine cycle crosses the midline once between its ends, at the half cycle. Both ends sit
    /// on the midline.
    #[test]
    fn a_sine_crosses_the_midline_once_between_its_ends() {
        let mut node = oscillator();
        node.choose(0, 0);
        let points = Oscillator
            .trace(&node, &Live::of(&node), 100)
            .expect("a trace")
            .points;
        let mut crossings = 0;
        let mut side = 0.0f32;
        for p in points {
            let s = p - 0.5;
            if s == 0.0 {
                continue;
            }
            if side != 0.0 && s.signum() != side {
                crossings += 1;
            }
            side = s.signum();
        }
        assert_eq!(crossings, 1);
    }

    /// The warp amount in `Live` changes the picture.
    #[test]
    fn the_warp_amount_changes_the_trace() {
        let mut node = oscillator();
        node.choose(0, 1);
        node.choose(1, 1);
        let low = Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(0.0));
        let high = Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(1.0));
        assert_ne!(trace_of(&node, &low), trace_of(&node, &high));
    }

    /// A value a control wire drives, passed through `Live::new`, moves the shape even when the
    /// node's own value stays put.
    #[test]
    fn a_driven_value_moves_the_trace() {
        let mut node = oscillator();
        node.choose(0, 2);
        node.choose(1, 3);
        node.set_value(KnobId::WarpAmount, 0.5);
        let own = Live::of(&node);
        let driven = Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(0.9));
        assert_ne!(trace_of(&node, &own), trace_of(&node, &driven));
    }

    /// The noise trace is the same on every call.
    #[test]
    fn the_noise_trace_is_fixed() {
        let mut node = oscillator();
        node.choose(0, 4);
        let live = Live::of(&node);
        assert_eq!(trace_of(&node, &live), trace_of(&node, &live));
    }

    /// The warp dropdown lists one row for every warp mode, in the same order.
    #[test]
    fn the_warp_labels_match_the_modes() {
        assert_eq!(WARP_LABELS.len(), WarpMode::ALL.len());
        assert!(!WARP_LABELS.contains(&"FM"));
    }

    /// An oscillator has one audio input, on the warp row, whatever the warp. Only PM and PD read
    /// it, but the pin stays so a saved wire doesn't depend on the mode.
    #[test]
    fn an_oscillator_has_one_audio_input_on_the_warp_row() {
        let mut node = oscillator();
        for choice in 0..WarpMode::ALL.len() {
            node.choose(1, choice);
            let audio: Vec<Port> = node
                .input_ports()
                .into_iter()
                .filter(|port| port.is_audio())
                .collect();
            assert_eq!(audio, [Port::AudioIn(0)]);
        }
        let rows = Oscillator.rows(&node);
        let pin = rows
            .iter()
            .find(|row| row.input == Some(Port::AudioIn(0)))
            .unwrap();
        assert_eq!(pin.control, Some(Control::Dropdown(1)));
    }

    /// The two warps that read the pin are named for what they do.
    #[test]
    fn pm_and_pd_are_the_last_two_warps() {
        assert_eq!(&WARP_LABELS[8..], ["PM", "PD"]);
        assert_eq!(WarpMode::ALL[8], WarpMode::Pm);
        assert_eq!(WarpMode::ALL[9], WarpMode::Pd);
    }

    /// A graph saved while the oscillator had an FM warp holds a choice past the last mode. It
    /// loads as no warp, not as whichever mode is last.
    #[test]
    fn a_saved_choice_past_the_last_warp_means_none() {
        let mut node = oscillator();
        node.choose(1, WarpMode::ALL.len());
        assert_eq!(warp_mode(&node), WarpMode::None);
    }
}
