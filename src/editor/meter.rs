//! The output meter: two thin bars at the top right of the editor, one for each output channel.
//! It stays in the same place on screen however the canvas is panned or zoomed, so the level is
//! always in view. The bars show peak level on a dB scale and fall back slowly, so a short peak
//! is still readable.

use super::depth::{draw_shadow, FLOAT_SHADOW};
use super::draw::rounded_rect;
use super::piano::SOUNDING_DOT_COLOR;
use super::style::{DIM_TEXT_COLOR, PILL_COLOR, PILL_HEIGHT, PILL_TOP, TEXT_COLOR};
use nih_plug_iced::widget::canvas::{Frame, Path, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle, Size};

const METER_WIDTH: f32 = 150.0;
const MARGIN: f32 = 16.0;
const BAR_HEIGHT: f32 = 8.0;
const BAR_GAP: f32 = 4.0;
/// Space on the left of the bars for the channel letter.
const LABEL_WIDTH: f32 = 24.0;
const END_PADDING: f32 = 10.0;

/// The level at the bottom of the scale, in dB. Anything quieter shows as empty.
const FLOOR_DB: f32 = -60.0;
/// Where the bar turns from green to yellow, and from yellow to red, in dB.
const YELLOW_DB: f32 = -12.0;
const RED_DB: f32 = -3.0;
/// How fast the bars fall back, in dB per second.
const FALL_DB_PER_SECOND: f32 = 20.0;

/// The empty part of a bar, lighter than the panel so the scale is visible with no sound.
const TRACK: Color = Color::from_rgb(0.30, 0.30, 0.35);
const GREEN: Color = Color::from_rgb(0.30, 0.78, 0.40);
const YELLOW: Color = Color::from_rgb(0.95, 0.80, 0.25);
const RED: Color = Color::from_rgb(0.92, 0.28, 0.25);

/// The meter's rectangle, in screen coordinates. `bounds` is the editor's.
pub(super) fn meter_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: bounds.width - MARGIN - METER_WIDTH,
        y: PILL_TOP,
        width: METER_WIDTH,
        height: PILL_HEIGHT,
    }
}

/// How far along the bar a peak reaches, from 0.0 (empty) to 1.0 (full scale or louder).
pub(super) fn fraction(peak: f32) -> f32 {
    if peak <= 0.0 || !peak.is_finite() {
        return 0.0;
    }
    ((20.0 * peak.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// The held level of each channel. A new peak jumps the bar up at once, and the bar falls back at
/// a steady rate in dB, so it takes the same time to drop from -6 dB to -26 dB as from -26 dB to
/// -46 dB.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Ballistics {
    held: [f32; 2],
}

impl Ballistics {
    /// Takes the newest peaks, `seconds` after the last call, and returns what the bars show.
    pub(super) fn update(&mut self, peaks: [f32; 2], seconds: f32) -> [f32; 2] {
        let fall = 10f32.powf(-FALL_DB_PER_SECOND * seconds.max(0.0) / 20.0);
        for (held, peak) in self.held.iter_mut().zip(peaks) {
            let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
            *held = peak.max(*held * fall);
            // A level far under the floor is an empty bar, so stop tracking it.
            if *held < 1e-6 {
                *held = 0.0;
            }
        }
        self.held
    }
}

/// How long the MIDI light takes to fade after a note arrives, in seconds.
const MIDI_LIGHT_SECONDS: f32 = 0.35;

/// How lit the MIDI light is `seconds` after a note arrived: full at once, then fading to dark.
pub(super) fn midi_light(seconds: f32) -> f32 {
    (1.0 - seconds / MIDI_LIGHT_SECONDS).clamp(0.0, 1.0)
}

/// The MIDI light: a dot with its name under it, left of the meter. It lights when the audio
/// thread takes in a note, whether it came from the host or the key strip, so a note that is
/// heard as nothing can be told from one that never arrived. `light` is from 0.0 to 1.0.
fn draw_midi_light(frame: &mut Frame, panel: Rectangle, light: f32) {
    let center = Point::new(panel.x - 22.0, panel.y + PILL_HEIGHT / 2.0 - 5.0);
    if light > 0.0 {
        frame.fill(
            &Path::circle(center, 7.0),
            Color {
                a: 0.25 * light,
                ..SOUNDING_DOT_COLOR
            },
        );
    }
    frame.fill(
        &Path::circle(center, 3.5),
        lerp_color(TRACK, SOUNDING_DOT_COLOR, light),
    );
    frame.fill_text(Text {
        content: "MIDI".to_string(),
        position: Point::new(center.x, center.y + 11.0),
        color: DIM_TEXT_COLOR,
        size: 9.0,
        horizontal_alignment: alignment::Horizontal::Center,
        vertical_alignment: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// The color a fraction `t` of the way from `from` to `to`.
fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    Color::from_rgb(mix(from.r, to.r), mix(from.g, to.g), mix(from.b, to.b))
}

/// Draws the meter. `levels` are the held peaks as linear amplitude. `midi_light` is how lit the
/// MIDI indicator is.
pub(super) fn draw(frame: &mut Frame, bounds: Rectangle, levels: [f32; 2], midi_light: f32) {
    let panel = meter_bounds(bounds);
    draw_midi_light(frame, panel, midi_light);
    draw_shadow(frame, panel, PILL_HEIGHT / 2.0, FLOAT_SHADOW);
    frame.fill(&rounded_rect(panel, PILL_HEIGHT / 2.0), PILL_COLOR);

    let left = panel.x + LABEL_WIDTH;
    let width = panel.width - LABEL_WIDTH - END_PADDING;
    let top = panel.y + (panel.height - (2.0 * BAR_HEIGHT + BAR_GAP)) / 2.0;
    for (channel, (label, level)) in ["L", "R"].into_iter().zip(levels).enumerate() {
        let y = top + channel as f32 * (BAR_HEIGHT + BAR_GAP);
        let track = Rectangle {
            x: left,
            y,
            width,
            height: BAR_HEIGHT,
        };
        frame.fill(&rounded_rect(track, BAR_HEIGHT / 2.0), TRACK);

        // The bar is filled in up to three colored runs, so the hot part reads at a glance.
        let reach = fraction(level);
        let runs = [
            (0.0, fraction_of_db(YELLOW_DB), GREEN),
            (fraction_of_db(YELLOW_DB), fraction_of_db(RED_DB), YELLOW),
            (fraction_of_db(RED_DB), 1.0, RED),
        ];
        for (from, to, color) in runs {
            let end = reach.min(to);
            if end > from {
                frame.fill(
                    &Path::rectangle(
                        Point::new(left + from * width, y),
                        Size::new((end - from) * width, BAR_HEIGHT),
                    ),
                    color,
                );
            }
        }

        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(panel.x + 12.0, y + BAR_HEIGHT / 2.0),
            color: TEXT_COLOR,
            size: 10.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// Where a level in dB falls along the bar.
fn fraction_of_db(db: f32) -> f32 {
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The MIDI light is full when a note arrives, fades steadily, and is dark after its time.
    #[test]
    fn the_midi_light_fades_after_a_note() {
        assert_eq!(midi_light(0.0), 1.0);
        let half = midi_light(MIDI_LIGHT_SECONDS / 2.0);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        assert_eq!(midi_light(MIDI_LIGHT_SECONDS), 0.0);
        assert_eq!(midi_light(10.0), 0.0);
    }

    fn db(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// Silence is empty, full scale is full, and the dB scale is linear between them.
    #[test]
    fn the_bar_follows_a_db_scale() {
        assert_eq!(fraction(0.0), 0.0);
        assert!((fraction(1.0) - 1.0).abs() < 1e-6);
        assert!((fraction(db(-30.0)) - 0.5).abs() < 1e-4);
        assert_eq!(fraction(db(-80.0)), 0.0);
        assert_eq!(fraction(4.0), 1.0);
        assert_eq!(fraction(f32::NAN), 0.0);
    }

    /// The colour thresholds sit where the constants say.
    #[test]
    fn the_color_runs_split_at_the_thresholds() {
        assert!((fraction_of_db(YELLOW_DB) - fraction(db(YELLOW_DB))).abs() < 1e-4);
        assert!((fraction_of_db(RED_DB) - fraction(db(RED_DB))).abs() < 1e-4);
        assert!(fraction_of_db(YELLOW_DB) < fraction_of_db(RED_DB));
    }

    /// A louder peak lifts the bar at once.
    #[test]
    fn a_new_peak_jumps_the_bar_up() {
        let mut meter = Ballistics::default();
        assert_eq!(meter.update([0.5, 0.0], 0.016), [0.5, 0.0]);
        assert_eq!(meter.update([0.8, 0.2], 0.016)[0], 0.8);
    }

    /// With no new peak, the bar falls 20 dB each second.
    #[test]
    fn the_bar_falls_at_a_steady_rate_in_db() {
        let mut meter = Ballistics::default();
        meter.update([1.0, 1.0], 0.0);
        let after_one_second = meter.update([0.0, 0.0], 1.0);
        assert!((after_one_second[0] - db(-20.0)).abs() < 1e-4);
        let after_two = meter.update([0.0, 0.0], 1.0);
        assert!((after_two[0] - db(-40.0)).abs() < 1e-4);
    }

    /// The two channels are held apart, and a bad value does not stick.
    #[test]
    fn channels_are_independent_and_bad_values_are_ignored() {
        let mut meter = Ballistics::default();
        let held = meter.update([0.9, f32::NAN], 0.016);
        assert_eq!(held, [0.9, 0.0]);
        let held = meter.update([f32::INFINITY, 0.1], 0.016);
        assert!(held[0] < 0.9 && held[0].is_finite());
        assert_eq!(held[1], 0.1);
    }

    /// A tail that has fallen far under the floor goes to zero and stays there.
    #[test]
    fn a_very_quiet_tail_settles_to_zero() {
        let mut meter = Ballistics::default();
        meter.update([1e-5, 0.0], 0.0);
        assert_eq!(meter.update([0.0, 0.0], 2.0), [0.0, 0.0]);
    }

    /// The meter sits at the top right, clear of the pill in the default window.
    #[test]
    fn the_meter_sits_right_of_the_pill() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let meter = meter_bounds(bounds);
        let pill = super::super::menus::pill_bounds(bounds);
        assert!(meter.x > pill.x + pill.width);
        assert!(meter.x + meter.width <= bounds.width);
    }
}
