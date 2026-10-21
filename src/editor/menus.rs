//! The pill of add buttons, the category lists, dropdown lists, and the right-click menu: the
//! chrome that floats above the nodes.

use crate::groups::GroupEntry;
use crate::nodes::{Bypass, Category, FlodeNode, NodeKind, Port};
use crate::presets::Library;
use nih_plug_iced::{Point, Rectangle};

use super::graph::PresetMenu;
use super::group::PinSide;
use super::style::*;

/// The add buttons on the pill, from left to right.
/// What a pill button does: add a node kind, or open a category's list below the pill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PillItem {
    Kind(NodeKind),
    Category(Category),
    /// Opens the list of presets, with the entry that saves one.
    Presets,
    /// Opens the list of saved groups.
    Groups,
}

/// The pill's buttons, from left to right.
pub(super) const PILL_ITEMS: [PillItem; 6] = [
    PillItem::Kind(NodeKind::Oscillator),
    PillItem::Category(Category::Driver),
    PillItem::Category(Category::Util),
    PillItem::Category(Category::Effects),
    PillItem::Presets,
    PillItem::Groups,
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

/// How many saved groups the list under the Groups button shows. It stops short of the key
/// strip, like the preset list, and the rest are cut off.
pub(super) fn group_rows_shown(bounds: Rectangle, total: usize) -> usize {
    total.min(preset_room(bounds))
}

/// The list under the Groups button, one row per saved group. With none saved it has one row that
/// says so.
pub(super) fn group_list_bounds(bounds: Rectangle, total: usize) -> Rectangle {
    let index = PILL_ITEMS
        .iter()
        .position(|item| *item == PillItem::Groups)
        .unwrap_or(0);
    let segment = pill_segment(bounds, index);
    let width = segment.width.max(PRESET_LIST_MIN_WIDTH);
    Rectangle {
        // The list lines up with the pill's right end, so it stays inside the window.
        x: segment.x + segment.width - width,
        y: segment.y + segment.height + LIST_GAP,
        width,
        height: group_rows_shown(bounds, total).max(1) as f32 * MENU_ITEM_HEIGHT,
    }
}

/// The saved group whose row of the list is under `point`. The note row for an empty list is not
/// a row.
pub(super) fn group_row_at(bounds: Rectangle, total: usize, point: Point) -> Option<usize> {
    let list = group_list_bounds(bounds, total);
    if !list.contains(point) {
        return None;
    }
    let row = ((point.y - list.y) / MENU_ITEM_HEIGHT) as usize;
    (row < group_rows_shown(bounds, total)).then_some(row)
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
    /// Shows the graph inside a group node.
    Open,
    /// Names a group node, a pin, or a macro knob, whichever the menu is for.
    Rename,
    /// Breaks a group node back into its nodes.
    Ungroup,
    /// Makes a group of the selected nodes. Offered only for several nodes.
    Group,
    /// Takes a pin off a group.
    RemovePin,
    /// Saves a group node to a file, to load into other patches.
    SaveGroup,
}

impl MenuItem {
    /// The text on the entry. A switched-off node offers to switch back on.
    pub(super) fn label(self, node: &FlodeNode) -> &'static str {
        match (self, node.bypass(), node.bypassed) {
            (MenuItem::Duplicate, _, _) => "Duplicate",
            (MenuItem::AutoPack, _, _) => "Auto pack",
            (MenuItem::Delete, _, _) => "Delete",
            (MenuItem::Open, _, _) => "Open",
            (MenuItem::Rename, _, _) => "Rename",
            (MenuItem::Ungroup, _, _) => "Ungroup",
            (MenuItem::Group, _, _) => "Group",
            (MenuItem::RemovePin, _, _) => "Remove pin",
            (MenuItem::SaveGroup, _, _) => "Save group",
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
    if node.kind == NodeKind::Group {
        return &[
            MenuItem::Open,
            MenuItem::Rename,
            MenuItem::Ungroup,
            MenuItem::Duplicate,
            MenuItem::SaveGroup,
            MenuItem::Delete,
        ];
    }
    match (node.can_duplicate(), node.bypass() != Bypass::None) {
        (true, true) => &[MenuItem::Duplicate, MenuItem::Bypass, MenuItem::Delete],
        (true, false) => &[MenuItem::Duplicate, MenuItem::Delete],
        (false, true) => &[MenuItem::Bypass, MenuItem::Delete],
        (false, false) => &[MenuItem::Delete],
    }
}

/// The entries for a node in a group of several selected nodes, which add Auto pack above Delete.
/// They add Group above that when `groupable` says some selected node can go in a group.
pub(super) fn group_menu_items(node: &FlodeNode, groupable: bool) -> &'static [MenuItem] {
    match (
        node.can_duplicate(),
        node.bypass() != Bypass::None,
        groupable,
    ) {
        (true, true, false) => &[
            MenuItem::Duplicate,
            MenuItem::Bypass,
            MenuItem::AutoPack,
            MenuItem::Delete,
        ],
        (true, true, true) => &[
            MenuItem::Duplicate,
            MenuItem::Bypass,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::Delete,
        ],
        (true, false, false) => &[MenuItem::Duplicate, MenuItem::AutoPack, MenuItem::Delete],
        (true, false, true) => &[
            MenuItem::Duplicate,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::Delete,
        ],
        (false, true, false) => &[MenuItem::Bypass, MenuItem::AutoPack, MenuItem::Delete],
        (false, true, true) => &[
            MenuItem::Bypass,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::Delete,
        ],
        (false, false, false) => &[MenuItem::AutoPack, MenuItem::Delete],
        (false, false, true) => &[MenuItem::Group, MenuItem::AutoPack, MenuItem::Delete],
    }
}

/// The entries of the menu on a pin of a group.
pub(super) const PIN_MENU_ITEMS: &[MenuItem] = &[MenuItem::Rename, MenuItem::RemovePin];

/// The entries of the menu on a row of a macro node.
pub(super) const MACRO_MENU_ITEMS: &[MenuItem] = &[MenuItem::Rename, MenuItem::Delete];

/// What a right-click menu was opened on, besides the node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MenuTarget {
    /// The node as a whole.
    Node,
    /// A pin of a group. The node is the group node, or the edge node inside the group.
    Pin(PinSide, usize),
    /// A row of a macro node. It is the number of the macro, from 0.
    Macro(usize),
}

/// The right-click menu for one node. `position` is its top-left corner in screen coordinates, so
/// the menu keeps one size and one place on screen whatever the zoom and pan are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ContextMenu {
    pub(super) node: usize,
    pub(super) position: Point,
    pub(super) items: &'static [MenuItem],
    pub(super) target: MenuTarget,
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
    /// The saved groups the box lists beside the node kinds, read when it opened.
    pub(super) groups: Vec<QuickGroup>,
    /// True until the first key after opening. The A that opened the box also arrives as a typed
    /// character, and this drops it.
    pub(super) skip_char: bool,
}

/// A saved group in the quick add box.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct QuickGroup {
    pub(super) entry: GroupEntry,
    /// The output pin of the group that feeds the input a wire was dragged back from, when the box
    /// opened from that drag. A group with no such pin is not listed then.
    pub(super) out: Option<Port>,
}

/// One result of the quick add box: a node kind, or a saved group by its place in
/// `QuickAdd::groups`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuickPick {
    Kind(NodeKind),
    Group(usize),
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

/// The kinds and saved groups that match `query`, best first. Among equals the kinds come first,
/// in the menu's order, then the groups in the order given. An empty query lists everything, so
/// the kinds fill the rows. `allowed` drops the kinds that can't be added, such as a second
/// output.
pub(super) fn quick_add_picks(
    query: &str,
    allowed: impl Fn(NodeKind) -> bool,
    groups: &[QuickGroup],
) -> Vec<QuickPick> {
    let query = query.trim().to_lowercase();
    let kinds = NodeKind::ALL
        .iter()
        .copied()
        .filter(|&kind| allowed(kind))
        .map(|kind| (QuickPick::Kind(kind), kind.title()));
    let saved = groups
        .iter()
        .enumerate()
        .map(|(index, group)| (QuickPick::Group(index), group.entry.name.as_str()));
    let mut found: Vec<(u8, QuickPick)> = kinds
        .chain(saved)
        .filter_map(|(pick, title)| {
            let rank = if query.is_empty() {
                0
            } else {
                match_rank(&query, title)?
            };
            Some((rank, pick))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found
        .into_iter()
        .map(|(_, pick)| pick)
        .take(QUICK_ADD_ROWS)
        .collect()
}

/// The node kinds `quick_add_picks` lists when no group is saved. The tests that check which
/// kinds a box offers read it.
#[cfg(test)]
pub(super) fn quick_add_results(query: &str, allowed: impl Fn(NodeKind) -> bool) -> Vec<NodeKind> {
    quick_add_picks(query, allowed, &[])
        .into_iter()
        .filter_map(|pick| match pick {
            QuickPick::Kind(kind) => Some(kind),
            QuickPick::Group(_) => None,
        })
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
