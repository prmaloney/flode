//! Maps the computer keyboard to MIDI notes, laid out like Ableton Live's computer MIDI keyboard.
//! A, W, S, E, D, F, T, G, Y, H, U, J, K play C through C of the next octave, with W, E, T, Y,
//! U as the black keys. O, L, P, and semicolon continue the upper octave. Z and X shift the
//! octave down and up.

use nih_plug_iced::keyboard::{self, KeyCode};
use nih_plug_iced::widget::canvas::{Frame, Path};
use nih_plug_iced::{Color, Point, Rectangle};

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
            self.octave_shift =
                (self.octave_shift + shift).clamp(-MAX_OCTAVE_SHIFT, MAX_OCTAVE_SHIFT);
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
/// The lowest and highest notes on the strip. Three octaves, C3 to B5.
pub const LOW_NOTE: u8 = 48;
pub const HIGH_NOTE: u8 = 83;
/// The velocity a click on a key plays at.
pub const CLICK_VELOCITY: f32 = 0.8;

const WHITE_KEY_COLOR: Color = Color::from_rgb(0.90, 0.90, 0.92);
const BLACK_KEY_COLOR: Color = Color::from_rgb(0.12, 0.12, 0.14);
const PRESSED_KEY_COLOR: Color = Color::from_rgb(0.60, 0.60, 0.66);
const KEY_EDGE_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.44);
/// The dot under a note that is sounding.
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

/// True for the black keys, by pitch class.
fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// The white keys on the strip, in order.
fn white_notes() -> impl Iterator<Item = u8> {
    (LOW_NOTE..=HIGH_NOTE).filter(|&note| !is_black(note))
}

/// Every key on the strip with its rectangle. White keys share the strip evenly. Each black key
/// sits over the seam between two white keys.
fn key_rects(strip: Rectangle) -> Vec<(u8, Rectangle, bool)> {
    let white_count = white_notes().count() as f32;
    let white_width = strip.width / white_count;
    let mut keys = Vec::new();
    for (index, note) in white_notes().enumerate() {
        keys.push((
            note,
            Rectangle {
                x: strip.x + index as f32 * white_width,
                y: strip.y,
                width: white_width,
                height: strip.height,
            },
            false,
        ));
    }
    let black_width = white_width * 0.6;
    for note in LOW_NOTE..=HIGH_NOTE {
        if !is_black(note) {
            continue;
        }
        // The white keys below this note set where its seam sits.
        let whites_below = (LOW_NOTE..note).filter(|&n| !is_black(n)).count() as f32;
        keys.push((
            note,
            Rectangle {
                x: strip.x + whites_below * white_width - black_width / 2.0,
                y: strip.y,
                width: black_width,
                height: strip.height * 0.6,
            },
            true,
        ));
    }
    keys
}

/// The key under `point`, if any. Black keys sit on top of the white keys.
pub fn key_at(strip: Rectangle, point: Point) -> Option<u8> {
    let keys = key_rects(strip);
    keys.iter()
        .filter(|(_, _, black)| *black)
        .chain(keys.iter().filter(|(_, _, black)| !black))
        .find(|(_, rect, _)| rect.contains(point))
        .map(|(note, _, _)| *note)
}

/// True if `note` is in the bit set. The set holds notes 0 to 127 in two words.
pub fn is_set(notes: [u64; 2], note: u8) -> bool {
    notes[(note / 64) as usize] & (1u64 << (note % 64)) != 0
}

/// Draws the strip. `sounding` is the set of notes the synth is playing. `pressed` is the key the
/// mouse holds, if any.
pub fn draw_strip(frame: &mut Frame, strip: Rectangle, sounding: [u64; 2], pressed: Option<u8>) {
    let keys = key_rects(strip);
    let mut draw_key = |note: u8, rect: Rectangle, black: bool| {
        let color = if pressed == Some(note) {
            PRESSED_KEY_COLOR
        } else if black {
            BLACK_KEY_COLOR
        } else {
            WHITE_KEY_COLOR
        };
        frame.fill(
            &Path::rectangle(Point::new(rect.x, rect.y), rect.size()),
            color,
        );
        if !black {
            frame.fill(
                &Path::rectangle(
                    Point::new(rect.x, rect.y + rect.height - 1.0),
                    nih_plug_iced::Size::new(rect.width, 1.0),
                ),
                KEY_EDGE_COLOR,
            );
        }
        if is_set(sounding, note) {
            let dot = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height - 9.0);
            frame.fill(&Path::circle(dot, 3.0), SOUNDING_DOT_COLOR);
        }
    };
    for (note, rect, black) in keys.iter().copied().filter(|(_, _, black)| !black) {
        draw_key(note, rect, black);
    }
    for (note, rect, black) in keys.iter().copied().filter(|(_, _, black)| *black) {
        draw_key(note, rect, black);
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    /// A click on a white key's centre plays that key, and a click on a black key's top plays the
    /// black key over the seam, not the white key under it.
    #[test]
    fn clicks_land_on_the_right_keys() {
        let strip = strip_bounds(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 400.0,
        });
        let white_width = strip.width / 21.0;
        let y = strip.y + strip.height - 8.0;
        assert_eq!(key_at(strip, Point::new(white_width / 2.0, y)), Some(48));
        assert_eq!(
            key_at(strip, Point::new(white_width, strip.y + 10.0)),
            Some(49)
        );
        assert_eq!(key_at(strip, Point::new(white_width * 1.5, y)), Some(50));
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
