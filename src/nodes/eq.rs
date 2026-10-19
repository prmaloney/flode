//! The EQ node. A graph of four bands sits under the audio pins. Dragging a point in it sets that
//! band's frequency and gain. Each band also has a section with its own Freq, Gain, and Q knobs,
//! and the knobs and the point are two views of the same values. Any of the knobs can be driven by
//! a control wire, and the point follows it. A toggle in each section's header changes what the
//! band does: the two outer bands are a shelf or a cut, and the two inner bands are a peak or a
//! notch. The toggles are the node's switches, one per band.

use super::viewer::Live;
use super::{Bypass, Control, FlodeNode, KnobId, NodeBehavior, Port, Row, Section, EQ_WIDTH};
use crate::dsp::effects::eq::{EqBand, EqKind, EqParams, BANDS, MAX_GAIN_DB};

pub struct Eq;

/// The knobs of each band, from the lowest band to the highest.
pub const FREQ: [KnobId; BANDS] = [
    KnobId::EqFreq1,
    KnobId::EqFreq2,
    KnobId::EqFreq3,
    KnobId::EqFreq4,
];
pub const GAIN: [KnobId; BANDS] = [
    KnobId::EqGain1,
    KnobId::EqGain2,
    KnobId::EqGain3,
    KnobId::EqGain4,
];
pub const Q: [KnobId; BANDS] = [KnobId::EqQ1, KnobId::EqQ2, KnobId::EqQ3, KnobId::EqQ4];

/// What each band does with its switch off and on.
const KINDS: [[EqKind; 2]; BANDS] = [
    [EqKind::LowShelf, EqKind::HighPass],
    [EqKind::Peak, EqKind::Notch],
    [EqKind::Peak, EqKind::Notch],
    [EqKind::HighShelf, EqKind::LowPass],
];

impl NodeBehavior for Eq {
    fn title(&self) -> &'static str {
        "EQ"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        let mut rows = vec![
            Row::input(Port::Audio).with_output(Port::Audio),
            Row::control(Control::Eq),
            Row::control(Control::Switch(0)),
        ];
        // Each band's knobs fold under its header, so the node stays short. A folded band keeps
        // its pins, so a wire can still drive it.
        for band in 0..BANDS {
            rows.push(Row::section(band));
            rows.push(Row::driven_knob(FREQ[band]));
            rows.push(Row::driven_knob(GAIN[band]));
            rows.push(Row::driven_knob(Q[band]));
        }
        rows
    }

    fn switch_labels(&self) -> &'static [&'static str] {
        &["Analyzer"]
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: "Band 1",
                collapsed: true,
            },
            Section {
                title: "Band 2",
                collapsed: true,
            },
            Section {
                title: "Band 3",
                collapsed: true,
            },
            Section {
                title: "Band 4",
                collapsed: true,
            },
        ]
    }

    fn width(&self) -> f32 {
        EQ_WIDTH
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
}

/// The icon on band `band`'s toggle, as a line of points in a unit box. The x runs left to right
/// and the y runs top to bottom, so a boost is high up in the box. It shows what the band does
/// now: a shelf, a cut, a peak, or a notch. The high band's shelf and cut face the other way from
/// the low band's.
pub fn toggle_icon(node: &FlodeNode, band: usize) -> Vec<(f32, f32)> {
    const STEPS: usize = 16;
    let bump = |x: f32, width: f32| (-((x - 0.5) / width).powi(2)).exp();
    // A smooth step from 0.0 to 1.0 across the middle of the box.
    let step = |x: f32| {
        let t = ((x - 0.25) / 0.5).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    // A high pass: low at the left, climbing to a flat top.
    let climb = |x: f32| 0.9 - 0.7 * (x / 0.6).clamp(0.0, 1.0).powf(0.6);
    let shape = |x: f32| match kind(node, band) {
        EqKind::Peak => 0.8 - 0.6 * bump(x, 0.22),
        EqKind::Notch => 0.2 + 0.6 * bump(x, 0.1),
        EqKind::LowShelf => 0.2 + 0.6 * step(x),
        EqKind::HighShelf => 0.2 + 0.6 * step(1.0 - x),
        EqKind::HighPass => climb(x),
        EqKind::LowPass => climb(1.0 - x),
    };
    (0..=STEPS)
        .map(|i| {
            let x = i as f32 / STEPS as f32;
            (x, shape(x))
        })
        .collect()
}

/// The switch that flips band `band`. Switch 0 is the analyzer.
pub const fn band_switch(band: usize) -> usize {
    band + 1
}

/// True if the node measures its output for the graph.
pub fn analyzer(node: &FlodeNode) -> bool {
    node.switch(0)
}

/// What band `band` does right now.
pub fn kind(node: &FlodeNode, band: usize) -> EqKind {
    KINDS[band][usize::from(node.switch(band_switch(band)))]
}

/// The settings of every band, reading each knob through `live` so a Q that a wire drives counts.
pub fn params(node: &FlodeNode, live: &Live) -> EqParams {
    EqParams {
        analyzer: analyzer(node),
        bands: std::array::from_fn(|band| EqBand {
            kind: kind(node, band),
            freq_hz: live.plain(FREQ[band]),
            gain_db: live.plain(GAIN[band]),
            q: live.plain(Q[band]),
        }),
    }
}

/// The graph's frequency axis, in Hz and in the editor's picture. The editor does not know the
/// host's sample rate, so the picture assumes 44.1 kHz.
pub const VIEW_LOW: f32 = 20.0;
pub const VIEW_HIGH: f32 = 20000.0;
pub const VIEW_SAMPLE_RATE: f32 = 44100.0;
/// The dB at the bottom and top of the graph.
pub const VIEW_DB: f32 = MAX_GAIN_DB;

/// A position across the graph, from 0.0 to 1.0, as a frequency on the log axis.
pub fn frequency_at(x: f32) -> f32 {
    VIEW_LOW * (VIEW_HIGH / VIEW_LOW).powf(x.clamp(0.0, 1.0))
}

/// The position across the graph of a frequency on the log axis.
pub fn position_of(freq: f32) -> f32 {
    ((freq / VIEW_LOW).ln() / (VIEW_HIGH / VIEW_LOW).ln()).clamp(0.0, 1.0)
}

/// A position up the graph, from 0.0 at the bottom to 1.0 at the top, for a level in dB.
pub fn height_of(db: f32) -> f32 {
    ((db + VIEW_DB) / (2.0 * VIEW_DB)).clamp(0.0, 1.0)
}

/// The level in dB at a position up the graph.
pub fn db_at(y: f32) -> f32 {
    (y.clamp(0.0, 1.0) * 2.0 - 1.0) * VIEW_DB
}

/// Where band `band`'s point sits in the graph, as a position across and a position up, both from
/// 0.0 to 1.0. It reads through `live`, so a knob that a wire drives moves the point. A cut or a
/// notch has no gain to show, so its point sits on the 0 dB line.
pub fn point_of(node: &FlodeNode, live: &Live, band: usize) -> (f32, f32) {
    let x = position_of(live.plain(FREQ[band]));
    let y = if kind(node, band).uses_gain() {
        height_of(live.plain(GAIN[band]))
    } else {
        0.5
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Eq, Point::new(0.0, 0.0))
    }

    #[test]
    fn the_switches_pick_each_bands_kind() {
        let mut node = node();
        let kinds = |node: &FlodeNode| std::array::from_fn::<_, BANDS, _>(|b| kind(node, b));
        assert_eq!(
            kinds(&node),
            [
                EqKind::LowShelf,
                EqKind::Peak,
                EqKind::Peak,
                EqKind::HighShelf
            ]
        );
        for band in 0..BANDS {
            node.set_switch(band_switch(band), true);
        }
        assert_eq!(
            kinds(&node),
            [
                EqKind::HighPass,
                EqKind::Notch,
                EqKind::Notch,
                EqKind::LowPass
            ]
        );
    }

    /// The graph row comes first, and every band has a Freq, Gain, and Q knob with a pin, under
    /// its own section. The toggles are not rows.
    #[test]
    fn every_band_has_three_pinned_knobs() {
        let node = node();
        let rows = node.rows();
        assert_eq!(rows[1].control, Some(Control::Eq));
        let knobs: Vec<KnobId> = rows.iter().filter_map(|row| row.knob).collect();
        let expected: Vec<KnobId> = (0..BANDS).flat_map(|b| [FREQ[b], GAIN[b], Q[b]]).collect();
        assert_eq!(knobs, expected);
        assert!(rows
            .iter()
            .filter(|row| row.knob.is_some())
            .all(|row| row.input == Some(Port::Knob(row.knob.unwrap()))));
        assert_eq!(node.kind.behavior().sections().len(), BANDS);
        // The only switch row is the analyzer checkbox, right under the graph. The band toggles
        // are not rows.
        let switches: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row.control, Some(Control::Switch(_))))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(switches, vec![2]);
        assert_eq!(rows[2].control, Some(Control::Switch(0)));
        assert_eq!(node.switch_labels(), &["Analyzer"]);
    }

    /// The analyzer is switch 0, and flipping a band's toggle leaves it alone.
    #[test]
    fn the_analyzer_switch_is_separate_from_the_band_toggles() {
        let mut node = node();
        assert!(!analyzer(&node));
        assert!(!params(&node, &Live::of(&node)).analyzer);
        node.set_switch(0, true);
        assert!(params(&node, &Live::of(&node)).analyzer);
        assert_eq!(kind(&node, 0), EqKind::LowShelf);
        node.set_switch(band_switch(0), true);
        assert_eq!(kind(&node, 0), EqKind::HighPass);
        assert!(analyzer(&node));
    }

    #[test]
    fn a_new_node_is_flat() {
        let node = node();
        let params = params(&node, &Live::of(&node));
        assert!(params.bands.iter().all(|band| band.gain_db == 0.0));
        let freqs: Vec<f32> = params.bands.iter().map(|b| b.freq_hz).collect();
        assert!(freqs.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// Each icon stays in its box, and the icon changes with the switch.
    #[test]
    fn the_toggle_icons_follow_the_switch() {
        let mut node = node();
        let before: Vec<_> = (0..BANDS).map(|b| toggle_icon(&node, b)).collect();
        for icon in &before {
            assert!(icon.len() > 2);
            assert!(icon
                .iter()
                .all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)));
        }
        for band in 0..BANDS {
            node.set_switch(band_switch(band), true);
            assert_ne!(toggle_icon(&node, band), before[band], "band {band}");
        }
    }

    /// A low shelf is high on the left, and a high shelf is high on the right. A peak is high in
    /// the middle, and a notch is low there. Higher in the box is a smaller y.
    #[test]
    fn the_icons_show_their_shapes() {
        let mut node = node();
        let at = |icon: &[(f32, f32)], i: usize| icon[i].1;
        let low = toggle_icon(&node, 0);
        assert!(at(&low, 0) < at(&low, low.len() - 1));
        let high = toggle_icon(&node, 3);
        assert!(at(&high, 0) > at(&high, high.len() - 1));
        let peak = toggle_icon(&node, 1);
        assert!(at(&peak, peak.len() / 2) < at(&peak, 0));
        node.set_switch(band_switch(1), true);
        let notch = toggle_icon(&node, 1);
        assert!(at(&notch, notch.len() / 2) > at(&notch, 0));
        node.set_switch(band_switch(0), true);
        node.set_switch(band_switch(3), true);
        let high_pass = toggle_icon(&node, 0);
        assert!(at(&high_pass, 0) > at(&high_pass, high_pass.len() - 1));
        let low_pass = toggle_icon(&node, 3);
        assert!(at(&low_pass, 0) < at(&low_pass, low_pass.len() - 1));
    }

    #[test]
    fn the_axes_round_trip() {
        for freq in [20.0, 100.0, 1000.0, 20000.0] {
            assert!((frequency_at(position_of(freq)) / freq - 1.0).abs() < 1e-3);
        }
        for db in [-18.0, 0.0, 7.5, 18.0] {
            assert!((db_at(height_of(db)) - db).abs() < 1e-3);
        }
        assert_eq!(height_of(0.0), 0.5);
    }

    /// A band with no gain to show sits on the 0 dB line, wherever its gain knob is.
    #[test]
    fn a_cut_and_a_notch_sit_on_the_zero_line() {
        let mut node = node();
        node.set_value(GAIN[0], GAIN[0].to_normalized(12.0));
        node.set_value(GAIN[1], GAIN[1].to_normalized(-9.0));
        assert!(point_of(&node, &Live::of(&node), 0).1 > 0.5);
        assert!(point_of(&node, &Live::of(&node), 1).1 < 0.5);
        node.set_switch(band_switch(0), true);
        node.set_switch(band_switch(1), true);
        assert_eq!(point_of(&node, &Live::of(&node), 0).1, 0.5);
        assert_eq!(point_of(&node, &Live::of(&node), 1).1, 0.5);
    }
}
