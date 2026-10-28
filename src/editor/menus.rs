//! The burger menu, the category lists, dropdown lists, and the right-click menu: the chrome that
//! floats above the nodes.

use crate::groups::GroupEntry;
use crate::nodes::{Bypass, Category, FlodeNode, NodeKind, Port};
use crate::presets::Library;
use nih_plug_iced::{Point, Rectangle};

use super::graph::PresetMenu;
use super::group::PinSide;
use super::logo::{LOGO_GAP, LOGO_MARGIN, LOGO_SIZE};
use super::style::*;

/// A row of the burger menu's top-level list, top to bottom. A divider is drawn between Utility
/// and Presets, but it is not a row of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum BurgerRow {
    /// Adds an oscillator directly, since it is added far more often than anything else.
    AddOscillator,
    /// Adds a sampler directly. It is a primary sound source the same way an oscillator is, and
    /// none of the four categories describe what it does, so it sits beside Add Oscillator
    /// instead of inside one of them.
    AddSampler,
    /// Opens a category's list of node kinds.
    Category(Category),
    /// Opens the list of presets, with the entry that saves one.
    Presets,
    /// Opens the list of saved groups.
    Groups,
}

/// The burger menu's top-level rows, in the order they are drawn.
pub(super) const BURGER_ROWS: [BurgerRow; 9] = [
    BurgerRow::AddOscillator,
    BurgerRow::AddSampler,
    BurgerRow::Category(Category::Midi),
    BurgerRow::Category(Category::Modulation),
    BurgerRow::Category(Category::Wavetable),
    BurgerRow::Category(Category::Effects),
    BurgerRow::Category(Category::Utility),
    BurgerRow::Presets,
    BurgerRow::Groups,
];

/// The row Presets sits on, and the one after it where Groups sits. The divider is drawn just
/// above the Presets row.
pub(super) const BURGER_PRESETS_ROW: usize = 7;
pub(super) const BURGER_GROUPS_ROW: usize = 8;

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

/// The widest a preset name is drawn on the Presets row, in characters.
pub(super) const PILL_NAME_CHARS: usize = 10;

/// A preset name cut to fit the Presets row, with an ellipsis if it was longer.
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
    let top = burger_row_bounds(BURGER_PRESETS_ROW).y;
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

/// The list under the Presets row: the save and init entries, the categories, then the presets
/// that have no category. With none saved it has a row that says so. It opens beside the Presets
/// row of the burger menu, the same way a category's submenu opens beside its own row.
pub(super) fn preset_list_bounds(bounds: Rectangle, library: &Library) -> Rectangle {
    let (categories, loose) = preset_rows_shown(bounds, library);
    let rows = PRESET_ACTION_ROWS + (categories + loose).max(1);
    beside_burger_row(bounds, BURGER_PRESETS_ROW, rows, PRESET_LIST_MIN_WIDTH)
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

/// The list under the Groups row, one row per saved group. With none saved it has one row that
/// says so. It opens beside the Groups row of the burger menu.
pub(super) fn group_list_bounds(bounds: Rectangle, total: usize) -> Rectangle {
    let rows = group_rows_shown(bounds, total).max(1);
    beside_burger_row(bounds, BURGER_GROUPS_ROW, rows, PRESET_LIST_MIN_WIDTH)
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
    /// Removes the node and reconnects whatever fed it straight to whatever it fed, so the chain
    /// it sat in keeps running. A node with no single through-path is just removed, same as
    /// Delete. The same trick is on Command+X in Edit mode.
    DeleteInPlace,
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
            (MenuItem::DeleteInPlace, _, _) => "Delete and reconnect",
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
            MenuItem::Bypass,
            MenuItem::Ungroup,
            MenuItem::Duplicate,
            MenuItem::SaveGroup,
            MenuItem::Delete,
        ];
    }
    match (node.can_duplicate(), node.bypass() != Bypass::None) {
        (true, true) => &[
            MenuItem::Duplicate,
            MenuItem::Bypass,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (true, false) => &[
            MenuItem::Duplicate,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (false, true) => &[MenuItem::Bypass, MenuItem::DeleteInPlace, MenuItem::Delete],
        (false, false) => &[MenuItem::DeleteInPlace, MenuItem::Delete],
    }
}

/// The entries for a node in a group of several selected nodes, which add Auto pack and Delete
/// and reconnect above Delete. They add Group above that when `groupable` says some selected node
/// can go in a group. Delete and reconnect acts on the whole selection, reconnecting around each
/// node that has a through-path and removing the rest plainly.
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
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (true, true, true) => &[
            MenuItem::Duplicate,
            MenuItem::Bypass,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (true, false, false) => &[
            MenuItem::Duplicate,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (true, false, true) => &[
            MenuItem::Duplicate,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (false, true, false) => &[
            MenuItem::Bypass,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (false, true, true) => &[
            MenuItem::Bypass,
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (false, false, false) => &[
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
        (false, false, true) => &[
            MenuItem::Group,
            MenuItem::AutoPack,
            MenuItem::DeleteInPlace,
            MenuItem::Delete,
        ],
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

/// The small square button that opens the burger menu, just right of the logo at the top left.
pub(super) fn burger_bounds() -> Rectangle {
    Rectangle {
        x: LOGO_MARGIN + LOGO_SIZE + LOGO_GAP,
        y: PILL_TOP,
        width: BURGER_SIZE,
        height: BURGER_SIZE,
    }
}

/// The burger menu's top-level list, anchored under the button and left-aligned with it.
pub(super) fn burger_list_bounds() -> Rectangle {
    let burger = burger_bounds();
    Rectangle {
        x: burger.x,
        y: burger.y + burger.height + LIST_GAP,
        width: BURGER_MENU_WIDTH,
        height: BURGER_ROWS.len() as f32 * MENU_ITEM_HEIGHT + BURGER_DIVIDER_HEIGHT,
    }
}

/// The rectangle of one row of the burger menu's top-level list, with room made for the divider
/// above the Presets row.
pub(super) fn burger_row_bounds(row: usize) -> Rectangle {
    let list = burger_list_bounds();
    let divider = if row >= BURGER_PRESETS_ROW {
        BURGER_DIVIDER_HEIGHT
    } else {
        0.0
    };
    Rectangle {
        x: list.x,
        y: list.y + row as f32 * MENU_ITEM_HEIGHT + divider,
        width: list.width,
        height: MENU_ITEM_HEIGHT,
    }
}

/// The row of the burger menu under `point`, if any. The divider itself is not a row.
pub(super) fn burger_row_at(point: Point) -> Option<BurgerRow> {
    let list = burger_list_bounds();
    if !list.contains(point) {
        return None;
    }
    let before = BURGER_PRESETS_ROW as f32 * MENU_ITEM_HEIGHT;
    let y = point.y - list.y;
    let row = if y < before {
        (y / MENU_ITEM_HEIGHT) as usize
    } else {
        let past_divider = y - before;
        if past_divider < BURGER_DIVIDER_HEIGHT {
            return None;
        }
        BURGER_PRESETS_ROW + ((past_divider - BURGER_DIVIDER_HEIGHT) / MENU_ITEM_HEIGHT) as usize
    };
    BURGER_ROWS.get(row).copied()
}

/// Where a list opens beside a row of the burger menu: to the right if there is room, to the left
/// otherwise, leveled with the row and nudged up to clear the key strip. The same positioning a
/// preset's own category submenu already uses beside the preset list.
fn beside_burger_row(bounds: Rectangle, row: usize, rows: usize, width: f32) -> Rectangle {
    let list = burger_list_bounds();
    let height = rows.max(1) as f32 * MENU_ITEM_HEIGHT;
    let right = list.x + list.width + LIST_GAP;
    let x = if right + width <= bounds.width {
        right
    } else {
        list.x - width - LIST_GAP
    };
    let level = burger_row_bounds(row).y;
    let lowest = bounds.height - super::piano::STRIP_HEIGHT - height;
    Rectangle {
        x,
        y: level.min(lowest).max(list.y),
        width,
        height,
    }
}

/// The list that opens beside a category's row in the burger menu.
pub(super) fn category_list_bounds(bounds: Rectangle, category: Category) -> Rectangle {
    let row = BURGER_ROWS
        .iter()
        .position(|item| *item == BurgerRow::Category(category))
        .unwrap_or(0);
    beside_burger_row(
        bounds,
        row,
        category.kinds().len().max(1),
        CATEGORY_MIN_WIDTH,
    )
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

/// What a dropped audio file can become. The drop chooser shows one card for each, side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropOption {
    /// A Wave file node, which cuts the file into frames for a wavetable.
    Wavetable,
    /// A Sampler, which plays the file back.
    Sampler,
}

impl DropOption {
    /// Every option, left to right.
    pub(super) const ALL: [DropOption; 2] = [DropOption::Wavetable, DropOption::Sampler];

    /// The node a file dropped with this option becomes.
    pub(super) fn kind(self) -> NodeKind {
        match self {
            DropOption::Wavetable => NodeKind::WaveFile,
            DropOption::Sampler => NodeKind::Sampler,
        }
    }

    /// The card's name.
    pub(super) fn label(self) -> &'static str {
        match self {
            DropOption::Wavetable => "Wavetable",
            DropOption::Sampler => "Sampler",
        }
    }

    /// The line under the name, saying what the card makes.
    pub(super) fn hint(self) -> &'static str {
        match self {
            DropOption::Wavetable => "Cut into frames",
            DropOption::Sampler => "Play it back",
        }
    }
}

/// Audio files dropped on the editor, waiting for the drop chooser to say what they become.
/// `screen` is where they were dropped, in screen coordinates. Files dropped together all join
/// the one chooser.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DropChoice {
    pub(super) paths: Vec<std::path::PathBuf>,
    pub(super) screen: Point,
}

/// How big one card of the drop chooser is, the gap between the two, and the margin around them.
pub(super) const DROP_CARD_WIDTH: f32 = 150.0;
pub(super) const DROP_CARD_HEIGHT: f32 = 64.0;
pub(super) const DROP_CARD_GAP: f32 = 6.0;
pub(super) const DROP_PADDING: f32 = 6.0;
/// The height of the line over the cards that names the file.
pub(super) const DROP_TITLE_HEIGHT: f32 = 26.0;

/// Where the drop chooser sits: centered on the drop, moved to keep the whole box inside the
/// editor and above the key strip.
pub(super) fn drop_choice_bounds(bounds: Rectangle, at: Point) -> Rectangle {
    let cards = DropOption::ALL.len() as f32;
    let width = 2.0 * DROP_PADDING + cards * DROP_CARD_WIDTH + (cards - 1.0) * DROP_CARD_GAP;
    let height = DROP_TITLE_HEIGHT + DROP_CARD_HEIGHT + DROP_PADDING;
    let lowest = bounds.height - super::piano::STRIP_HEIGHT - height;
    Rectangle {
        x: (at.x - width / 2.0).min(bounds.width - width).max(0.0),
        y: (at.y - height / 2.0).min(lowest).max(0.0),
        width,
        height,
    }
}

/// The card for `option` in the drop chooser at `area`.
pub(super) fn drop_card_bounds(area: Rectangle, option: DropOption) -> Rectangle {
    let index = DropOption::ALL
        .iter()
        .position(|o| *o == option)
        .unwrap_or(0);
    Rectangle {
        x: area.x + DROP_PADDING + index as f32 * (DROP_CARD_WIDTH + DROP_CARD_GAP),
        y: area.y + DROP_TITLE_HEIGHT,
        width: DROP_CARD_WIDTH,
        height: DROP_CARD_HEIGHT,
    }
}

/// The card of the drop chooser at `area` under `point`, if any.
pub(super) fn drop_option_at(area: Rectangle, point: Point) -> Option<DropOption> {
    DropOption::ALL
        .into_iter()
        .find(|option| drop_card_bounds(area, *option).contains(point))
}

#[cfg(test)]
mod drop_choice_tests {
    use super::*;

    const EDITOR: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    };

    #[test]
    fn the_cards_sit_side_by_side_inside_the_chooser() {
        let area = drop_choice_bounds(EDITOR, Point::new(500.0, 300.0));
        let left = drop_card_bounds(area, DropOption::Wavetable);
        let right = drop_card_bounds(area, DropOption::Sampler);
        assert_eq!(left.y, right.y);
        assert!(left.x + left.width < right.x);
        for card in [left, right] {
            assert!(card.x >= area.x && card.x + card.width <= area.x + area.width);
            assert!(card.y >= area.y && card.y + card.height <= area.y + area.height);
        }
    }

    #[test]
    fn a_press_on_a_card_picks_it_and_a_press_between_them_picks_nothing() {
        let area = drop_choice_bounds(EDITOR, Point::new(500.0, 300.0));
        let left = drop_card_bounds(area, DropOption::Wavetable);
        let right = drop_card_bounds(area, DropOption::Sampler);
        let middle = |r: Rectangle| Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(
            drop_option_at(area, middle(left)),
            Some(DropOption::Wavetable)
        );
        assert_eq!(
            drop_option_at(area, middle(right)),
            Some(DropOption::Sampler)
        );
        let gap = Point::new(left.x + left.width + DROP_CARD_GAP / 2.0, middle(left).y);
        assert_eq!(drop_option_at(area, gap), None);
        assert_eq!(
            drop_option_at(area, Point::new(area.x + 1.0, area.y + 1.0)),
            None
        );
    }

    #[test]
    fn a_drop_near_an_edge_keeps_the_chooser_on_screen_and_above_the_keys() {
        let strip = super::super::piano::STRIP_HEIGHT;
        for at in [
            Point::new(0.0, 0.0),
            Point::new(1024.0, 712.0),
            Point::new(1020.0, 10.0),
        ] {
            let area = drop_choice_bounds(EDITOR, at);
            assert!(
                area.x >= 0.0 && area.x + area.width <= EDITOR.width,
                "{at:?}"
            );
            assert!(
                area.y >= 0.0 && area.y + area.height <= EDITOR.height - strip,
                "{at:?}"
            );
        }
    }

    #[test]
    fn the_wavetable_card_makes_a_wave_file_node() {
        assert_eq!(DropOption::Wavetable.kind(), NodeKind::WaveFile);
        assert_eq!(DropOption::Sampler.kind(), NodeKind::Sampler);
    }
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
