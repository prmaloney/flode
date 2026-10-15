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

pub(super) const PILL_WIDTH: f32 = 480.0;
pub(super) const PILL_HEIGHT: f32 = 36.0;
pub(super) const PILL_TOP: f32 = 12.0;

/// The narrowest a category list is, so short labels still get a usable row.
pub(super) const CATEGORY_MIN_WIDTH: f32 = 160.0;

pub(super) const MENU_WIDTH: f32 = 150.0;
pub(super) const MENU_ITEM_HEIGHT: f32 = 32.0;
pub(super) const MENU_CORNER_RADIUS: f32 = 6.0;

pub(super) const NODE_BODY_COLOR: Color = Color::from_rgb(0.24, 0.24, 0.27);
pub(super) const NODE_HEADER_COLOR: Color = Color::from_rgb(0.30, 0.30, 0.36);
/// The body and header of a node that is muted or bypassed, darker than a live node's.
pub(super) const NODE_OFF_BODY_COLOR: Color = Color::from_rgb(0.19, 0.19, 0.21);
pub(super) const NODE_OFF_HEADER_COLOR: Color = Color::from_rgb(0.23, 0.23, 0.26);
pub(super) const SELECTION_COLOR: Color = Color::WHITE;
pub(super) const SELECTION_WIDTH: f32 = 2.0;
pub(super) const TEXT_COLOR: Color = Color::WHITE;
pub(super) const DIM_TEXT_COLOR: Color = Color::from_rgb(0.55, 0.55, 0.58);
pub(super) const TERMINAL_COLOR: Color = Color::from_rgb(0.75, 0.75, 0.80);
pub(super) const WIRE_COLOR: Color = Color::from_rgb(0.85, 0.85, 0.90);
/// Every non-audio connection is this green.
pub(super) const CONTROL_COLOR: Color = Color::from_rgb(0.35, 0.85, 0.45);
pub(super) const PENDING_WIRE_COLOR: Color = Color::from_rgb(0.50, 0.50, 0.55);
pub(super) const PILL_COLOR: Color = Color::from_rgb(0.18, 0.18, 0.21);
pub(super) const PILL_DIVIDER_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.45);
pub(super) const MENU_COLOR: Color = Color::from_rgb(0.20, 0.20, 0.24);
pub(super) const DROPDOWN_COLOR: Color = Color::from_rgb(0.16, 0.16, 0.19);
/// A dropdown and its list rows lighten under the cursor.
pub(super) const DROPDOWN_HOVER_COLOR: Color = Color::from_rgb(0.23, 0.23, 0.28);
pub(super) const LIST_ROW_HOVER_COLOR: Color = Color::from_rgb(0.26, 0.26, 0.32);
pub(super) const HIGHLIGHT_WIRE_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.25);
/// Points sampled along a wire when checking whether a node sits on it.
pub(super) const WIRE_SAMPLES: usize = 24;
pub(super) const CHECKBOX_CHECKED_COLOR: Color = Color::from_rgb(0.19, 0.50, 0.90);
