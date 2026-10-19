//! The pill of add buttons, the category lists, dropdown lists, and the right-click menu: the
//! chrome that floats above the nodes.

use crate::nodes::{Bypass, Category, FlodeNode, NodeKind, Port};
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
    /// Goes back to the graph a new instance starts with.
    Init,
    /// A category, which opens its submenu. The position is in `library.categories`.
    Category(usize),
    /// A preset with no category. The position is in `library.loose`.
    Loose(usize),
    /// A preset in an open submenu: the category's position, then the preset's in that category.
    Sub(usize, usize),
}

impl PresetRow {
    /// The line of the preset list this row is on, counting from the top, when the list shows
    /// `categories` categories. A row in a submenu is on a line of the submenu, not the list.
    pub(super) fn list_line(self, categories: usize) -> Option<usize> {
        match self {
            PresetRow::SaveAs => Some(0),
            PresetRow::Init => Some(1),
            PresetRow::Category(index) => Some(PRESET_ACTION_ROWS + index),
            PresetRow::Loose(index) => Some(PRESET_ACTION_ROWS + categories + index),
            PresetRow::Sub(..) => None,
        }
    }
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

/// The rows at the top of the preset list that are not presets: "Save as..." and "Init".
pub(super) const PRESET_ACTION_ROWS: usize = 2;

/// How many categories and how many loose presets the main list shows. The two action rows come
/// first, then the categories, then as many loose presets as fit.
pub(super) fn preset_rows_shown(bounds: Rectangle, library: &Library) -> (usize, usize) {
    let room = preset_room(bounds) - PRESET_ACTION_ROWS;
    let categories = library.categories.len().min(room);
    let loose = library.loose.len().min(room - categories);
    (categories, loose)
}

/// The list under the Presets button: the save and init entries, the categories, then the presets
/// that have no category. With none saved it has a row that says so.
pub(super) fn preset_list_bounds(bounds: Rectangle, library: &Library) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Presets)
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    let (categories, loose) = preset_rows_shown(bounds, library);
    let rows = PRESET_ACTION_ROWS + (categories + loose).max(1);
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
    let level = list.y + (PRESET_ACTION_ROWS + category) as f32 * MENU_ITEM_HEIGHT;
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
        1 => Some(PresetRow::Init),
        row if row < PRESET_ACTION_ROWS + categories => {
            Some(PresetRow::Category(row - PRESET_ACTION_ROWS))
        }
        row if row < PRESET_ACTION_ROWS + categories + loose => {
            Some(PresetRow::Loose(row - PRESET_ACTION_ROWS - categories))
        }
        _ => None,
    }
}

/// The most rows of category chips the save box shows. Categories past that are not drawn, but
/// the arrow keys still reach them.
const MAX_CHIP_ROWS: usize = 3;

/// How the save box is laid out inside: where each existing category's chip sits, and where the
/// hint line and the bottom edge are. Everything is measured from the box's top left corner.
pub(super) struct NameBoxLayout {
    /// For each chip drawn: the position of its category in the list, and its rectangle.
    pub(super) chips: Vec<(usize, Rectangle)>,
    /// The height of the label over the chips, or None when there are no chips.
    pub(super) chips_label_y: Option<f32>,
    pub(super) hint_y: f32,
    pub(super) height: f32,
}

/// The width of a category's chip. The font is proportional, so this is an estimate from the
/// number of characters.
fn chip_width(name: &str) -> f32 {
    name.chars().count() as f32 * 6.6 + 20.0
}

/// Lays out the save box for a list of existing categories. The chips wrap onto rows that fit
/// the box's width.
pub(super) fn name_box_layout(categories: &[String]) -> NameBoxLayout {
    if categories.is_empty() {
        return NameBoxLayout {
            chips: Vec::new(),
            chips_label_y: None,
            hint_y: 116.0,
            height: NAME_BOX_HEIGHT,
        };
    }
    let left = 12.0;
    let right = NAME_BOX_WIDTH - 12.0;
    let first_row = 122.0;
    let mut chips = Vec::new();
    let (mut x, mut row) = (left, 0);
    for (index, name) in categories.iter().enumerate() {
        let width = chip_width(name);
        if x + width > right && x > left {
            x = left;
            row += 1;
        }
        if row >= MAX_CHIP_ROWS {
            break;
        }
        chips.push((
            index,
            Rectangle {
                x,
                y: first_row + row as f32 * (CHIP_HEIGHT + CHIP_GAP),
                width,
                height: CHIP_HEIGHT,
            },
        ));
        x += width + CHIP_GAP;
    }
    let rows = row + 1;
    let hint_y = first_row + rows as f32 * (CHIP_HEIGHT + CHIP_GAP) + 6.0;
    NameBoxLayout {
        chips,
        chips_label_y: Some(108.0),
        hint_y,
        height: hint_y + 20.0,
    }
}

/// The box where a preset is named and sorted. It sits under the pill, in the middle, and is
/// taller when there are existing categories to pick from.
pub(super) fn name_box_bounds(bounds: Rectangle, categories: &[String]) -> Rectangle {
    Rectangle {
        x: (bounds.width - NAME_BOX_WIDTH) / 2.0,
        y: pill_bounds(bounds).y + PILL_HEIGHT + LIST_GAP + 6.0,
        width: NAME_BOX_WIDTH,
        height: name_box_layout(categories).height,
    }
}

/// The existing category whose chip is under `point`, as its position in `categories`.
pub(super) fn category_chip_at(
    bounds: Rectangle,
    categories: &[String],
    point: Point,
) -> Option<usize> {
    let area = name_box_bounds(bounds, categories);
    name_box_layout(categories)
        .chips
        .into_iter()
        .find(|(_, chip)| {
            Rectangle {
                x: area.x + chip.x,
                y: area.y + chip.y,
                ..*chip
            }
            .contains(point)
        })
        .map(|(index, _)| index)
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
    /// Arranges the selected nodes by how they are wired. Offered only for several nodes.
    AutoPack,
    Delete,
}

impl MenuItem {
    /// The text on the entry. A switched-off node offers to switch back on.
    pub(super) fn label(self, node: &FlodeNode) -> &'static str {
        match (self, node.bypass(), node.bypassed) {
            (MenuItem::Duplicate, _, _) => "Duplicate",
            (MenuItem::AutoPack, _, _) => "Auto pack",
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

/// The entries for a node in a group of several selected nodes, which add Auto pack above Delete.
pub(super) fn group_menu_items(node: &FlodeNode) -> &'static [MenuItem] {
    match (node.can_duplicate(), node.bypass() != Bypass::None) {
        (true, true) => &[
            MenuItem::Duplicate,
            MenuItem::Bypass,
            MenuItem::AutoPack,
            MenuItem::Delete,
        ],
        (true, false) => &[MenuItem::Duplicate, MenuItem::AutoPack, MenuItem::Delete],
        (false, true) => &[MenuItem::Bypass, MenuItem::AutoPack, MenuItem::Delete],
        (false, false) => &[MenuItem::AutoPack, MenuItem::Delete],
    }
}

/// The right-click menu for one node. `position` is its top-left corner in screen coordinates, so
/// the menu keeps one size and one place on screen whatever the zoom and pan are.
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

/// The most results the quick add box lists.
pub(super) const QUICK_ADD_ROWS: usize = 8;
/// The quick add box is this wide, and its search field is this tall.
pub(super) const QUICK_ADD_WIDTH: f32 = 240.0;
pub(super) const QUICK_ADD_FIELD_HEIGHT: f32 = 36.0;

/// The quick add box, opened with Shift+A. It holds what is typed and which result is lit.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct QuickAdd {
    pub(super) query: String,
    /// The lit result, as a position in the results.
    pub(super) selected: usize,
    /// Where the box opened, in screen coordinates. The box keeps inside the editor.
    pub(super) screen: Point,
    /// Where the node goes, in editor coordinates. It is where the cursor was when the box opened.
    pub(super) world: Point,
    /// The input a wire was dragged back from, as its node and port, when the box opened from
    /// that drag. The box then lists only the kinds that can feed the input, and the node it adds
    /// is wired to it.
    pub(super) feed: Option<(usize, Port)>,
    /// True until the first key after opening. The A that opened the box also arrives as a typed
    /// character, and this drops it.
    pub(super) skip_char: bool,
}

/// How well `query` matches `title`, lower being better, or None for no match. The best match is
/// a title that starts with the query, then one with a word that does, then one that contains it,
/// and last one that has the query's letters in order.
fn match_rank(query: &str, title: &str) -> Option<u8> {
    let title = title.to_lowercase();
    if title.starts_with(query) {
        Some(0)
    } else if title.split_whitespace().any(|word| word.starts_with(query)) {
        Some(1)
    } else if title.contains(query) {
        Some(2)
    } else {
        let mut letters = title.chars();
        query.chars().all(|c| letters.any(|t| t == c)).then_some(3)
    }
}

/// The kinds that match `query`, best first, and in the menu's order among equals. An empty query
/// lists every kind. `allowed` drops the kinds that can't be added, such as a second output.
pub(super) fn quick_add_results(query: &str, allowed: impl Fn(NodeKind) -> bool) -> Vec<NodeKind> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<(u8, NodeKind)> = NodeKind::ALL
        .iter()
        .copied()
        .filter(|&kind| allowed(kind))
        .filter_map(|kind| {
            let rank = if query.is_empty() {
                0
            } else {
                match_rank(&query, kind.title())?
            };
            Some((rank, kind))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found
        .into_iter()
        .map(|(_, kind)| kind)
        .take(QUICK_ADD_ROWS)
        .collect()
}

/// Where the quick add box sits for `rows` results: its corner at the cursor, moved to keep the
/// whole box inside the editor. An empty list still has a row, which says nothing matched.
pub(super) fn quick_add_bounds(bounds: Rectangle, at: Point, rows: usize) -> Rectangle {
    let height = QUICK_ADD_FIELD_HEIGHT + rows.max(1) as f32 * MENU_ITEM_HEIGHT;
    Rectangle {
        x: at.x.min(bounds.width - QUICK_ADD_WIDTH).max(0.0),
        y: at.y.min(bounds.height - height).max(0.0),
        width: QUICK_ADD_WIDTH,
        height,
    }
}

/// The result row under `point`, as a position in the results.
pub(super) fn quick_add_row_at(area: Rectangle, point: Point, rows: usize) -> Option<usize> {
    let top = area.y + QUICK_ADD_FIELD_HEIGHT;
    if !area.contains(point) || point.y < top {
        return None;
    }
    let row = ((point.y - top) / MENU_ITEM_HEIGHT) as usize;
    (row < rows).then_some(row)
}

#[cfg(test)]
mod quick_add_tests {
    use super::*;

    fn all(_: NodeKind) -> bool {
        true
    }

    /// An empty query lists kinds, capped at the row limit.
    #[test]
    fn empty_query_lists_kinds() {
        assert_eq!(quick_add_results("", all).len(), QUICK_ADD_ROWS);
    }

    /// A prefix beats a substring, and matching ignores case.
    #[test]
    fn prefix_ranks_first() {
        let found = quick_add_results("FL", all);
        assert_eq!(found[0], NodeKind::Flanger);
        assert!(found.contains(&NodeKind::Filter));
    }

    /// Letters in order find a kind, and letters that are not there find nothing.
    #[test]
    fn letters_in_order_match() {
        assert!(quick_add_results("dst", all).contains(&NodeKind::Distortion));
        assert!(quick_add_results("zzz", all).is_empty());
    }

    /// A kind the graph can't take is left out.
    #[test]
    fn disallowed_kinds_are_dropped() {
        let found = quick_add_results("out", |kind| kind != NodeKind::Output);
        assert!(!found.contains(&NodeKind::Output));
    }

    /// The box keeps inside the editor.
    #[test]
    fn box_stays_in_bounds() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 712.0,
        };
        let area = quick_add_bounds(bounds, Point::new(1000.0, 700.0), 8);
        assert!(area.x + area.width <= 1024.0 && area.y + area.height <= 712.0);
    }
}
