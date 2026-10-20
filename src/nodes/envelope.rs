//! The envelope node. It runs an attack, decay, sustain, and release for every voice, and sends
//! that level out as a control signal on its shape pin. It has no audio pins. To shape a sound,
//! drive an oscillator's gain from it.

use super::viewer::{sample, Live, Trace};
use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};
use crate::dsp::envelope::EnvPhase;

pub struct Envelope;

/// The share of the box width the sustain hold takes. It is fixed, since a held note has no length.
const HOLD_SHARE: f32 = 0.2;

/// The least share of the box width a timed stage gets, at the bottom of its knob. It is a sliver,
/// so the shortest attack or release draws as a nearly vertical line, and not zero, so the stages
/// and their handles stay in order.
const MIN_SHARE: f32 = 0.01;

/// The most share of the box width a timed stage gets, at the top of its knob. Three stages at
/// this width and the hold fill the box, so the curve never passes the right edge.
const SHARE_MAX: f32 = (1.0 - HOLD_SHARE) / 3.0;

/// The width a stage takes across the box, from 0.0 to 1.0, for its knob's normalized value.
fn stage_width(normalized: f32) -> f32 {
    MIN_SHARE + (SHARE_MAX - MIN_SHARE) * normalized.clamp(0.0, 1.0)
}

/// Where each stage ends across the box, from 0.0 to 1.0, for the normalized values of the Attack,
/// Decay, and Release knobs.
///
/// Each timed stage has a width that depends on its own knob and no other, so the time axis is
/// fixed and moving one knob moves only the stages after it. The sustain hold takes `HOLD_SHARE`.
/// The result is the ends of attack, decay, the hold, and release. The curve is flat at zero from
/// the release end to the right edge.
fn stage_ends(attack: f32, decay: f32, release: f32) -> [f32; 4] {
    let attack_end = stage_width(attack);
    let decay_end = attack_end + stage_width(decay);
    let hold_end = decay_end + HOLD_SHARE;
    [
        attack_end,
        decay_end,
        hold_end,
        hold_end + stage_width(release),
    ]
}

/// The stage ends for the knobs as they are now, wires included.
pub fn ends_of(live: &Live) -> [f32; 4] {
    stage_ends(
        live.value(KnobId::Attack),
        live.value(KnobId::Decay),
        live.value(KnobId::Release),
    )
}

/// The knob value for a stage that is `width` wide, the inverse of `stage_width`. A width past
/// either limit gives the nearest end of the knob.
fn value_for_width(width: f32) -> f32 {
    ((width - MIN_SHARE) / (SHARE_MAX - MIN_SHARE)).clamp(0.0, 1.0)
}

/// The normalized Attack that ends the attack at `x` across the box.
pub fn attack_at(x: f32) -> f32 {
    value_for_width(x)
}

/// The normalized Decay that ends the decay at `x`, when the attack ends at `attack_end`.
pub fn decay_at(x: f32, attack_end: f32) -> f32 {
    value_for_width(x - attack_end)
}

/// The normalized Release that ends the release at `x`, when the hold ends at `hold_end`.
pub fn release_at(x: f32, hold_end: f32) -> f32 {
    value_for_width(x - hold_end)
}

/// The knobs behind each handle. The decay handle sets two, and the others set one.
pub const HANDLE_KNOBS: [&[KnobId]; 3] = [
    &[KnobId::Attack],
    &[KnobId::Decay, KnobId::Sustain],
    &[KnobId::Release],
];

/// Where the three handles of the curve sit in the unit box, as (x across, y up from the bottom).
/// They are the peak at the end of the attack, the sustain level at the end of the decay, and the
/// end of the release. They read `live`, so a knob that a wire drives moves its handle.
pub fn handles(live: &Live) -> [(f32, f32); 3] {
    let [attack_end, decay_end, _, release_end] = ends_of(live);
    [
        (attack_end, 1.0),
        (decay_end, live.value(KnobId::Sustain).clamp(0.0, 1.0)),
        (release_end, 0.0),
    ]
}

/// The level at position `x` across the box. The curves match `dsp::envelope`: a linear climb to
/// full, a linear fall to the sustain level, a hold, a linear fall to zero, then silence.
fn level_at(x: f32, ends: [f32; 4], sustain: f32) -> f32 {
    let [attack_end, decay_end, hold_end, release_end] = ends;
    if x < attack_end {
        x / attack_end
    } else if x < decay_end {
        1.0 - (1.0 - sustain) * (x - attack_end) / (decay_end - attack_end)
    } else if x < hold_end {
        sustain
    } else if x < release_end {
        sustain * (1.0 - (x - hold_end) / (release_end - hold_end))
    } else {
        0.0
    }
}

/// Where the live envelope sits on the trace, as a position across the box and a height.
///
/// A level alone does not say which stage a voice is in, since the climb and the release pass the
/// same levels. So the stage picks the segment and the level picks the place along it. A held
/// note has no length, so the sustain puts the dot in the middle of the hold.
///
/// The drawn release starts from the sustain level. A note let go earlier releases from a higher
/// level, so while its level is above the sustain the dot waits at the start of the release,
/// then follows the drawn line down. That keeps the dot on the curve.
fn cursor_at(phase: EnvPhase, level: f32, ends: [f32; 4], sustain: f32) -> (f32, f32) {
    let [attack_end, decay_end, hold_end, release_end] = ends;
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
            hold_end + fallen.clamp(0.0, 1.0) * (release_end - hold_end)
        }
    };
    let y = match phase {
        EnvPhase::Release => level.min(sustain),
        _ => level,
    };
    (x, y)
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
        let ends = ends_of(live);
        Some(Trace {
            points: sample(samples, |x| level_at(x, ends, sustain)),
            markers: ends[..3].to_vec(),
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
    fn a_very_short_attack_is_nearly_vertical() {
        let mut node = node();
        node.set_value(KnobId::Attack, 0.0);
        let trace = trace_of(&node);
        assert!(trace.markers[0] >= MIN_SHARE - 1e-6);
        assert!(trace.markers[0] < 0.03, "{}", trace.markers[0]);
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

    /// A note let go above the sustain level shows on the drawn curve, at the start of the release,
    /// and not above it. It follows the line once its level is under the sustain.
    #[test]
    fn a_release_from_above_the_sustain_stays_on_the_curve() {
        let node = shaped();
        let trace = trace_of(&node);
        let last = (TRACE_SAMPLES - 1) as f32;
        let drawn = |x: f32| trace.points[(x * last).round() as usize];
        let hold_end = trace.markers[2];
        for level in [1.0, 0.8, 0.6, 0.5] {
            let (x, y) = cursor_in(EnvPhase::Release, level);
            assert!((x - hold_end).abs() < 1e-6, "{level}");
            assert!((y - 0.5).abs() < 1e-6, "{level}");
            assert!((drawn(x) - y).abs() < 0.03, "{level}");
        }
        let (x, y) = cursor_in(EnvPhase::Release, 0.25);
        assert!(x > hold_end && y < 0.5);
        assert!((drawn(x) - y).abs() < 0.03);
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

    /// A knob's value at the bottom, middle, and top gives the narrowest, a middle, and the widest
    /// stage, and the curve ends inside the box even with every knob at the top.
    #[test]
    fn the_curve_stays_inside_the_box() {
        let [attack_end, .., release_end] = stage_ends(0.0, 0.0, 0.0);
        assert!((attack_end - MIN_SHARE).abs() < 1e-6);
        assert!(release_end < 1.0);
        let full = stage_ends(1.0, 1.0, 1.0);
        assert!((full[3] - 1.0).abs() < 1e-5);
        assert!(full[3] <= 1.0 + 1e-6);
    }

    /// Each stage's width follows its own knob, so moving one knob leaves the other widths alone.
    #[test]
    fn a_stage_width_ignores_the_other_knobs() {
        let width = |ends: [f32; 4]| {
            [
                ends[0],
                ends[1] - ends[0],
                ends[2] - ends[1],
                ends[3] - ends[2],
            ]
        };
        let base = width(stage_ends(0.3, 0.5, 0.7));
        let attack = width(stage_ends(0.9, 0.5, 0.7));
        assert!(attack[0] > base[0]);
        let same = |a: [f32; 4], b: [f32; 4], skip: usize| {
            (0..4).all(|i| i == skip || (a[i] - b[i]).abs() < 1e-5)
        };
        assert!(same(attack, base, 0));
        let decay = width(stage_ends(0.3, 0.1, 0.7));
        assert!(decay[1] < base[1]);
        assert!(same(decay, base, 1));
        let release = width(stage_ends(0.3, 0.5, 0.0));
        assert!(release[3] < base[3]);
        assert!(same(release, base, 3));
    }

    #[test]
    fn the_inverse_maps_undo_the_stage_ends() {
        for n in [0.0, 0.1, 0.5, 0.9, 1.0] {
            let [attack_end, ..] = stage_ends(n, 0.4, 0.6);
            assert!((attack_at(attack_end) - n).abs() < 1e-5);
            let [attack_end, decay_end, ..] = stage_ends(0.25, n, 0.6);
            assert!((decay_at(decay_end, attack_end) - n).abs() < 1e-5);
            let [_, _, hold_end, release_end] = stage_ends(0.25, 0.4, n);
            assert!((release_at(release_end, hold_end) - n).abs() < 1e-5);
        }
    }

    #[test]
    fn the_inverse_maps_clamp_at_both_ends() {
        assert_eq!(attack_at(-1.0), 0.0);
        assert_eq!(attack_at(0.0), 0.0);
        assert_eq!(attack_at(5.0), 1.0);
        assert_eq!(decay_at(0.1, 0.3), 0.0);
        assert_eq!(decay_at(9.0, 0.3), 1.0);
        assert_eq!(release_at(0.2, 0.6), 0.0);
        assert_eq!(release_at(9.0, 0.6), 1.0);
    }

    /// The curve is flat at zero from the release end to the right edge.
    #[test]
    fn the_curve_is_silent_after_the_release() {
        let mut node = node();
        node.set_value(KnobId::Release, 0.0);
        let trace = trace_of(&node);
        let release_end = handles(&Live::of(&node))[2].0;
        assert!(release_end < 0.95);
        let last = (TRACE_SAMPLES - 1) as f32;
        let from = (release_end * last).ceil() as usize;
        assert!(trace.points[from..].iter().all(|p| *p == 0.0));
    }

    #[test]
    fn the_handles_sit_on_the_curve() {
        let node = shaped();
        let live = Live::of(&node);
        let [attack, decay, release] = handles(&live);
        let [attack_end, decay_end, _, release_end] = ends_of(&live);
        assert_eq!(attack, (attack_end, 1.0));
        assert_eq!(decay, (decay_end, 0.5));
        assert_eq!(release, (release_end, 0.0));
        let ends = ends_of(&live);
        assert!((level_at(attack.0 - 1e-4, ends, 0.5) - 1.0).abs() < 0.01);
        assert!((level_at(decay.0 + 1e-4, ends, 0.5) - 0.5).abs() < 0.01);
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
