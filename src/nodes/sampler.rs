//! The Sampler node. It plays back a loaded WAV file per voice, pitched from the note relative to
//! a root key, with the same one-voice-per-note model as the oscillator.

use super::viewer::{Live, Mark, Trace};
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Sampler;

/// The Loop dropdown's rows.
pub const LOOP_LABELS: [&str; 3] = ["Off", "Forward", "Ping-pong"];

/// What the Loop dropdown says, read from a node's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    Forward,
    PingPong,
}

impl LoopMode {
    pub const ALL: [LoopMode; 3] = [LoopMode::Off, LoopMode::Forward, LoopMode::PingPong];

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(LoopMode::Off)
    }
}

/// The Loop mode a Sampler node is set to.
pub fn loop_mode(node: &FlodeNode) -> LoopMode {
    LoopMode::from_index(node.choice(0))
}

/// True while pitch tracking is on: the Sampler plays faster or slower as the note moves away
/// from the root key. Off plays every note at the file's own recorded speed.
pub fn pitch_tracking(node: &FlodeNode) -> bool {
    node.switch(0)
}

/// True for one shot: the Sampler ignores note off and plays through to the end, or loops
/// forever. False (the default) is gated: the envelope stops it on release, like an oscillator.
pub fn one_shot(node: &FlodeNode) -> bool {
    node.switch(1)
}

/// True to play the sample backward.
pub fn reverse(node: &FlodeNode) -> bool {
    node.switch(2)
}

impl NodeBehavior for Sampler {
    fn title(&self) -> &'static str {
        "Sampler"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            // The notes pin sits on the load button's row, at the very top, the same place the
            // oscillator keeps its notes pin.
            Row {
                input: Some(Port::Notes),
                ..Row::control(Control::LoadSample)
            },
            Row::control(Control::Viewer),
            // The envelope input shapes this voice's amplitude, one copy per voice.
            Row::input(Port::Shape).with_output(Port::Audio),
            Row::driven_knob(KnobId::OscGain),
            Row::driven_knob(KnobId::Pan),
            Row::driven_knob(KnobId::SamplerRoot),
            Row::control(Control::Switch(0)),
            Row::driven_knob(KnobId::SamplerStart),
            Row::control(Control::Switch(2)),
            Row::control(Control::Switch(1)),
            Row::control(Control::Dropdown(0)),
            Row::driven_knob(KnobId::SamplerLoopStart),
            Row::driven_knob(KnobId::SamplerLoopEnd),
        ]
    }

    fn audio_out(&self) -> bool {
        true
    }

    fn bypass(&self) -> Bypass {
        Bypass::Mute
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Pitch track", "One shot", "Reverse"]
    }

    /// Pitch tracking starts on, which is what most loaded samples want.
    fn default_switches(&self) -> &'static [usize] {
        &[0]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Loop"]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&LOOP_LABELS]
    }

    /// The loaded waveform, downsampled to fit the box, as the peak magnitude in each bucket. A
    /// node with nothing loaded draws a flat line at the middle. The Start offset and the two loop
    /// points always show, each in its own color, and move live as their knobs move.
    fn trace(&self, node: &FlodeNode, live: &Live, samples: usize) -> Option<Trace> {
        let marks = vec![
            (live.plain(KnobId::SamplerStart) / 100.0, Mark::Start),
            (
                live.plain(KnobId::SamplerLoopStart) / 100.0,
                Mark::LoopStart,
            ),
            (live.plain(KnobId::SamplerLoopEnd) / 100.0, Mark::LoopEnd),
        ];
        let cursor = live
            .sampler()
            .map(|position| (position.clamp(0.0, 1.0), 0.5));
        let Some(data) = &node.sample else {
            return Some(Trace {
                points: vec![0.5; samples],
                marks,
                cursor,
                cursor_line: true,
                ..Trace::default()
            });
        };
        let len = data.len().max(1);
        let points = (0..samples)
            .map(|i| {
                let start = i * len / samples;
                let end = (((i + 1) * len / samples).max(start + 1)).min(len);
                let peak = (start..end)
                    .map(|n| {
                        let (l, r) = data.frame(n);
                        l.abs().max(r.abs())
                    })
                    .fold(0.0f32, f32::max);
                (0.5 + peak * 0.5).clamp(0.0, 1.0)
            })
            .collect();
        Some(Trace {
            points,
            marks,
            fill: true,
            cursor,
            cursor_line: true,
            ..Trace::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::sample::SampleData;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;
    use std::sync::Arc;

    fn sampler() -> FlodeNode {
        FlodeNode::new(NodeKind::Sampler, Point::new(0.0, 0.0))
    }

    #[test]
    fn it_has_a_notes_input_and_an_audio_output() {
        let node = sampler();
        assert!(node.input_ports().contains(&Port::Notes));
        assert!(Sampler.audio_out());
    }

    #[test]
    fn pitch_tracking_starts_on() {
        assert!(pitch_tracking(&sampler()));
    }

    #[test]
    fn gated_and_forward_facing_is_the_default() {
        let node = sampler();
        assert!(!one_shot(&node));
        assert!(!reverse(&node));
        assert_eq!(loop_mode(&node), LoopMode::Off);
    }

    #[test]
    fn the_loop_dropdown_follows_the_choice() {
        let mut node = sampler();
        node.choose(0, 1);
        assert_eq!(loop_mode(&node), LoopMode::Forward);
        node.choose(0, 2);
        assert_eq!(loop_mode(&node), LoopMode::PingPong);
    }

    /// With nothing loaded the viewer still draws, as a flat line.
    #[test]
    fn an_empty_sampler_draws_a_flat_trace() {
        let node = sampler();
        let trace = Sampler.trace(&node, &Live::of(&node), 32).unwrap();
        assert_eq!(trace.points, vec![0.5; 32]);
    }

    /// A loaded sample's peaks show up in the trace.
    #[test]
    fn a_loaded_sample_traces_its_peaks() {
        let mut node = sampler();
        node.sample = Some(Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.0; 50].into_iter().chain(vec![1.0; 50]).collect(),
        }));
        let trace = Sampler.trace(&node, &Live::of(&node), 10).unwrap();
        assert!(trace.points[0] < 0.6);
        assert!(trace.points[9] > 0.9);
    }

    /// The viewer always marks Start, Loop start, and Loop end, each with its own `Mark` kind, and
    /// a node with nothing loaded still shows them over its flat line.
    #[test]
    fn the_trace_marks_start_and_the_loop_points() {
        let node = sampler();
        let trace = Sampler.trace(&node, &Live::of(&node), 16).unwrap();
        assert_eq!(trace.marks.len(), 3);
        assert_eq!(trace.marks[0].1, Mark::Start);
        assert_eq!(trace.marks[1].1, Mark::LoopStart);
        assert_eq!(trace.marks[2].1, Mark::LoopEnd);
        // Defaults: Start and Loop start at 0%, Loop end at 100%.
        assert!((trace.marks[0].0 - 0.0).abs() < 1e-4);
        assert!((trace.marks[1].0 - 0.0).abs() < 1e-4);
        assert!((trace.marks[2].0 - 1.0).abs() < 1e-4);
    }

    /// Moving the knobs moves the marks, and a value driven by a control wire moves them too,
    /// without touching the node's own stored value.
    #[test]
    fn the_marks_follow_live_knob_values() {
        let mut node = sampler();
        node.set_value(
            KnobId::SamplerStart,
            KnobId::SamplerStart.to_normalized(25.0),
        );
        let own = Sampler.trace(&node, &Live::of(&node), 16).unwrap();
        assert!((own.marks[0].0 - 0.25).abs() < 1e-3);

        let driven = Live::new(&node, |id| {
            (id == KnobId::SamplerLoopEnd).then_some(KnobId::SamplerLoopEnd.to_normalized(50.0))
        });
        let moved = Sampler.trace(&node, &driven, 16).unwrap();
        assert!((moved.marks[2].0 - 0.5).abs() < 1e-3);
    }

    /// With nothing playing, there is no cursor, loaded or not.
    #[test]
    fn there_is_no_cursor_without_a_live_voice() {
        assert_eq!(
            Sampler
                .trace(&sampler(), &Live::of(&sampler()), 16)
                .unwrap()
                .cursor,
            None
        );
        let mut node = sampler();
        node.sample = Some(Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.0; 100],
        }));
        assert_eq!(
            Sampler.trace(&node, &Live::of(&node), 16).unwrap().cursor,
            None
        );
    }

    /// A live voice's position sits across the box at its fraction through the file, vertically
    /// centered since the viewer is a waveform, not an amplitude trace. It draws as a line, not a
    /// dot, since a dot would sit lost in the middle of the shape.
    #[test]
    fn the_cursor_sits_at_the_live_fraction_and_draws_as_a_line() {
        let node = sampler();
        let live = Live::of(&node).with_sampler(0.4);
        let trace = Sampler.trace(&node, &live, 16).unwrap();
        assert_eq!(trace.cursor, Some((0.4, 0.5)));
        assert!(trace.cursor_line);
    }

    /// The cursor clamps into the box even if the engine ever reported a position outside it.
    #[test]
    fn the_cursor_clamps_to_the_box() {
        let node = sampler();
        let live = Live::of(&node).with_sampler(1.5);
        let trace = Sampler.trace(&node, &live, 16).unwrap();
        assert_eq!(trace.cursor, Some((1.0, 0.5)));
    }
}
