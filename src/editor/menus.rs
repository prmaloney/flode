//! The pill of add buttons, the category lists, dropdown lists, and the right-click menu: the
//! chrome that floats above the nodes.

use crate::nodes::{Bypass, Category, FlodeNode, NodeKind};
use crate::presets::Library;
use nih_plug_iced::{Point, Rectangle};

use super::graph::PresetMenu;
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

/// A row of the preset list or of a category's submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PresetRow {
    /// Opens the box that names and saves the current graph.
    SaveAs,
    /// A category, which opens its submenu. The position is in `library.categories`.
    Category(usize),
    /// A preset with no category. The position is in `library.loose`.
    Loose(usize),
    /// A preset in an open submenu: the category's position, then the preset's in that category.
    Sub(usize, usize),
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

/// How many rows a preset list or submenu can have. It stops short of the key strip, and always
/// has room for a few.
fn preset_room(bounds: Rectangle) -> usize {
    let top = pill_bounds(bounds).y + PILL_HEIGHT + LIST_GAP;
    let room = (bounds.height - super::piano::STRIP_HEIGHT - top) / MENU_ITEM_HEIGHT;
    (room.floor() as usize).max(4)
}

/// How many categories and how many loose presets the main list shows. The save entry takes the
/// first row, then the categories, then as many loose presets as fit.
pub(super) fn preset_rows_shown(bounds: Rectangle, library: &Library) -> (usize, usize) {
    let room = preset_room(bounds) - 1;
    let categories = library.categories.len().min(room);
    let loose = library.loose.len().min(room - categories);
    (categories, loose)
}

/// The list under the Presets button: the save entry, the categories, then the presets that have
/// no category. With none saved it has a row that says so.
pub(super) fn preset_list_bounds(bounds: Rectangle, library: &Library) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Presets)
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    let (categories, loose) = preset_rows_shown(bounds, library);
    let rows = 1 + (categories + loose).max(1);
    let width = segment.width.max(PRESET_LIST_MIN_WIDTH);
    Rectangle {
        // The list lines up with the pill's right end, so it stays inside the window.
        x: segment.x + segment.width - width,
        y: segment.y + segment.height + LIST_GAP,
        width,
        height: rows as f32 * MENU_ITEM_HEIGHT,
    }
}

/// How many presets a category's submenu shows.
pub(super) fn submenu_rows_shown(bounds: Rectangle, library: &Library, category: usize) -> usize {
    library
        .categories
        .get(category)
        .map_or(0, |c| c.presets.len().min(preset_room(bounds)))
}

/// The submenu of a category. It opens beside the list, level with the category's row. It goes to
/// the right of the list if there is room and to the left if not, and it is moved up if it would
/// run into the key strip.
pub(super) fn submenu_bounds(bounds: Rectangle, library: &Library, category: usize) -> Rectangle {
    let list = preset_list_bounds(bounds, library);
    let rows = submenu_rows_shown(bounds, library, category).max(1);
    let height = rows as f32 * MENU_ITEM_HEIGHT;
    let width = PRESET_LIST_MIN_WIDTH;
    let right = list.x + list.width + LIST_GAP;
    let x = if right + width <= bounds.width {
        right
    } else {
        list.x - width - LIST_GAP
    };
    let level = list.y + (1 + category) as f32 * MENU_ITEM_HEIGHT;
    let lowest = bounds.height - super::piano::STRIP_HEIGHT - height;
    Rectangle {
        x,
        y: level.min(lowest).max(list.y),
        width,
        height,
    }
}

/// The row of the preset list, or of the open submenu, under `point`. The note row for an empty
/// list is not a row.
pub(super) fn preset_row_at(
    bounds: Rectangle,
    menu: &PresetMenu,
    point: Point,
) -> Option<PresetRow> {
    let library = &menu.library;
    if let Some(open) = menu.open {
        let sub = submenu_bounds(bounds, library, open);
        if sub.contains(point) {
            let row = ((point.y - sub.y) / MENU_ITEM_HEIGHT) as usize;
            return (row < submenu_rows_shown(bounds, library, open))
                .then_some(PresetRow::Sub(open, row));
        }
    }
    let list = preset_list_bounds(bounds, library);
    if !list.contains(point) {
        return None;
    }
    let (categories, loose) = preset_rows_shown(bounds, library);
    match ((point.y - list.y) / MENU_ITEM_HEIGHT) as usize {
        0 => Some(PresetRow::SaveAs),
        row if row <= categories => Some(PresetRow::Category(row - 1)),
        row if row <= categories + loose => Some(PresetRow::Loose(row - 1 - categories)),
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
