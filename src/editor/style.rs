//! Sizes and colors the editor draws with.

use nih_plug_iced::Color;

pub(super) const CORNER_RADIUS: f32 = 8.0;
pub(super) const TERMINAL_HIT_RADIUS: f32 = 10.0;
pub(super) const PIN_RADIUS: f32 = 5.0;
/// Control pins are smaller than audio pins.
pub(super) const CONTROL_PIN_RADIUS: f32 = 4.0;
pub(super) const PIN_LABEL_INSET: f32 = 10.0;
pub(super) const PIN_LABEL_SIZE: f32 = 11.0;
/// The name and value under a pin sit this far above and below the pin's row center.
pub(super) const NAME_OFFSET: f32 = -7.0;
pub(super) const VALUE_OFFSET: f32 = 7.0;

pub(super) const DROPDOWN_CORNER_RADIUS: f32 = 4.0;
pub(super) const WAVEFORM_ITEM_HEIGHT: f32 = 26.0;
/// Gap between the dropdown and the list that opens below it.
pub(super) const LIST_GAP: f32 = 2.0;

pub(super) const CHECKBOX_SIZE: f32 = 12.0;

/// The height of the controls docked at the top of the editor: the burger button, the meter, and
/// the status line's gap under them.
pub(super) const PILL_HEIGHT: f32 = 36.0;
pub(super) const PILL_TOP: f32 = 12.0;
/// The side of the square burger button, the same height as the other top controls.
pub(super) const BURGER_SIZE: f32 = PILL_HEIGHT;
/// How wide the burger menu's top-level list is.
pub(super) const BURGER_MENU_WIDTH: f32 = 190.0;
/// The gap the burger menu's divider takes, between Utility and Presets.
pub(super) const BURGER_DIVIDER_HEIGHT: f32 = 9.0;

/// The narrowest a category list is, so short labels still get a usable row.
pub(super) const CATEGORY_MIN_WIDTH: f32 = 160.0;

/// How long the line under the burger menu stays up.
pub(super) const STATUS_SECONDS: std::time::Duration = std::time::Duration::from_secs(3);
/// The preset list is at least this wide, so most names fit on a row.
pub(super) const PRESET_LIST_MIN_WIDTH: f32 = 220.0;

pub(super) const MENU_WIDTH: f32 = 150.0;
pub(super) const MENU_ITEM_HEIGHT: f32 = 32.0;
pub(super) const MENU_CORNER_RADIUS: f32 = 6.0;

/// The window behind the graph: a cool, dark blue grey, so the nodes lift off it.
pub(super) const BACKGROUND_COLOR: Color = Color::from_rgb(0.105, 0.11, 0.135);
pub(super) const NODE_BODY_COLOR: Color = Color::from_rgb(0.235, 0.24, 0.285);
pub(super) const NODE_HEADER_COLOR: Color = Color::from_rgb(0.29, 0.30, 0.38);
/// The band behind a section's title, darker than the node's body so it reads as a recess, with
/// light text and arrow on it.
pub(super) const SECTION_COLOR: Color = Color::from_rgb(0.165, 0.17, 0.21);
pub(super) const SECTION_TEXT_COLOR: Color = Color::from_rgb(0.82, 0.82, 0.88);
/// The pins of a folded section or node are drawn smaller than ordinary pins, so a tight stack of
/// them stays readable.
pub(super) const PACKED_PIN_RADIUS: f32 = 3.0;
/// The arrow of a section header sits this far from the node's left edge.
pub(super) const SECTION_ARROW_X: f32 = 18.0;
/// The body and header of a node that is muted or bypassed, darker than a live node's.
pub(super) const NODE_OFF_BODY_COLOR: Color = Color::from_rgb(0.185, 0.19, 0.225);
pub(super) const NODE_OFF_HEADER_COLOR: Color = Color::from_rgb(0.225, 0.23, 0.275);
/// The selected node's outline and glow: the interactive blue, lighter than the knob arcs so it
/// reads against a dark background.
pub(super) const SELECTION_COLOR: Color = Color::from_rgb(0.42, 0.68, 1.0);
pub(super) const SELECTION_WIDTH: f32 = 2.0;
/// The spectrum the EQ's analyzer draws behind its curve.
pub(super) const ANALYZER_COLOR: Color = Color::from_rgba(0.19, 0.50, 0.90, 0.40);
/// The monitor node's screen, darker than a dropdown so the picture stands out.
pub(super) const MONITOR_SCREEN_COLOR: Color = Color::from_rgb(0.06, 0.065, 0.09);
/// The left and right channel traces of a monitor.
pub(super) const MONITOR_LEFT_COLOR: Color = SELECTION_COLOR;
pub(super) const MONITOR_RIGHT_COLOR: Color = AUTO_LINK_COLOR;
/// The zones of a level bar, and the clip light.
pub(super) const LEVEL_GREEN_COLOR: Color = CONTROL_COLOR;
pub(super) const LEVEL_YELLOW_COLOR: Color = Color::from_rgb(0.95, 0.82, 0.25);
pub(super) const LEVEL_RED_COLOR: Color = Color::from_rgb(0.92, 0.28, 0.25);
/// The points of a vectorscope, soft so a dense cloud builds up.
pub(super) const VECTORSCOPE_DOT_COLOR: Color = Color::from_rgba(0.35, 0.85, 0.45, 0.55);
/// The Sampler's playhead, a fourth color distinct from its three marks.
pub(super) const PLAYHEAD_COLOR: Color = SELECTION_COLOR;
pub(super) const TEXT_COLOR: Color = Color::WHITE;
pub(super) const DIM_TEXT_COLOR: Color = Color::from_rgb(0.55, 0.55, 0.58);
pub(super) const TERMINAL_COLOR: Color = Color::from_rgb(0.75, 0.75, 0.80);
pub(super) const WIRE_COLOR: Color = Color::from_rgb(0.85, 0.85, 0.90);
/// Every control connection is this green.
pub(super) const CONTROL_COLOR: Color = Color::from_rgb(0.35, 0.85, 0.45);
/// Notes, the MIDI signal an input node sends to an oscillator, are purple.
pub(super) const NOTES_COLOR: Color = Color::from_rgb(0.70, 0.48, 0.95);
/// A table, the frames of a wavetable and the wavetable itself, is rose. Purple is notes, so the
/// table gets a hue of its own, and its pins are diamonds so the shape tells it apart too.
pub(super) const TABLE_COLOR: Color = Color::from_rgb(0.95, 0.40, 0.56);
/// The colors of the eight frame slots, on a wavetable node's pins and the regions of its ramp:
/// sky, amber, teal, vermilion, yellow, blue, mint and sand.
pub(super) const FRAME_COLORS: [Color; 8] = [
    Color::from_rgb(0.34, 0.71, 0.91),
    Color::from_rgb(0.91, 0.66, 0.22),
    Color::from_rgb(0.18, 0.77, 0.71),
    Color::from_rgb(0.89, 0.34, 0.18),
    Color::from_rgb(0.94, 0.89, 0.26),
    Color::from_rgb(0.23, 0.49, 0.88),
    Color::from_rgb(0.61, 0.88, 0.36),
    Color::from_rgb(0.79, 0.72, 0.61),
];
pub(super) const PENDING_WIRE_COLOR: Color = Color::from_rgb(0.50, 0.50, 0.55);
pub(super) const PILL_COLOR: Color = Color::from_rgb(0.17, 0.175, 0.215);
pub(super) const PILL_DIVIDER_COLOR: Color = Color::from_rgb(0.38, 0.39, 0.46);
pub(super) const MENU_COLOR: Color = Color::from_rgb(0.195, 0.20, 0.25);
pub(super) const DROPDOWN_COLOR: Color = Color::from_rgb(0.15, 0.155, 0.195);
/// A dropdown and its list rows lighten under the cursor.
pub(super) const DROPDOWN_HOVER_COLOR: Color = Color::from_rgb(0.22, 0.23, 0.29);
pub(super) const LIST_ROW_HOVER_COLOR: Color = Color::from_rgb(0.25, 0.26, 0.33);
/// The line of an auto connect drag, and the dot where it starts.
pub(super) const AUTO_LINK_COLOR: Color = Color::from_rgb(0.96, 0.65, 0.23);
pub(super) const HIGHLIGHT_WIRE_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.25);
/// The farthest a wire's handles reach from a pin when the pins are close together.
pub(super) const MIN_WIRE_REACH: f32 = 60.0;
/// Points sampled along a wire when checking whether a node sits on it.
pub(super) const WIRE_SAMPLES: usize = 24;
pub(super) const CHECKBOX_CHECKED_COLOR: Color = Color::from_rgb(0.19, 0.50, 0.90);
