//! The oscillator node. It plays a wavetable wired to its Table pin, with unison, detune, warp and
//! phase randomness. It has no waveform of its own: with no table wired it is silent, like an
//! oscillator with no notes.

use super::viewer::{sample, Live, Trace};
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section};
use crate::dsp::warp::{warp_phase, warp_value, WarpMode};

/// How many cycles the viewer draws.
const TRACE_CYCLES: f32 = 1.0;

pub struct Oscillator;

/// The numbers of the oscillator's sections, as `Row::section` and `sections` use them.
const SECTION_PITCH: usize = 0;
const SECTION_UNISON: usize = 1;
const SECTION_WARP: usize = 2;

impl NodeBehavior for Oscillator {
    fn title(&self) -> &'static str {
        "Oscillator"
    }

    /// The rows in the order they show. The notes pin leads, since with no wire there the
    /// oscillator is silent. The viewer's row has the table pin, which is the oscillator's only
    /// source of sound. The shape row has the envelope input and the audio output, so both pins
    /// stay in the same place whatever is folded. Level and pan follow, and the rest is in three
    /// sections that fold: pitch, unison, and warp. The warp dropdown sits directly above the warp
    /// amount.
    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![Row::input(Port::Notes)];
        // The wavetable wired to the pin on the viewer's row is what plays, at the table's own
        // Position.
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
        &["", "Warp"]
    }

    /// Only the warp mode is a dropdown. It is number 1, since number 0 used to be the waveform,
    /// and a saved graph keeps its warp choice at that place. Nothing shows number 0 now.
    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&[], &WARP_LABELS]
    }

    /// One cycle of the wired table at its current position, through the warp at its live
    /// amount. With no table the box shows a flat line, since there is nothing to play.
    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let cycle = live.table().filter(|cycle| !cycle.is_empty());
        let points = if let Some(cycle) = cycle {
            let mode = warp_mode(node);
            let amount = live.value(KnobId::WarpAmount);
            sample(samples, |x| {
                let phase = warp_phase(mode, (x * TRACE_CYCLES).fract(), amount);
                let value = warp_value(mode, cycle_at(cycle, phase), amount);
                value * 0.5 + 0.5
            })
        } else {
            vec![0.5; samples]
        };
        Some(Trace {
            points,
            ..Trace::default()
        })
    }
}

/// A cycle read at `phase` from 0.0 to 1.0, with a straight line between its samples. The cycle
/// wraps, so the last sample leads back to the first.
fn cycle_at(cycle: &[f32], phase: f32) -> f32 {
    let at = phase.rem_euclid(1.0) * cycle.len() as f32;
    let i = (at as usize).min(cycle.len() - 1);
    let frac = at - i as f32;
    let (a, b) = (cycle[i], cycle[(i + 1) % cycle.len()]);
    a + (b - a) * frac
}

/// A fixed jagged line for the noise node's viewer, from 0.0 to 1.0. It hashes the position, so
/// the same position gives the same height every frame.
pub(super) fn noise_shape(x: f32) -> f32 {
    let step = (x * 95.0).round() as u32;
    let mut h = step.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    0.1 + 0.8 * (h & 0xFFFF) as f32 / 65535.0
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
    use crate::nodes::viewer::TRACE_SAMPLES;
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

    /// One cycle of a sine and of a saw, as a table at some position would give.
    fn sine() -> Vec<f32> {
        (0..2048)
            .map(|n| (n as f32 / 2048.0 * std::f32::consts::TAU).sin())
            .collect()
    }

    fn saw() -> Vec<f32> {
        (0..2048).map(|n| n as f32 / 1024.0 - 1.0).collect()
    }

    /// With no table the box shows a flat line at the middle, and with one it shows a point per
    /// sample, inside the box.
    #[test]
    fn the_trace_is_flat_without_a_table_and_in_range_with_one() {
        let node = oscillator();
        let flat = Oscillator
            .trace(&node, &Live::of(&node), 64)
            .expect("a trace");
        assert_eq!(flat.points, vec![0.5; 64]);
        let live = Live::of(&node).with_table(saw());
        let trace = Oscillator.trace(&node, &live, 64).expect("a trace");
        assert_eq!(trace.points.len(), 64);
        assert!(trace.points.iter().all(|p| (0.0..=1.0).contains(p)));
        assert_ne!(trace.points, flat.points);
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
        let node = oscillator();
        let live = Live::of(&node).with_table(sine());
        let points = Oscillator.trace(&node, &live, 100).expect("a trace").points;
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
        node.choose(1, 1);
        let low =
            Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(0.0)).with_table(saw());
        let high =
            Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(1.0)).with_table(saw());
        assert_ne!(trace_of(&node, &low), trace_of(&node, &high));
    }

    /// A value a control wire drives, passed through `Live::new`, moves the shape even when the
    /// node's own value stays put.
    #[test]
    fn a_driven_value_moves_the_trace() {
        let mut node = oscillator();
        node.choose(1, 3);
        node.set_value(KnobId::WarpAmount, 0.5);
        let own = Live::of(&node).with_table(saw());
        let driven =
            Live::new(&node, |id| (id == KnobId::WarpAmount).then_some(0.9)).with_table(saw());
        assert_ne!(trace_of(&node, &own), trace_of(&node, &driven));
    }

    /// A wired table's cycle replaces the waveform in the trace, resampled to the point count,
    /// and a warp still bends it.
    #[test]
    fn a_table_cycle_replaces_the_waveform() {
        let mut node = oscillator();
        // One cycle of a square, as a table at the current position would give.
        let square: Vec<f32> = (0..2048)
            .map(|n| if n < 1024 { 1.0 } else { -1.0 })
            .collect();
        let live = Live::of(&node).with_table(square);
        let points = trace_of(&node, &live);
        assert_eq!(points.len(), TRACE_SAMPLES);
        assert_eq!(points[10], 1.0);
        assert_eq!(points[TRACE_SAMPLES - 10], 0.0);
        assert_ne!(points, trace_of(&node, &Live::of(&node)));
        node.choose(1, 5);
        node.set_value(KnobId::WarpAmount, 1.0);
        let live = Live::of(&node).with_table(live.table().unwrap().to_vec());
        let flipped = trace_of(&node, &live);
        assert_eq!(flipped[10], 0.0);
    }

    /// The oscillator has a notes pin and a table pin, and no dropdown of its own for the wave:
    /// the warp is the only dropdown it shows.
    #[test]
    fn the_oscillator_has_no_wave_dropdown() {
        let node = oscillator();
        let dropdowns: Vec<Control> = Oscillator
            .rows(&node)
            .into_iter()
            .filter_map(|row| row.control)
            .filter(|control| matches!(control, Control::Dropdown(_)))
            .collect();
        assert_eq!(dropdowns, vec![Control::Dropdown(1)]);
        let ports = node.input_ports();
        assert_eq!(ports[0], Port::Notes);
        assert!(ports.contains(&Port::TableIn));
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
