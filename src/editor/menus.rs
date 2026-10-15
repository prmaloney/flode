//! The pill of add buttons, the category lists, dropdown lists, and the right-click menu: the
//! chrome that floats above the nodes.

use crate::nodes::{Category, NodeKind};
use nih_plug_iced::{Point, Rectangle};

use super::style::*;

/// The add buttons on the pill, from left to right.
/// What a pill button does: add a node kind, or open a category's list below the pill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PillItem {
    Kind(NodeKind),
    Category(Category),
}

/// The pill's buttons, from left to right.
pub(super) const PILL_ITEMS: [PillItem; 4] = [
    PillItem::Kind(NodeKind::Oscillator),
    PillItem::Category(Category::Driver),
    PillItem::Category(Category::Util),
    PillItem::Category(Category::Effects),
];
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

/// The right-click menu for one node. `position` is its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ContextMenu {
    pub(super) node: usize,
    pub(super) position: Point,
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
        height: MENU_ITEM_HEIGHT,
    }
}
