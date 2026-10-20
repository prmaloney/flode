//! The LFO node. It is a low-frequency oscillator. Its shape goes out on the shape pin as a control
//! signal, and it drives any knob a control wire reaches. It makes no sound.
//!
//! With Sync set to a note division, the LFO follows the host's playhead instead of its rate.
//!
//! The Custom wave is a curve the user draws as points in the viewer. One shot plays one cycle from
//! each note and then holds the end value.

use super::viewer::{sample, Live, Trace};
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::curve::Curve;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::Waveform;

pub struct Lfo;

/// The Wave dropdown's rows: the oscillator's waves, in the same order, then Custom.
pub const LFO_WAVE_LABELS: [&str; 6] = ["Sine", "Saw", "Square", "Triangle", "Noise", "Custom"];
/// The row of Custom, which follows the built-in waves.
const CUSTOM_ROW: usize = Waveform::ALL.len();

/// The shape an LFO node runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LfoShape {
    Wave(Waveform),
    Custom,
}

/// The note divisions Sync offers, with how many cycles of the LFO fit in one beat. Sync step `n`
/// is `DIVISIONS[n - 1]`. Step zero is free running.
pub const DIVISIONS: [(&str, f32); 8] = [
    ("1/1", 0.25),
    ("1/2", 0.5),
    ("1/4", 1.0),
    ("1/8", 2.0),
    ("1/16", 4.0),
    ("1/4 dotted", 2.0 / 3.0),
    ("1/8 dotted", 4.0 / 3.0),
    ("1/8 triplet", 3.0),
];
pub const DIVISION_COUNT: usize = DIVISIONS.len();

/// The cycles per beat for a Sync step past zero.
pub fn cycles_per_beat(step: usize) -> f32 {
    DIVISIONS[(step.max(1) - 1).min(DIVISION_COUNT - 1)].1
}

impl NodeBehavior for Lfo {
    fn title(&self) -> &'static str {
        "LFO"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::control(Control::Viewer),
            Row::driven_knob(KnobId::LfoRate).with_output(Port::Shape),
            Row::plain_knob(KnobId::LfoSync),
            Row::control(Control::Dropdown(0)),
            Row::control(Control::Switch(0)),
            Row::control(Control::Switch(1)),
        ]
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Retrigger", "One shot"]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&LFO_WAVE_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Wave"]
    }

    /// One cycle of the chosen wave. The box shows a fixed number of cycles, so the rate knob
    /// does not change it.
    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let shape = shape(node);
        let points = match shape {
            LfoShape::Wave(Waveform::Noise) => noise_points(samples),
            LfoShape::Wave(wave) => sample(samples, |x| {
                // The engine maps the wave from -1..1 to 0..1 the same way.
                (wave.naive((x * CYCLES as f32).fract()) + 1.0) * 0.5
            }),
            LfoShape::Custom => {
                let curve = curve(node);
                sample(samples, |x| curve.eval(x))
            }
        };
        // The dot rides the cycle shown, at the LFO's phase. Noise holds a random value per cycle,
        // so its fixed picture has no place for a dot.
        let cursor = match (shape, live.lfo()) {
            (LfoShape::Wave(Waveform::Noise), _) | (_, None) => None,
            (_, Some((phase, level))) => Some((phase.clamp(0.0, 1.0) / CYCLES as f32, level)),
        };
        Some(Trace {
            points,
            cursor,
            ..Trace::default()
        })
    }
}

/// Cycles of the wave the viewer shows.
const CYCLES: usize = 1;
/// Steps in the noise trace.
const NOISE_STEPS: usize = 16;

/// A fixed stepped trace for Noise. The engine holds a random value per cycle, so this holds one
/// per step. The seed is fixed, so the picture is the same every frame.
fn noise_points(samples: usize) -> Vec<f32> {
    let mut rng = Rng::default();
    let steps: Vec<f32> = (0..NOISE_STEPS).map(|_| rng.unit()).collect();
    sample(samples, |x| {
        steps[((x * NOISE_STEPS as f32) as usize).min(NOISE_STEPS - 1)]
    })
}

/// The shape an LFO node runs.
pub fn shape(node: &FlodeNode) -> LfoShape {
    let row = node.choice(0);
    if row >= CUSTOM_ROW {
        LfoShape::Custom
    } else {
        LfoShape::Wave(Waveform::ALL[row])
    }
}

/// The built-in waveform an LFO node runs. A Custom LFO has none, and reads as a sine.
pub fn waveform(node: &FlodeNode) -> Waveform {
    match shape(node) {
        LfoShape::Wave(wave) => wave,
        LfoShape::Custom => Waveform::Sine,
    }
}

/// True while the node's wave is Custom, which is when its viewer takes points.
pub fn is_custom(node: &FlodeNode) -> bool {
    shape(node) == LfoShape::Custom
}

/// The node's custom curve, or the default if it has none.
pub fn curve(node: &FlodeNode) -> Curve {
    Curve::from_points(&node.points).unwrap_or_default()
}

/// Stores `curve` as the node's custom wave.
pub fn set_curve(node: &mut FlodeNode, curve: &Curve) {
    node.points = curve.points().to_vec();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::viewer::TRACE_SAMPLES;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn trace_of(wave: Waveform) -> Vec<f32> {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        let index = Waveform::ALL.iter().position(|w| *w == wave).unwrap();
        node.choices = vec![index];
        Lfo.trace(&node, &Live::of(&node), TRACE_SAMPLES)
            .expect("an LFO has a trace")
            .points
    }

    #[test]
    fn every_waveform_gives_points_in_range() {
        for wave in Waveform::ALL {
            let points = trace_of(wave);
            assert_eq!(points.len(), TRACE_SAMPLES);
            assert!(points.iter().all(|p| (0.0..=1.0).contains(p)));
        }
    }

    /// A sine trace peaks once per cycle, so one cycle has one peak.
    #[test]
    fn a_sine_trace_has_one_cycle() {
        let points = trace_of(Waveform::Sine);
        let peaks = points
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > 0.9)
            .count();
        assert_eq!(peaks, 1);
    }

    #[test]
    fn saw_and_triangle_differ() {
        assert_ne!(trace_of(Waveform::Saw), trace_of(Waveform::Triangle));
    }

    /// The cursor for an LFO of `wave` at `phase` with the level the engine gives that phase.
    fn cursor_of(wave: Waveform, phase: f32) -> Option<(f32, f32)> {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        node.choices = vec![Waveform::ALL.iter().position(|w| *w == wave).unwrap()];
        let level = (wave.naive(phase) + 1.0) * 0.5;
        let live = Live::of(&node).with_lfo(phase, level);
        Lfo.trace(&node, &live, TRACE_SAMPLES)
            .expect("an LFO has a trace")
            .cursor
    }

    #[test]
    fn there_is_no_cursor_without_a_live_signal() {
        let node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        let trace = Lfo.trace(&node, &Live::of(&node), TRACE_SAMPLES).unwrap();
        assert_eq!(trace.cursor, None);
    }

    /// The dot sits at the LFO's phase across the box, on the drawn wave.
    #[test]
    fn the_cursor_sits_at_the_phase_on_the_wave() {
        for wave in [
            Waveform::Sine,
            Waveform::Saw,
            Waveform::Triangle,
            Waveform::Square,
        ] {
            let (x, y) = cursor_of(wave, 0.3).expect("a cursor");
            assert!((x - 0.3).abs() < 1e-6, "{wave:?}");
            let points = trace_of(wave);
            let drawn = points[(x * (TRACE_SAMPLES - 1) as f32).round() as usize];
            assert!((drawn - y).abs() < 0.1, "{wave:?}: {drawn} vs {y}");
        }
    }

    /// A noise LFO holds a random value, which its fixed picture cannot show.
    #[test]
    fn noise_has_no_cursor() {
        assert_eq!(cursor_of(Waveform::Noise, 0.3), None);
    }

    /// The Custom trace is the node's curve, straight between its points.
    #[test]
    fn a_custom_trace_follows_the_curve() {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        node.choices = vec![CUSTOM_ROW];
        node.points = vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.25]];
        let trace = Lfo
            .trace(&node, &Live::of(&node), TRACE_SAMPLES)
            .expect("an LFO has a trace");
        assert_eq!(trace.points.len(), TRACE_SAMPLES);
        assert_eq!(trace.points[0], 0.0);
        assert_eq!(trace.points[TRACE_SAMPLES - 1], 0.25);
        let last = (TRACE_SAMPLES - 1) as f32;
        let quarter = trace.points[(0.25 * last).round() as usize];
        assert!((quarter - 0.5).abs() < 0.02, "{quarter}");
    }

    /// The dot on a custom curve sits at the LFO's phase and level, and a node with no points of
    /// its own draws the default triangle.
    #[test]
    fn a_custom_cursor_rides_the_curve() {
        let mut node = FlodeNode::new(NodeKind::Lfo, Point::new(0.0, 0.0));
        node.choices = vec![CUSTOM_ROW];
        let live = Live::of(&node).with_lfo(0.25, 0.5);
        let trace = Lfo.trace(&node, &live, TRACE_SAMPLES).unwrap();
        assert_eq!(trace.cursor, Some((0.25, 0.5)));
        let peak = trace.points[TRACE_SAMPLES / 2];
        assert!(peak > 0.98, "{peak}");
    }

    #[test]
    fn noise_is_the_same_every_time_and_not_flat() {
        let first = trace_of(Waveform::Noise);
        assert_eq!(first, trace_of(Waveform::Noise));
        assert!(first.iter().any(|p| (*p - first[0]).abs() > 0.1));
    }
}
