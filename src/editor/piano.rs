//! Maps the computer keyboard to MIDI notes, laid out like Ableton Live's computer MIDI keyboard.
//! A, W, S, E, D, F, T, G, Y, H, U, J, K play C through C of the next octave, with W, E, T, Y,
//! U as the black keys. O, L, P, and semicolon continue the upper octave. Z and X shift the
//! octave down and up.
//!
//! The strip along the bottom of the editor draws the keys. It shows three octaves, which follow
//! the octave shift, so the keys on screen are the ones the computer keyboard plays. A column at
//! its left holds the octave buttons.

use super::draw::rounded_rect;
use super::style::{
    CHECKBOX_CHECKED_COLOR, DIM_TEXT_COLOR, DROPDOWN_COLOR, DROPDOWN_HOVER_COLOR, TEXT_COLOR,
};
use nih_plug_iced::keyboard::{self, KeyCode};
use nih_plug_iced::widget::canvas::{Frame, LineCap, Path, Stroke, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle, Size};

/// MIDI note for C3, which is where Live's computer keyboard starts.
const BASE_NOTE: i32 = 60;
const MAX_OCTAVE_SHIFT: i32 = 3;
/// About 100 out of 127, the velocity Live uses for the computer keyboard.
const VELOCITY: f32 = 0.79;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PianoEvent {
    NoteOn { note: u8, velocity: f32 },
    NoteOff { note: u8 },
}

/// Turns keyboard events into note events. Holds the octave and the keys that are down.
pub struct Piano {
    octave_shift: i32,
    /// Keys currently down, with the note each one started. Octave keys store `None`, so a
    /// release never sends a note off for them.
    held: Vec<(KeyCode, Option<u8>)>,
}

impl Default for Piano {
    fn default() -> Self {
        Self {
            octave_shift: 0,
            held: Vec::new(),
        }
    }
}

impl Piano {
    /// True if the event is for a key the piano uses. The editor must capture these. If it
    /// doesn't, AppKit beeps on every key repeat.
    pub fn uses(event: &keyboard::Event) -> bool {
        match event {
            keyboard::Event::KeyPressed { key_code, .. }
            | keyboard::Event::KeyReleased { key_code, .. } => {
                semitone(*key_code).is_some() || octave_shift(*key_code).is_some()
            }
            _ => false,
        }
    }

    /// How many octaves the keys are shifted from the start, from -3 to 3.
    pub fn octave(&self) -> i32 {
        self.octave_shift
    }

    /// Shifts the octave by `delta`, stopping at the ends. Z, X, and the strip's buttons all
    /// come here. Notes that are already down keep the notes they started.
    pub fn shift_octave(&mut self, delta: i32) {
        self.octave_shift = (self.octave_shift + delta).clamp(-MAX_OCTAVE_SHIFT, MAX_OCTAVE_SHIFT);
    }

    pub fn handle(&mut self, event: keyboard::Event) -> Option<PianoEvent> {
        match event {
            keyboard::Event::KeyPressed { key_code, .. } => self.press(key_code),
            keyboard::Event::KeyReleased { key_code, .. } => self.release(key_code),
            _ => None,
        }
    }

    fn press(&mut self, key: KeyCode) -> Option<PianoEvent> {
        // Auto-repeat sends another press while a key is held. Ignore it.
        if self.held.iter().any(|(held_key, _)| *held_key == key) {
            return None;
        }

        if let Some(shift) = octave_shift(key) {
            self.shift_octave(shift);
            self.held.push((key, None));
            return None;
        }

        let semitone = semitone(key)?;
        let note = (BASE_NOTE + self.octave_shift * 12 + semitone).clamp(0, 127) as u8;
        self.held.push((key, Some(note)));
        Some(PianoEvent::NoteOn {
            note,
            velocity: VELOCITY,
        })
    }

    fn release(&mut self, key: KeyCode) -> Option<PianoEvent> {
        let index = self
            .held
            .iter()
            .position(|(held_key, _)| *held_key == key)?;
        let (_, note) = self.held.swap_remove(index);
        note.map(|note| PianoEvent::NoteOff { note })
    }
}

/// Semitones above the octave's C for a note key.
fn semitone(key: KeyCode) -> Option<i32> {
    Some(match key {
        KeyCode::A => 0,
        KeyCode::W => 1,
        KeyCode::S => 2,
        KeyCode::E => 3,
        KeyCode::D => 4,
        KeyCode::F => 5,
        KeyCode::T => 6,
        KeyCode::G => 7,
        KeyCode::Y => 8,
        KeyCode::H => 9,
        KeyCode::U => 10,
        KeyCode::J => 11,
        KeyCode::K => 12,
        KeyCode::O => 13,
        KeyCode::L => 14,
        KeyCode::P => 15,
        KeyCode::Semicolon => 16,
        _ => return None,
    })
}

fn octave_shift(key: KeyCode) -> Option<i32> {
    match key {
        KeyCode::Z => Some(-1),
        KeyCode::X => Some(1),
        _ => None,
    }
}

/// The strip of keys along the bottom of the editor.
pub const STRIP_HEIGHT: f32 = 64.0;
/// The lowest note on the strip with no octave shift. Live calls it C2. The strip shows three
/// octaves, so with no shift it runs from C2 to B4, and the computer keyboard's C3 to E4 sits in
/// the middle of it.
pub const LOW_NOTE: u8 = 48;
/// How many notes past the lowest one the strip shows.
const SPAN: u8 = 35;
/// The velocity a click on a key plays at.
pub const CLICK_VELOCITY: f32 = 0.8;
/// The width of the column at the strip's left, which holds the octave buttons.
pub const CONTROLS_WIDTH: f32 = 116.0;
/// The first and last computer-keyboard notes, as semitones above the octave's C. They are the
/// A key and the semicolon key.
const COMPUTER_KEYS: (i32, i32) = (0, 16);

const BUTTON_WIDTH: f32 = 44.0;
const BUTTON_HEIGHT: f32 = 24.0;
const BUTTON_GAP: f32 = 4.0;

const STRIP_COLOR: Color = Color::from_rgb(0.085, 0.09, 0.115);
const CONTROLS_COLOR: Color = Color::from_rgb(0.13, 0.135, 0.17);
const WHITE_KEY_COLOR: Color = Color::from_rgb(0.90, 0.90, 0.92);
const BLACK_KEY_COLOR: Color = Color::from_rgb(0.12, 0.12, 0.14);
const BLACK_KEY_SHINE_COLOR: Color = Color::from_rgb(0.24, 0.24, 0.28);
const KEY_EDGE_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.44);
const KEY_LABEL_COLOR: Color = Color::from_rgb(0.48, 0.48, 0.54);
/// A key the mouse holds: blue, like the other things you can press.
const PRESSED_WHITE_COLOR: Color = Color::from_rgb(0.62, 0.76, 0.97);
const PRESSED_BLACK_COLOR: Color = Color::from_rgb(0.17, 0.38, 0.72);
/// A key that is sounding: orange, like the live dots on the nodes.
const SOUNDING_WHITE_COLOR: Color = Color::from_rgb(1.0, 0.84, 0.55);
const SOUNDING_BLACK_COLOR: Color = Color::from_rgb(0.78, 0.52, 0.14);
/// The color of live signals: the dot on a map or a viewer, and the highlighted wire.
pub const SOUNDING_DOT_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.25);

/// The strip's rectangle, along the bottom of the editor. It is in screen coordinates.
pub fn strip_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: 0.0,
        y: bounds.height - STRIP_HEIGHT,
        width: bounds.width,
        height: STRIP_HEIGHT,
    }
}

/// The part of the strip that holds the keys, right of the octave column.
fn keys_bounds(strip: Rectangle) -> Rectangle {
    Rectangle {
        x: strip.x + CONTROLS_WIDTH,
        width: (strip.width - CONTROLS_WIDTH).max(0.0),
        ..strip
    }
}

/// The lowest note on the strip for an octave shift. The strip moves a whole octave per step.
pub fn low_note(octave: i32) -> u8 {
    (i32::from(LOW_NOTE) + octave * 12).clamp(0, 127 - i32::from(SPAN)) as u8
}

/// The name Live gives a C, such as "C3" for note 60.
fn c_name(note: u8) -> String {
    format!("C{}", i32::from(note) / 12 - 2)
}

/// True for the black keys, by pitch class.
fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// The white keys on the strip, in order, for a strip that starts at `low`.
fn white_notes(low: u8) -> impl Iterator<Item = u8> {
    (low..=low + SPAN).filter(|&note| !is_black(note))
}

/// Every key on the strip with its rectangle. White keys share the key area evenly. Each black
/// key sits over the seam between two white keys.
fn key_rects(strip: Rectangle, low: u8) -> Vec<(u8, Rectangle, bool)> {
    let keys_area = keys_bounds(strip);
    let white_count = white_notes(low).count() as f32;
    let white_width = keys_area.width / white_count;
    let mut keys = Vec::new();
    for (index, note) in white_notes(low).enumerate() {
        keys.push((
            note,
            Rectangle {
                x: keys_area.x + index as f32 * white_width,
                y: keys_area.y,
                width: white_width,
                height: keys_area.height,
            },
            false,
        ));
    }
    let black_width = white_width * 0.6;
    for note in low..=low + SPAN {
        if !is_black(note) {
            continue;
        }
        // The white keys below this note set where its seam sits.
        let whites_below = (low..note).filter(|&n| !is_black(n)).count() as f32;
        keys.push((
            note,
            Rectangle {
                x: keys_area.x + whites_below * white_width - black_width / 2.0,
                y: keys_area.y,
                width: black_width,
                height: keys_area.height * 0.6,
            },
            true,
        ));
    }
    keys
}

/// The key under `point`, if any. Black keys sit on top of the white keys. The octave column has
/// no keys.
pub fn key_at(strip: Rectangle, low: u8, point: Point) -> Option<u8> {
    let keys = key_rects(strip, low);
    keys.iter()
        .filter(|(_, _, black)| *black)
        .chain(keys.iter().filter(|(_, _, black)| !black))
        .find(|(_, rect, _)| rect.contains(point))
        .map(|(note, _, _)| *note)
}

/// The octave button of `direction`, which is -1 for down and 1 for up.
fn octave_button(strip: Rectangle, direction: i32) -> Rectangle {
    let side = if direction < 0 { 0.0 } else { 1.0 };
    let pair = 2.0 * BUTTON_WIDTH + BUTTON_GAP;
    Rectangle {
        x: strip.x + (CONTROLS_WIDTH - pair) / 2.0 + side * (BUTTON_WIDTH + BUTTON_GAP),
        y: strip.y + 32.0,
        width: BUTTON_WIDTH,
        height: BUTTON_HEIGHT,
    }
}

/// The octave button under `point`: -1 for down, 1 for up.
pub fn octave_button_at(strip: Rectangle, point: Point) -> Option<i32> {
    [-1, 1]
        .into_iter()
        .find(|&direction| octave_button(strip, direction).contains(point))
}

/// True if `note` is in the bit set. The set holds notes 0 to 127 in two words.
pub fn is_set(notes: [u64; 2], note: u8) -> bool {
    notes[(note / 64) as usize] & (1u64 << (note % 64)) != 0
}

/// What the strip shows besides its keys.
pub struct StripView {
    /// The octave shift. It sets which three octaves the strip shows.
    pub octave: i32,
    /// The notes the synth is playing.
    pub sounding: [u64; 2],
    /// The key the mouse holds, if any.
    pub pressed: Option<u8>,
    /// True while the editor is in Keyboard mode, where the computer keyboard plays notes. The
    /// strip then marks the keys it plays. In Edit mode those same keys are shortcuts instead, so
    /// the strip leaves the mark off rather than show a legend for keys that do something else.
    pub computer_keys: bool,
    /// The octave button under the cursor, if any.
    pub hover: Option<i32>,
    /// False when something is drawn over the strip. Text would show through it, so the labels
    /// are left out.
    pub labels: bool,
}

/// Draws the strip: the octave column, the keys, and the mark for the computer keyboard's range.
pub fn draw_strip(frame: &mut Frame, strip: Rectangle, view: &StripView) {
    let low = low_note(view.octave);
    frame.fill(
        &Path::rectangle(Point::new(strip.x, strip.y), strip.size()),
        STRIP_COLOR,
    );

    // A soft edge above the strip, so it sits over the graph. It fades upward.
    for (step, alpha) in [0.30, 0.20, 0.11, 0.05].into_iter().enumerate() {
        frame.fill(
            &Path::rectangle(
                Point::new(strip.x, strip.y - 1.0 - step as f32),
                Size::new(strip.width, 1.0),
            ),
            Color::from_rgba(0.0, 0.0, 0.0, alpha),
        );
    }

    draw_octave_column(frame, strip, view);

    let keys = key_rects(strip, low);
    for (note, rect, black) in keys.iter().copied().filter(|(_, _, black)| !black) {
        draw_key(frame, note, rect, black, view);
    }
    for (note, rect, black) in keys.iter().copied().filter(|(_, _, black)| *black) {
        draw_key(frame, note, rect, black, view);
    }

    if view.computer_keys {
        draw_computer_range(frame, &keys, view.octave);
    }
}

/// The octave column: the shift as text, and the down and up buttons.
fn draw_octave_column(frame: &mut Frame, strip: Rectangle, view: &StripView) {
    let column = Rectangle {
        width: CONTROLS_WIDTH,
        ..strip
    };
    frame.fill(
        &Path::rectangle(Point::new(column.x, column.y), column.size()),
        CONTROLS_COLOR,
    );

    if view.labels {
        let readout = if view.octave == 0 {
            "Octave 0".to_string()
        } else {
            format!("Octave {:+}", view.octave)
        };
        frame.fill_text(Text {
            content: readout,
            position: Point::new(column.x + column.width / 2.0, column.y + 16.0),
            color: TEXT_COLOR,
            size: 12.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }

    for direction in [-1, 1] {
        let rect = octave_button(strip, direction);
        // A button at the end of the range has nowhere to go.
        let enabled = (view.octave + direction).abs() <= MAX_OCTAVE_SHIFT;
        let color = if enabled && view.hover == Some(direction) {
            DROPDOWN_HOVER_COLOR
        } else {
            DROPDOWN_COLOR
        };
        frame.fill(&rounded_rect(rect, 4.0), color);

        let glyph = if enabled { TEXT_COLOR } else { DIM_TEXT_COLOR };
        let stroke = Stroke::default()
            .with_color(glyph)
            .with_width(1.5)
            .with_line_cap(LineCap::Round);
        let center = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        frame.stroke(
            &Path::line(
                Point::new(center.x - 5.0, center.y),
                Point::new(center.x + 5.0, center.y),
            ),
            stroke,
        );
        if direction > 0 {
            frame.stroke(
                &Path::line(
                    Point::new(center.x, center.y - 5.0),
                    Point::new(center.x, center.y + 5.0),
                ),
                stroke,
            );
        }
    }
}

/// Fills a rectangle whose top corners are square and bottom corners are round.
fn fill_hanging(frame: &mut Frame, rect: Rectangle, radius: f32, color: Color) {
    frame.fill(&rounded_rect(rect, radius), color);
    frame.fill(
        &Path::rectangle(Point::new(rect.x, rect.y), Size::new(rect.width, radius)),
        color,
    );
}

/// One key. A white key leaves a one pixel gap to its neighbour, and both kinds have round bottom
/// corners. A sounding key is orange, and a key the mouse holds is blue.
fn draw_key(frame: &mut Frame, note: u8, rect: Rectangle, black: bool, view: &StripView) {
    let pressed = view.pressed == Some(note);
    let sounding = is_set(view.sounding, note);
    let color = match (black, pressed, sounding) {
        (false, true, _) => PRESSED_WHITE_COLOR,
        (true, true, _) => PRESSED_BLACK_COLOR,
        (false, false, true) => SOUNDING_WHITE_COLOR,
        (true, false, true) => SOUNDING_BLACK_COLOR,
        (false, false, false) => WHITE_KEY_COLOR,
        (true, false, false) => BLACK_KEY_COLOR,
    };
    if black {
        fill_hanging(frame, rect, 2.5, color);
        // A faint line near the bottom edge, as if the key caught the light.
        frame.fill(
            &Path::rectangle(
                Point::new(rect.x + 3.0, rect.y + rect.height - 4.0),
                Size::new(rect.width - 6.0, 1.0),
            ),
            BLACK_KEY_SHINE_COLOR,
        );
        return;
    }

    let key = Rectangle {
        x: rect.x + 0.5,
        width: rect.width - 1.0,
        ..rect
    };
    fill_hanging(frame, key, 3.0, color);
    frame.fill(
        &Path::rectangle(
            Point::new(key.x + 2.0, key.y + key.height - 1.0),
            Size::new(key.width - 4.0, 1.0),
        ),
        KEY_EDGE_COLOR,
    );
    if view.labels && note % 12 == 0 {
        frame.fill_text(Text {
            content: c_name(note),
            position: Point::new(key.x + key.width / 2.0, key.y + key.height - 9.0),
            color: KEY_LABEL_COLOR,
            size: 10.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// A bar across the top of the keys the computer keyboard plays, from the A key to the semicolon
/// key. It moves with the octave shift.
fn draw_computer_range(frame: &mut Frame, keys: &[(u8, Rectangle, bool)], octave: i32) {
    let first = (BASE_NOTE + octave * 12 + COMPUTER_KEYS.0) as u8;
    let last = (BASE_NOTE + octave * 12 + COMPUTER_KEYS.1) as u8;
    let span = keys
        .iter()
        .filter(|(note, _, _)| (first..=last).contains(note))
        .fold(None, |span: Option<(f32, f32, f32)>, (_, rect, _)| {
            let (left, right, top) = span.unwrap_or((rect.x, rect.x + rect.width, rect.y));
            Some((
                left.min(rect.x),
                right.max(rect.x + rect.width),
                top.min(rect.y),
            ))
        });
    if let Some((left, right, top)) = span {
        frame.fill(
            &Path::rectangle(Point::new(left, top), Size::new(right - left, 3.0)),
            CHECKBOX_CHECKED_COLOR,
        );
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    fn strip() -> Rectangle {
        strip_bounds(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 400.0,
        })
    }

    /// The width of one white key on the strip, which has 21 of them.
    fn white_width() -> f32 {
        (strip().width - CONTROLS_WIDTH) / 21.0
    }

    /// A click on a white key's centre plays that key, and a click on a black key's top plays the
    /// black key over the seam, not the white key under it.
    #[test]
    fn clicks_land_on_the_right_keys() {
        let strip = strip();
        let w = white_width();
        let left = strip.x + CONTROLS_WIDTH;
        let y = strip.y + strip.height - 8.0;
        assert_eq!(key_at(strip, 48, Point::new(left + w / 2.0, y)), Some(48));
        assert_eq!(
            key_at(strip, 48, Point::new(left + w, strip.y + 10.0)),
            Some(49)
        );
        assert_eq!(key_at(strip, 48, Point::new(left + w * 1.5, y)), Some(50));
    }

    /// The octave column is not a key.
    #[test]
    fn the_octave_column_plays_nothing() {
        let strip = strip();
        let y = strip.y + strip.height - 8.0;
        assert_eq!(key_at(strip, 48, Point::new(10.0, y)), None);
        assert_eq!(key_at(strip, 48, Point::new(CONTROLS_WIDTH - 1.0, y)), None);
    }

    /// The strip moves a whole octave per shift, and stays inside the 128 notes.
    #[test]
    fn the_strip_follows_the_octave() {
        assert_eq!(low_note(0), 48);
        assert_eq!(low_note(1), 60);
        assert_eq!(low_note(-3), 12);
        assert_eq!(low_note(3), 84);
        for octave in -MAX_OCTAVE_SHIFT..=MAX_OCTAVE_SHIFT {
            assert!(low_note(octave) + SPAN <= 127);
            assert_eq!(low_note(octave) % 12, 0);
        }
    }

    /// With a shift, the keys on the strip are the shifted notes.
    #[test]
    fn a_click_plays_the_shifted_note() {
        let strip = strip();
        let left = strip.x + CONTROLS_WIDTH;
        let y = strip.y + strip.height - 8.0;
        let first = Point::new(left + white_width() / 2.0, y);
        assert_eq!(key_at(strip, low_note(1), first), Some(60));
        assert_eq!(key_at(strip, low_note(-1), first), Some(36));
    }

    /// The computer keyboard's range sits on the strip at every shift, so its bar is always drawn.
    #[test]
    fn the_computer_range_is_always_on_the_strip() {
        for octave in -MAX_OCTAVE_SHIFT..=MAX_OCTAVE_SHIFT {
            let low = low_note(octave);
            let first = (BASE_NOTE + octave * 12 + COMPUTER_KEYS.0) as u8;
            let last = (BASE_NOTE + octave * 12 + COMPUTER_KEYS.1) as u8;
            assert!(first >= low && last <= low + SPAN, "octave {octave}");
        }
    }

    /// Live's names: note 60 is C3, and the strip's first note is C2.
    #[test]
    fn c_names_follow_live() {
        assert_eq!(c_name(60), "C3");
        assert_eq!(c_name(48), "C2");
        assert_eq!(c_name(12), "C-1");
    }

    /// The two buttons are told apart, and a press anywhere else in the column hits neither.
    #[test]
    fn the_octave_buttons_are_found_by_position() {
        let strip = strip();
        let center = |direction| {
            let rect = octave_button(strip, direction);
            Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
        };
        assert_eq!(octave_button_at(strip, center(-1)), Some(-1));
        assert_eq!(octave_button_at(strip, center(1)), Some(1));
        assert_eq!(
            octave_button_at(strip, Point::new(4.0, strip.y + 4.0)),
            None
        );
        assert_eq!(
            octave_button_at(strip, Point::new(500.0, strip.y + 40.0)),
            None
        );
    }

    #[test]
    fn the_buttons_sit_inside_the_column() {
        let strip = strip();
        for direction in [-1, 1] {
            let rect = octave_button(strip, direction);
            assert!(rect.x >= strip.x && rect.x + rect.width <= strip.x + CONTROLS_WIDTH);
            assert!(rect.y >= strip.y && rect.y + rect.height <= strip.y + strip.height);
        }
    }

    /// The buttons and the Z and X keys move the same octave, and it stops at three either way.
    #[test]
    fn the_octave_stops_at_three_each_way() {
        let mut piano = Piano::default();
        for _ in 0..10 {
            piano.shift_octave(1);
        }
        assert_eq!(piano.octave(), 3);
        for _ in 0..10 {
            piano.shift_octave(-1);
        }
        assert_eq!(piano.octave(), -3);
        piano.shift_octave(1);
        let press = |piano: &mut Piano, key_code| {
            piano.handle(keyboard::Event::KeyPressed {
                key_code,
                modifiers: keyboard::Modifiers::default(),
            })
        };
        press(&mut piano, KeyCode::X);
        assert_eq!(piano.octave(), -1);
    }

    /// A key played before the octave shifts still ends with the note it started.
    #[test]
    fn a_held_note_keeps_its_note_through_a_shift() {
        let mut piano = Piano::default();
        let event = |pressed, key_code| {
            let modifiers = keyboard::Modifiers::default();
            if pressed {
                keyboard::Event::KeyPressed {
                    key_code,
                    modifiers,
                }
            } else {
                keyboard::Event::KeyReleased {
                    key_code,
                    modifiers,
                }
            }
        };
        let on = piano.handle(event(true, KeyCode::A));
        piano.shift_octave(1);
        let off = piano.handle(event(false, KeyCode::A));
        assert_eq!(
            on,
            Some(PianoEvent::NoteOn {
                note: 60,
                velocity: VELOCITY
            })
        );
        assert_eq!(off, Some(PianoEvent::NoteOff { note: 60 }));
    }

    /// The sounding set reads one bit per note.
    #[test]
    fn sounding_set_reads_bits() {
        let notes = [1u64 << 0, 1u64 << 12];
        assert!(is_set(notes, 0));
        assert!(is_set(notes, 76));
        assert!(!is_set(notes, 1));
    }
}
