//! The filter node. A state-variable filter with a low, high, or band pass response.

#[cfg(test)]
use super::viewer::TRACE_SAMPLES;
use super::viewer::{sample, Live, Trace};
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::svf::{magnitude, FilterType, Slope};

pub struct Filter;

const FILTER_LABELS: [&str; 3] = ["Low pass", "High pass", "Band pass"];

impl NodeBehavior for Filter {
    fn title(&self) -> &'static str {
        "Filter"
    }

    fn rows(&self, node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::driven_knob(KnobId::Cutoff),
            Row::driven_knob(KnobId::Resonance),
        ];
        // The slope only means something for the low and high pass. It has no pin, so a saved
        // wire never depends on whether the row is showing.
        if has_slope(filter_type(node)) {
            rows.push(Row::plain_knob(KnobId::Slope));
        }
        rows.push(Row::control(Control::Dropdown(0)));
        rows.push(Row::control(Control::Viewer));
        rows
    }

    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        Some(Self::response(node, live, samples))
    }

    fn audio_in(&self) -> bool {
        true
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::PassThrough
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&FILTER_LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Type"]
    }
}

/// The response a filter node has.
pub fn filter_type(node: &FlodeNode) -> FilterType {
    FilterType::ALL[node.choice(0).min(FilterType::ALL.len() - 1)]
}

/// Whether a response has a slope to choose. A band pass is a peak, not a roll-off.
pub fn has_slope(kind: FilterType) -> bool {
    kind != FilterType::BandPass
}

/// The lowest and highest frequency the viewer shows, in Hz.
const VIEW_LOW: f32 = 20.0;
const VIEW_HIGH: f32 = 20000.0;
/// The level window the viewer shows, in dB. It maps to the bottom and top of the box.
const VIEW_DB_LOW: f32 = -36.0;
const VIEW_DB_HIGH: f32 = 24.0;
/// The editor does not know the host's sample rate, so the picture assumes 44.1 kHz.
const VIEW_SAMPLE_RATE: f32 = 44100.0;

/// A position across the box, from 0.0 to 1.0, as a frequency on the log axis.
fn frequency_at(x: f32) -> f32 {
    VIEW_LOW * (VIEW_HIGH / VIEW_LOW).powf(x)
}

/// The position across the box of a frequency on the log axis.
fn position_of(freq: f32) -> f32 {
    ((freq / VIEW_LOW).ln() / (VIEW_HIGH / VIEW_LOW).ln()).clamp(0.0, 1.0)
}

impl Filter {
    fn response(node: &FlodeNode, live: &Live, samples: usize) -> Trace {
        let cutoff = live.plain(KnobId::Cutoff);
        let resonance = live.value(KnobId::Resonance);
        let kind = filter_type(node);
        let slope = Slope::from_step(live.plain(KnobId::Slope));
        let points = sample(samples, |x| {
            let gain = magnitude(
                kind,
                slope,
                frequency_at(x),
                cutoff,
                resonance,
                VIEW_SAMPLE_RATE,
            );
            let db = 20.0 * gain.max(1e-6).log10();
            (db - VIEW_DB_LOW) / (VIEW_DB_HIGH - VIEW_DB_LOW)
        });
        Trace {
            points,
            markers: vec![position_of(cutoff)],
            marks: Vec::new(),
            fill: true,
            cursor: None,
            cursor_line: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node(kind: FilterType) -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0));
        node.choices = vec![kind.index()];
        node
    }

    fn trace_of(node: &FlodeNode) -> Trace {
        let live = Live::of(node);
        Filter
            .trace(node, &live, TRACE_SAMPLES)
            .expect("the filter has a trace")
    }

    /// The point of the trace at the cutoff marker.
    fn at_cutoff(trace: &Trace) -> f32 {
        let i = (trace.markers[0] * (trace.points.len() - 1) as f32).round() as usize;
        trace.points[i]
    }

    #[test]
    fn the_trace_has_the_asked_for_points_in_the_box() {
        let node = node(FilterType::LowPass);
        let trace = trace_of(&node);
        assert_eq!(trace.points.len(), TRACE_SAMPLES);
        assert!(trace.points.iter().all(|p| (0.0..=1.0).contains(p)));
        assert_eq!(trace.markers.len(), 1);
        assert!(trace.fill);
        assert_eq!(
            Filter
                .trace(&node, &Live::of(&node), 10)
                .unwrap()
                .points
                .len(),
            10
        );
    }

    #[test]
    fn a_low_pass_is_higher_on_the_left() {
        let trace = trace_of(&node(FilterType::LowPass));
        assert!(trace.points[0] > *trace.points.last().unwrap());
    }

    #[test]
    fn a_high_pass_is_higher_on_the_right() {
        let trace = trace_of(&node(FilterType::HighPass));
        assert!(trace.points[0] < *trace.points.last().unwrap());
    }

    #[test]
    fn a_band_pass_peaks_at_the_cutoff() {
        let trace = trace_of(&node(FilterType::BandPass));
        let top = at_cutoff(&trace);
        assert!(top >= trace.points.iter().copied().fold(0.0, f32::max) - 0.01);
        assert!(top > trace.points[0] && top > *trace.points.last().unwrap());
    }

    /// The low and high pass peak higher as resonance rises. The band pass keeps its peak at unity
    /// and gets narrower, so its point at the cutoff stays put and the skirts fall.
    #[test]
    fn resonance_raises_the_cutoff_point() {
        for kind in [FilterType::LowPass, FilterType::HighPass] {
            let mut node = node(kind);
            node.set_value(KnobId::Resonance, 0.1);
            let low = at_cutoff(&trace_of(&node));
            node.set_value(KnobId::Resonance, 0.9);
            let high = at_cutoff(&trace_of(&node));
            assert!(high > low, "{kind:?}: {low} then {high}");
        }
    }

    #[test]
    fn resonance_narrows_a_band_pass() {
        let mut node = node(FilterType::BandPass);
        node.set_value(KnobId::Resonance, 0.1);
        let wide = trace_of(&node);
        node.set_value(KnobId::Resonance, 0.9);
        let narrow = trace_of(&node);
        assert!((at_cutoff(&wide) - at_cutoff(&narrow)).abs() < 0.02);
        let middle = wide.points.len() / 2;
        assert!(narrow.points[middle] < wide.points[middle]);
    }

    #[test]
    fn a_driven_cutoff_moves_the_marker() {
        let node = node(FilterType::LowPass);
        let own = trace_of(&node).markers[0];
        let live = Live::new(&node, |id| (id == KnobId::Cutoff).then_some(0.9));
        let driven = Filter.trace(&node, &live, TRACE_SAMPLES).unwrap();
        assert!(driven.markers[0] > own);
        assert_ne!(driven.points, trace_of(&node).points);
    }

    /// The slope row shows for the low and high pass and not the band pass, and it has no pin.
    #[test]
    fn the_slope_shows_only_for_low_and_high_pass() {
        let slope_row = |kind| {
            Filter
                .rows(&node(kind))
                .into_iter()
                .find(|row| row.knob == Some(KnobId::Slope))
        };
        assert!(slope_row(FilterType::LowPass).is_some());
        assert!(slope_row(FilterType::HighPass).is_some());
        assert!(slope_row(FilterType::BandPass).is_none());
        assert!(slope_row(FilterType::LowPass).unwrap().input.is_none());
    }

    /// A steeper slope falls further past the cutoff in the viewer.
    #[test]
    fn a_steeper_slope_draws_lower_above_the_cutoff() {
        let mut node = node(FilterType::LowPass);
        let last = |node: &FlodeNode| *trace_of(node).points.last().unwrap();
        node.set_value(KnobId::Slope, KnobId::Slope.to_normalized(0.0));
        let gentle = last(&node);
        node.set_value(KnobId::Slope, KnobId::Slope.to_normalized(3.0));
        assert!(last(&node) < gentle);
    }

    /// The slope knob steps through four values and starts at 12 dB.
    #[test]
    fn the_slope_knob_has_four_steps_and_starts_at_twelve() {
        assert_eq!(
            KnobId::Slope.format(KnobId::Slope.to_normalized(0.0)),
            "6 dB/oct"
        );
        assert_eq!(
            KnobId::Slope.format(KnobId::Slope.to_normalized(3.0)),
            "24 dB/oct"
        );
        let default = KnobId::Slope.default_normalized();
        assert_eq!(KnobId::Slope.format(default), "12 dB/oct");
        assert_eq!(KnobId::Slope.to_plain(KnobId::Slope.snap(0.4)).fract(), 0.0);
    }

    #[test]
    fn the_viewer_is_the_last_row() {
        let rows = Filter.rows(&node(FilterType::LowPass));
        assert_eq!(rows.last().unwrap().control, Some(Control::Viewer));
        assert!(rows[0].input.is_some() && rows[0].output.is_some());
    }
}
