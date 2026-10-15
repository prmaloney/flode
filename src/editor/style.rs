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

pub(super) const PILL_WIDTH: f32 = 600.0;
pub(super) const PILL_HEIGHT: f32 = 36.0;
pub(super) const PILL_TOP: f32 = 12.0;

/// The narrowest a category list is, so short labels still get a usable row.
pub(super) const CATEGORY_MIN_WIDTH: f32 = 160.0;

/// How long the line under the pill stays up.
pub(super) const STATUS_SECONDS: std::time::Duration = std::time::Duration::from_secs(3);
/// The preset list is at least this wide, so most names fit on a row.
pub(super) const PRESET_LIST_MIN_WIDTH: f32 = 220.0;
/// The box where a preset's name is typed.
pub(super) const NAME_BOX_WIDTH: f32 = 360.0;
pub(super) const NAME_BOX_HEIGHT: f32 = 136.0;
/// The tag on a category that does not exist yet.
pub(super) const NEW_TAG_COLOR: Color = Color::from_rgb(0.80, 0.50, 0.15);
/// The chips in the save box that pick an existing category.
pub(super) const CHIP_HEIGHT: f32 = 22.0;
pub(super) const CHIP_GAP: f32 = 6.0;

pub(super) const MENU_WIDTH: f32 = 150.0;
pub(super) const MENU_ITEM_HEIGHT: f32 = 32.0;
pub(super) const MENU_CORNER_RADIUS: f32 = 6.0;

pub(super) const NODE_BODY_COLOR: Color = Color::from_rgb(0.24, 0.24, 0.27);
pub(super) const NODE_HEADER_COLOR: Color = Color::from_rgb(0.30, 0.30, 0.36);
/// The band behind a section's title, darker than the node's body so it reads as a recess, with
/// light text and arrow on it.
pub(super) const SECTION_COLOR: Color = Color::from_rgb(0.17, 0.17, 0.20);
pub(super) const SECTION_TEXT_COLOR: Color = Color::from_rgb(0.82, 0.82, 0.88);
/// The pins of a folded section or node are drawn smaller than ordinary pins, so a tight stack of
/// them stays readable.
pub(super) const PACKED_PIN_RADIUS: f32 = 3.0;
/// The arrow of a section header sits this far from the node's left edge.
pub(super) const SECTION_ARROW_X: f32 = 18.0;
/// The body and header of a node that is muted or bypassed, darker than a live node's.
pub(super) const NODE_OFF_BODY_COLOR: Color = Color::from_rgb(0.19, 0.19, 0.21);
pub(super) const NODE_OFF_HEADER_COLOR: Color = Color::from_rgb(0.23, 0.23, 0.26);
pub(super) const SELECTION_COLOR: Color = Color::WHITE;
pub(super) const SELECTION_WIDTH: f32 = 2.0;
pub(super) const TEXT_COLOR: Color = Color::WHITE;
pub(super) const DIM_TEXT_COLOR: Color = Color::from_rgb(0.55, 0.55, 0.58);
pub(super) const TERMINAL_COLOR: Color = Color::from_rgb(0.75, 0.75, 0.80);
pub(super) const WIRE_COLOR: Color = Color::from_rgb(0.85, 0.85, 0.90);
/// Every control connection is this green.
pub(super) const CONTROL_COLOR: Color = Color::from_rgb(0.35, 0.85, 0.45);
/// Notes, the MIDI signal an input node sends to an oscillator, are purple.
pub(super) const NOTES_COLOR: Color = Color::from_rgb(0.70, 0.48, 0.95);
pub(super) const PENDING_WIRE_COLOR: Color = Color::from_rgb(0.50, 0.50, 0.55);
pub(super) const PILL_COLOR: Color = Color::from_rgb(0.18, 0.18, 0.21);
pub(super) const PILL_DIVIDER_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.45);
pub(super) const MENU_COLOR: Color = Color::from_rgb(0.20, 0.20, 0.24);
pub(super) const DROPDOWN_COLOR: Color = Color::from_rgb(0.16, 0.16, 0.19);
/// A dropdown and its list rows lighten under the cursor.
pub(super) const DROPDOWN_HOVER_COLOR: Color = Color::from_rgb(0.23, 0.23, 0.28);
pub(super) const LIST_ROW_HOVER_COLOR: Color = Color::from_rgb(0.26, 0.26, 0.32);
pub(super) const HIGHLIGHT_WIRE_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.25);
/// The farthest a wire's handles reach from a pin when the pins are close together.
pub(super) const MIN_WIRE_REACH: f32 = 60.0;
/// Points sampled along a wire when checking whether a node sits on it.
pub(super) const WIRE_SAMPLES: usize = 24;
pub(super) const CHECKBOX_CHECKED_COLOR: Color = Color::from_rgb(0.19, 0.50, 0.90);
