//! The pill of add buttons, the category lists, dropdown lists, and the right-click menu: the
//! chrome that floats above the nodes.

use crate::nodes::{Bypass, Category, FlodeNode, NodeKind};
use nih_plug_iced::{Point, Rectangle};

use super::style::*;

/// The add buttons on the pill, from left to right.
/// What a pill button does: add a node kind, or open a category's list below the pill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PillItem {
    Kind(NodeKind),
    Category(Category),
    /// Opens the list of presets, with the entry that saves one.
    Presets,
}

/// The pill's buttons, from left to right.
pub(super) const PILL_ITEMS: [PillItem; 5] = [
    PillItem::Kind(NodeKind::Oscillator),
    PillItem::Category(Category::Driver),
    PillItem::Category(Category::Util),
    PillItem::Category(Category::Effects),
    PillItem::Presets,
];

/// A row of the preset list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PresetRow {
    /// Opens the box that names and saves the current graph.
    SaveAs,
    /// Loads the saved preset at this position in the list.
    Load(usize),
}

/// The widest a preset name is drawn on the pill, in characters.
pub(super) const PILL_NAME_CHARS: usize = 10;

/// A preset name cut to fit the pill, with an ellipsis if it was longer.
pub(super) fn shorten(name: &str) -> String {
    if name.chars().count() <= PILL_NAME_CHARS {
        name.to_string()
    } else {
        let head: String = name.chars().take(PILL_NAME_CHARS - 1).collect();
        format!("{head}…")
    }
}

/// How many saved names the list shows. It stops short of the key strip, and always has room for
/// a few.
pub(super) fn preset_names_shown(bounds: Rectangle, names: usize) -> usize {
    let top = pill_bounds(bounds).y + PILL_HEIGHT + LIST_GAP;
    let room = (bounds.height - super::piano::STRIP_HEIGHT - top) / MENU_ITEM_HEIGHT;
    // One row of the room is the save entry.
    names.min((room.floor() as usize).saturating_sub(1).max(3))
}

/// The list under the Presets button: the save entry, then the saved names. With none saved it
/// has a row that says so.
pub(super) fn preset_list_bounds(bounds: Rectangle, names: usize) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Presets)
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    let rows = 1 + preset_names_shown(bounds, names).max(1);
    let width = segment.width.max(PRESET_LIST_MIN_WIDTH);
    Rectangle {
        // The list lines up with the pill's right end, so it stays inside the window.
        x: segment.x + segment.width - width,
        y: segment.y + segment.height + LIST_GAP,
        width,
        height: rows as f32 * MENU_ITEM_HEIGHT,
    }
}

/// The row of the preset list under `point`. The note row for an empty list is not a row.
pub(super) fn preset_row_at(bounds: Rectangle, names: usize, point: Point) -> Option<PresetRow> {
    let list = preset_list_bounds(bounds, names);
    if !list.contains(point) {
        return None;
    }
    let row = ((point.y - list.y) / MENU_ITEM_HEIGHT) as usize;
    match row {
        0 => Some(PresetRow::SaveAs),
        row if row <= preset_names_shown(bounds, names) => Some(PresetRow::Load(row - 1)),
        _ => None,
    }
}

/// The box where a preset's name is typed. It sits under the pill, in the middle.
pub(super) fn name_box_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: (bounds.width - NAME_BOX_WIDTH) / 2.0,
        y: pill_bounds(bounds).y + PILL_HEIGHT + LIST_GAP + 6.0,
        width: NAME_BOX_WIDTH,
        height: NAME_BOX_HEIGHT,
    }
}
/// The list that opens below a dropdown, with one row per label.
pub(super) fn dropdown_list_bounds(dropdown: Rectangle, rows: usize) -> Rectangle {
    Rectangle {
        x: dropdown.x,
        y: dropdown.y + dropdown.height + LIST_GAP,
        width: dropdown.width,
        height: rows as f32 * WAVEFORM_ITEM_HEIGHT,
    }
}

/// The index of the list row under `point`, if any.
pub(super) fn list_row_at(list: Rectangle, point: Point, rows: usize) -> Option<usize> {
    if !list.contains(point) {
        return None;
    }
    let index = ((point.y - list.y) / WAVEFORM_ITEM_HEIGHT) as usize;
    (index < rows).then_some(index)
}

/// One entry of the right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuItem {
    /// Copies the node, with its settings and without its wires.
    Duplicate,
    /// Mutes a source, or bypasses a processor. It is the same action as the `m` key.
    Bypass,
    Delete,
}

impl MenuItem {
    /// The text on the entry. A switched-off node offers to switch back on.
    pub(super) fn label(self, node: &FlodeNode) -> &'static str {
        match (self, node.bypass(), node.bypassed) {
            (MenuItem::Duplicate, _, _) => "Duplicate",
            (MenuItem::Delete, _, _) => "Delete",
            (MenuItem::Bypass, Bypass::Mute, false) => "Mute",
            (MenuItem::Bypass, Bypass::Mute, true) => "Unmute",
            (MenuItem::Bypass, _, false) => "Bypass",
            (MenuItem::Bypass, _, true) => "Enable",
        }
    }
}

/// The entries a node's menu offers, top to bottom. Every node can be deleted. Most can be
/// duplicated, and the ones that can be switched off also offer that.
pub(super) fn menu_items(node: &FlodeNode) -> &'static [MenuItem] {
    match (node.can_duplicate(), node.bypass() != Bypass::None) {
        (true, true) => &[MenuItem::Duplicate, MenuItem::Bypass, MenuItem::Delete],
        (true, false) => &[MenuItem::Duplicate, MenuItem::Delete],
        (false, true) => &[MenuItem::Bypass, MenuItem::Delete],
        (false, false) => &[MenuItem::Delete],
    }
}

/// The right-click menu for one node. `position` is its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ContextMenu {
    pub(super) node: usize,
    pub(super) position: Point,
    pub(super) items: &'static [MenuItem],
}

/// The pill at the top center of the editor.
pub(super) fn pill_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: (bounds.width - PILL_WIDTH) / 2.0,
        y: PILL_TOP,
        width: PILL_WIDTH,
        height: PILL_HEIGHT,
    }
}

/// The part of the pill under `point`.
pub(super) fn pill_button_at(bounds: Rectangle, point: Point) -> Option<PillItem> {
    let pill = pill_bounds(bounds);
    if !pill.contains(point) {
        return None;
    }
    let segment = pill.width / PILL_ITEMS.len() as f32;
    let index = (((point.x - pill.x) / segment) as usize).min(PILL_ITEMS.len() - 1);
    Some(PILL_ITEMS[index])
}

/// The rectangle of one pill button.
pub(super) fn pill_segment(bounds: Rectangle, index: usize) -> Rectangle {
    let pill = pill_bounds(bounds);
    let segment = pill.width / PILL_ITEMS.len() as f32;
    Rectangle {
        x: pill.x + index as f32 * segment,
        y: pill.y,
        width: segment,
        height: pill.height,
    }
}

/// The list that opens below a category's pill button.
pub(super) fn category_list_bounds(bounds: Rectangle, category: Category) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Category(category))
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    Rectangle {
        x: segment.x,
        y: segment.y + segment.height + LIST_GAP,
        width: segment.width.max(CATEGORY_MIN_WIDTH),
        height: category.kinds().len().max(1) as f32 * MENU_ITEM_HEIGHT,
    }
}

/// The node kind for the row of a category's list under `point`, if any.
pub(super) fn category_row_at(
    bounds: Rectangle,
    category: Category,
    point: Point,
) -> Option<NodeKind> {
    let list = category_list_bounds(bounds, category);
    if !list.contains(point) {
        return None;
    }
    let index = ((point.y - list.y) / MENU_ITEM_HEIGHT) as usize;
    category.kinds().get(index).copied()
}

pub(super) fn menu_bounds(menu: ContextMenu) -> Rectangle {
    Rectangle {
        x: menu.position.x,
        y: menu.position.y,
        width: MENU_WIDTH,
        height: menu.items.len() as f32 * MENU_ITEM_HEIGHT,
    }
}

/// The entry of the right-click menu under `point`.
pub(super) fn menu_item_at(menu: ContextMenu, point: Point) -> Option<MenuItem> {
    let bounds = menu_bounds(menu);
    if !bounds.contains(point) {
        return None;
    }
    let row = ((point.y - bounds.y) / MENU_ITEM_HEIGHT) as usize;
    menu.items.get(row).copied()
}
