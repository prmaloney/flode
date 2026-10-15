//! The envelope node. It runs an attack, decay, sustain, and release for every voice, and sends
//! that level out as a control signal on its shape pin. It has no audio pins. To shape a sound,
//! drive an oscillator's gain from it.

use super::viewer::{sample, Live, Trace};
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::envelope::EnvPhase;

pub struct Envelope;

/// The share of the box width the sustain hold takes. It is fixed, since a held note has no length.
const HOLD_SHARE: f32 = 0.2;

/// The least share of the box width a timed stage gets, so a 1 ms attack is still a visible slope.
const MIN_SHARE: f32 = 0.08;

/// Where each stage ends across the box, from 0.0 to 1.0, for stage times in seconds.
///
/// The sustain hold takes `HOLD_SHARE`. The three timed stages split the rest. Each gets
/// `MIN_SHARE`, and what is left after those minimums goes to the stages in proportion to their
/// times. The result is the ends of attack, decay, and the hold. Release runs from the last one to
/// the right edge.
fn stage_ends(attack: f32, decay: f32, release: f32) -> [f32; 3] {
    let timed = 1.0 - HOLD_SHARE;
    let spare = timed - 3.0 * MIN_SHARE;
    let total = (attack + decay + release).max(f32::EPSILON);
    let share = |t: f32| MIN_SHARE + spare * t / total;
    let attack_end = share(attack);
    let decay_end = attack_end + share(decay);
    [attack_end, decay_end, decay_end + HOLD_SHARE]
}

/// The level at position `x` across the box. The curves match `dsp::envelope`: a linear climb to
/// full, a linear fall to the sustain level, a hold, then a linear fall to zero.
fn level_at(x: f32, ends: [f32; 3], sustain: f32) -> f32 {
    let [attack_end, decay_end, hold_end] = ends;
    if x < attack_end {
        x / attack_end
    } else if x < decay_end {
        1.0 - (1.0 - sustain) * (x - attack_end) / (decay_end - attack_end)
    } else if x < hold_end {
        sustain
    } else {
        sustain * (1.0 - (x - hold_end) / (1.0 - hold_end))
    }
}

/// Where the live envelope sits on the trace, as a position across the box and a height.
///
/// A level alone does not say which stage a voice is in, since the climb and the release pass the
/// same levels. So the stage picks the segment and the level picks the place along it. A held
/// note has no length, so the sustain puts the dot in the middle of the hold. A release starts
/// from the sustain level, so a note let go earlier shows at the start of the release.
fn cursor_at(phase: EnvPhase, level: f32, ends: [f32; 3], sustain: f32) -> (f32, f32) {
    let [attack_end, decay_end, hold_end] = ends;
    let level = level.clamp(0.0, 1.0);
    let x = match phase {
        EnvPhase::Attack => level * attack_end,
        EnvPhase::Decay => {
            let fallen = if sustain < 1.0 {
                (1.0 - level) / (1.0 - sustain)
            } else {
                1.0
            };
            attack_end + fallen.clamp(0.0, 1.0) * (decay_end - attack_end)
        }
        EnvPhase::Sustain => (decay_end + hold_end) / 2.0,
        EnvPhase::Release => {
            let fallen = if sustain > 0.0 {
                1.0 - level / sustain
            } else {
                1.0
            };
            hold_end + fallen.clamp(0.0, 1.0) * (1.0 - hold_end)
        }
    };
    (x, level)
}

impl NodeBehavior for Envelope {
    fn title(&self) -> &'static str {
        "Envelope"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            Row::control(Control::Viewer),
            Row::driven_knob(KnobId::Attack).with_output(Port::Shape),
            Row::driven_knob(KnobId::Decay),
            Row::driven_knob(KnobId::Sustain),
            Row::driven_knob(KnobId::Release),
        ]
    }

    fn trace(&self, _node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let sustain = live.value(KnobId::Sustain).clamp(0.0, 1.0);
        let ends = stage_ends(
            live.plain(KnobId::Attack),
            live.plain(KnobId::Decay),
            live.plain(KnobId::Release),
        );
        Some(Trace {
            points: sample(samples, |x| level_at(x, ends, sustain)),
            markers: ends.to_vec(),
            fill: true,
            cursor: live
                .envelope()
                .map(|(phase, level)| cursor_at(phase, level, ends, sustain)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::viewer::TRACE_SAMPLES;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Envelope, Point::new(0.0, 0.0))
    }

    /// A node with a 100 ms attack, 200 ms decay, half sustain, and 100 ms release.
    fn shaped() -> FlodeNode {
        let mut node = node();
        for (id, plain) in [
            (KnobId::Attack, 0.1),
            (KnobId::Decay, 0.2),
            (KnobId::Sustain, 0.5),
            (KnobId::Release, 0.1),
        ] {
            node.set_value(id, id.to_normalized(plain));
        }
        node
    }

    fn trace_of(node: &FlodeNode) -> Trace {
        Envelope
            .trace(node, &Live::of(node), TRACE_SAMPLES)
            .expect("the envelope has a trace")
    }

    #[test]
    fn the_envelope_has_a_viewer_and_a_trace() {
        let node = node();
        assert!(node.has_viewer());
        assert!(node.trace(&Live::of(&node)).is_some());
    }

    #[test]
    fn the_trace_has_the_requested_points_in_range() {
        let trace = trace_of(&shaped());
        assert_eq!(trace.points.len(), TRACE_SAMPLES);
        assert!(trace.points.iter().all(|p| (0.0..=1.0).contains(p)));
        assert!(trace.fill);
    }

    #[test]
    fn the_trace_climbs_peaks_holds_and_falls() {
        let trace = trace_of(&shaped());
        let last = (TRACE_SAMPLES - 1) as f32;
        let at = |x: f32| trace.points[(x * last).round() as usize];
        assert_eq!(trace.points[0], 0.0);
        let (attack_end, decay_end, hold_end) =
            (trace.markers[0], trace.markers[1], trace.markers[2]);
        assert!((at(attack_end) - 1.0).abs() < 0.05);
        let mid_hold = (decay_end + hold_end) / 2.0;
        assert!((at(mid_hold) - 0.5).abs() < 1e-3);
        assert!(trace.points[TRACE_SAMPLES - 1] < 0.01);
    }

    #[test]
    fn markers_are_three_and_increasing() {
        let trace = trace_of(&shaped());
        assert_eq!(trace.markers.len(), 3);
        assert!(trace.markers.windows(2).all(|w| w[0] < w[1]));
        assert!(trace.markers.iter().all(|m| (0.0..1.0).contains(m)));
    }

    #[test]
    fn a_very_short_attack_still_gets_a_slope() {
        let mut node = node();
        node.set_value(KnobId::Attack, 0.0);
        let trace = trace_of(&node);
        assert!(trace.markers[0] >= MIN_SHARE - 1e-6);
    }

    #[test]
    fn changing_sustain_changes_the_trace() {
        let base = shaped();
        let mut lower = shaped();
        lower.set_value(KnobId::Sustain, 0.2);
        assert_ne!(trace_of(&base), trace_of(&lower));
    }

    /// The cursor's trace with the envelope in `phase` at `level`.
    fn cursor_in(phase: EnvPhase, level: f32) -> (f32, f32) {
        let node = shaped();
        let live = Live::of(&node).with_envelope(phase, level);
        Envelope
            .trace(&node, &live, TRACE_SAMPLES)
            .and_then(|trace| trace.cursor)
            .expect("the envelope has a cursor")
    }

    #[test]
    fn there_is_no_cursor_without_a_running_voice() {
        assert_eq!(trace_of(&shaped()).cursor, None);
    }

    /// The dot sits on the drawn shape in every stage, so it rides the line.
    #[test]
    fn the_cursor_rides_the_shape() {
        let node = shaped();
        let trace = trace_of(&node);
        let last = (TRACE_SAMPLES - 1) as f32;
        let drawn = |x: f32| trace.points[(x * last).round() as usize];
        for (phase, level) in [
            (EnvPhase::Attack, 0.5),
            (EnvPhase::Decay, 0.75),
            (EnvPhase::Sustain, 0.5),
            (EnvPhase::Release, 0.25),
        ] {
            let (x, y) = cursor_in(phase, level);
            assert!((y - level).abs() < 1e-6);
            assert!(
                (drawn(x) - y).abs() < 0.03,
                "{phase:?}: {} vs {y}",
                drawn(x)
            );
        }
    }

    /// The same level is in the climb and the release, and the stage says which one.
    #[test]
    fn the_stage_tells_the_climb_from_the_release() {
        let (climbing, _) = cursor_in(EnvPhase::Attack, 0.25);
        let (releasing, _) = cursor_in(EnvPhase::Release, 0.25);
        assert!(climbing < releasing);
    }

    /// The dot moves left to right through the stages in order.
    #[test]
    fn the_cursor_moves_forward_through_the_stages() {
        let xs: Vec<f32> = [
            (EnvPhase::Attack, 0.1),
            (EnvPhase::Attack, 0.9),
            (EnvPhase::Decay, 0.9),
            (EnvPhase::Decay, 0.6),
            (EnvPhase::Sustain, 0.5),
            (EnvPhase::Release, 0.4),
            (EnvPhase::Release, 0.1),
        ]
        .map(|(phase, level)| cursor_in(phase, level).0)
        .to_vec();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    }

    #[test]
    fn a_driven_value_changes_the_trace() {
        let node = shaped();
        let driven = Live::new(&node, |id| (id == KnobId::Sustain).then_some(0.1));
        let live = Envelope
            .trace(&node, &driven, TRACE_SAMPLES)
            .expect("the envelope has a trace");
        assert_ne!(live, trace_of(&node));
    }
}
