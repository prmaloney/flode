//! The group interface: where the floating pane sits, the box that renames things, and the
//! keyboard and mouse handling that opens, closes, makes, and edits groups. The commands
//! themselves are in `group`. Drawing is in `pane`.

use super::canvas::{Message, NodeEditor};
use super::graph::{EditorMode, Graph, Interaction, Terminal};
use super::group::PinSide;
use super::menus::{
    burger_bounds, ContextMenu, MenuItem, MenuTarget, MACRO_MENU_ITEMS, PIN_MENU_ITEMS,
};
use super::piano;
use super::shortcuts::Shortcut;
use super::style::{PILL_HEIGHT, PILL_TOP};
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::{FlodeNode, NodeKind, Port, ROW_PITCH};
use nih_plug_iced::widget::canvas::event;
use nih_plug_iced::{keyboard, Point, Rectangle, Size};

/// The editor's size before any event has reported it. It matches the window the plugin opens.
pub(super) const DEFAULT_CANVAS: (f32, f32) = (1024.0, 712.0);

/// How much of the editor's width the dimmed margin takes on each side of the pane, as a
/// fraction. The margin never goes under `MIN_MARGIN` pixels.
const PANE_MARGIN: f32 = 0.045;
const MIN_MARGIN: f32 = 16.0;
/// The room kept above the pane for the burger button and the status line under it.
const TOP_ROOM: f32 = 28.0;

/// The height of the pane's title strip, and the side of its close button.
pub(super) const TITLE_HEIGHT: f32 = 34.0;
pub(super) const CLOSE_SIZE: f32 = 22.0;
const TITLE_INSET: f32 = 14.0;
/// The width of one character of the title, which the layout estimates from the font size.
pub(super) const TITLE_CHAR_WIDTH: f32 = 7.0;
/// The most characters of the title the strip shows.
const TITLE_CHARS: usize = 64;

/// How far the cursor may move between a press and its release on the margin for the pair to
/// count as a click.
pub(super) const CLICK_SLOP: f32 = 6.0;

/// The most characters a name takes. A macro name has to fit beside its knob.
pub(super) const RENAME_MAX_LEN: usize = 24;
pub(super) const MACRO_NAME_MAX_LEN: usize = 12;
/// The size of the rename box.
pub(super) const RENAME_SIZE: (f32, f32) = (210.0, 30.0);

/// The floating pane that shows an open group's graph: almost all of the editor, with a margin
/// round it. The burger button stays above it and the key strip below, so both keep working.
pub(super) fn pane_bounds(bounds: Rectangle) -> Rectangle {
    let margin = (bounds.width * PANE_MARGIN).max(MIN_MARGIN);
    let top = PILL_TOP + PILL_HEIGHT + TOP_ROOM;
    let gap = (bounds.height * PANE_MARGIN * 0.5).max(10.0);
    let bottom = piano::strip_bounds(bounds).y - gap;
    Rectangle {
        x: margin,
        y: top,
        width: (bounds.width - 2.0 * margin).max(0.0),
        height: (bottom - top).max(0.0),
    }
}

/// The title strip along the top of the pane.
pub(super) fn title_bounds(pane: Rectangle) -> Rectangle {
    Rectangle {
        height: TITLE_HEIGHT.min(pane.height),
        ..pane
    }
}

/// The part of the pane under the title, where the graph is drawn.
pub(super) fn inner_bounds(pane: Rectangle) -> Rectangle {
    let title = title_bounds(pane);
    Rectangle {
        x: pane.x,
        y: pane.y + title.height,
        width: pane.width,
        height: (pane.height - title.height).max(0.0),
    }
}

/// The close button at the right end of the title strip.
pub(super) fn close_bounds(pane: Rectangle) -> Rectangle {
    Rectangle {
        x: pane.x + pane.width - CLOSE_SIZE - 10.0,
        y: pane.y + (TITLE_HEIGHT - CLOSE_SIZE) / 2.0,
        width: CLOSE_SIZE,
        height: CLOSE_SIZE,
    }
}

/// The save button, just left of the close button. It saves the open group to a file.
pub(super) fn save_bounds(pane: Rectangle) -> Rectangle {
    Rectangle {
        x: close_bounds(pane).x - CLOSE_SIZE - 6.0,
        ..close_bounds(pane)
    }
}

/// How the title strip lays out the breadcrumb: the path to the open group in dim text, and the
/// open group's own name after it. The width of the text is estimated from its length, so the
/// two share one place and the drawing and the hit test agree.
pub(super) struct TitleLayout {
    /// The dim part, "Main > Outer > ", or empty if the path is cut to nothing.
    pub prefix: String,
    /// The open group's name.
    pub name: String,
    /// Where the name sits, which is the part a click renames.
    pub name_area: Rectangle,
    pub name_x: f32,
}

/// The breadcrumb for the groups open now, outermost first, under a root called "Main".
pub(super) fn title_layout(pane: Rectangle, path: &[String]) -> TitleLayout {
    let last = path.last().cloned().unwrap_or_default();
    let name = cut(&last, TITLE_CHARS / 2);
    let mut parents: Vec<&str> = vec!["Main"];
    parents.extend(path.iter().rev().skip(1).rev().map(String::as_str));
    // A path too long for the strip loses its outer names, which an ellipsis stands in for.
    let room = TITLE_CHARS.saturating_sub(name.chars().count() + 3);
    let mut prefix = format!("{} > ", parents.join(" > "));
    if prefix.chars().count() > room {
        let kept: Vec<&str> = parents
            .iter()
            .rev()
            .scan(0usize, |used, name| {
                *used += name.chars().count() + 3;
                (*used + 4 <= room).then_some(*name)
            })
            .collect();
        let kept: Vec<&str> = kept.into_iter().rev().collect();
        prefix = if kept.is_empty() {
            "... > ".to_string()
        } else {
            format!("... > {} > ", kept.join(" > "))
        };
    }
    let name_x = pane.x + TITLE_INSET + prefix.chars().count() as f32 * TITLE_CHAR_WIDTH;
    let save = save_bounds(pane);
    let width = (name.chars().count() as f32 * TITLE_CHAR_WIDTH + 12.0)
        .min((save.x - 8.0 - name_x).max(0.0));
    let name_area = Rectangle {
        x: name_x - 6.0,
        y: pane.y + 4.0,
        width,
        height: TITLE_HEIGHT - 8.0,
    };
    TitleLayout {
        prefix,
        name,
        name_area,
        name_x,
    }
}

/// `text` cut to `max` characters, with an ellipsis on the end if it was longer.
pub(super) fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// What a rename box is renaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RenameTarget {
    /// A pin of a group part on show: the group node, or an edge node of the open group.
    Pin {
        node: usize,
        side: PinSide,
        index: usize,
    },
    /// A group node on show.
    Group { node: usize },
    /// The group that is open.
    OpenGroup,
    /// A knob of the macro node, by the number of the macro.
    Macro { node: usize, index: usize },
}

/// The box that renames something. It starts with the current name selected, so the first key
/// typed replaces it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rename {
    pub(super) target: RenameTarget,
    pub(super) text: String,
    /// True until the text is edited. A box left alone and confirmed changes nothing.
    pub(super) fresh: bool,
    /// The box's top left corner, in screen coordinates. The box keeps inside the editor.
    pub(super) screen: Point,
}

impl Rename {
    /// The most characters the name takes.
    pub(super) fn max_len(&self) -> usize {
        match self.target {
            RenameTarget::Macro { .. } => MACRO_NAME_MAX_LEN,
            _ => RENAME_MAX_LEN,
        }
    }
}

/// Where the rename box sits: its corner at `at`, moved to keep the whole box in the editor.
pub(super) fn rename_bounds(bounds: Rectangle, at: Point) -> Rectangle {
    let (width, height) = RENAME_SIZE;
    Rectangle {
        x: at.x.min(bounds.width - width).max(0.0),
        y: at.y.min(bounds.height - height).max(0.0),
        width,
        height,
    }
}

/// The part of the editor a press on the margin of an open pane may land in: not the burger
/// button, the logo, the meter, or the key strip, which all stay in use.
fn on_margin(bounds: Rectangle, screen: Point) -> bool {
    let pane = pane_bounds(bounds);
    if pane.contains(screen) || burger_bounds().contains(screen) {
        return false;
    }
    if piano::strip_bounds(bounds).contains(screen) {
        return false;
    }
    if super::logo::logo_bounds(bounds).is_some_and(|logo| logo.contains(screen)) {
        return false;
    }
    !super::meter::meter_bounds(bounds).contains(screen)
}

/// The group part, side, and pin number a terminal of the shown graph is, or None for any other
/// terminal.
fn pin_of(graph: &Graph, terminal: Terminal) -> Option<(usize, PinSide, usize)> {
    let (node, port, takes) = match terminal {
        Terminal::In(node, port) => (node, port, true),
        Terminal::Out(node, port) => (node, port, false),
    };
    let Port::Pin(_, index) = port else {
        return None;
    };
    let side = match (graph.nodes.get(node)?.kind, takes) {
        (NodeKind::Group, true) | (NodeKind::GroupInput, false) => PinSide::Input,
        (NodeKind::Group, false) | (NodeKind::GroupOutput, true) => PinSide::Output,
        _ => return None,
    };
    Some((node, side, index))
}

/// The macro a point on a macro node's body falls on, by row.
fn macro_row_at(node: &FlodeNode, point: Point) -> Option<usize> {
    if node.kind != NodeKind::Macro || !node.bounds().contains(point) {
        return None;
    }
    node.row_places()
        .iter()
        .position(|place| (point.y - (node.position.y + place.y)).abs() <= ROW_PITCH / 2.0)
        .filter(|&index| index < MACRO_COUNT)
}

/// The glyph in a group node's header that says it opens: a small box with an arrow in it. A
/// press on it opens the group.
pub(super) fn open_glyph_bounds(node: &FlodeNode) -> Option<Rectangle> {
    if node.kind != NodeKind::Group {
        return None;
    }
    let header = node.header_bounds();
    let size = 16.0;
    Some(Rectangle {
        x: header.x + header.width - size - 10.0,
        y: header.y + (header.height - size) / 2.0,
        width: size,
        height: size,
    })
}

impl Graph {
    /// Opens the rename box on `target`, with the box's corner at `screen`. Returns false, and
    /// opens nothing, if the target is not there.
    pub(super) fn open_rename(&mut self, target: RenameTarget, screen: Point) -> bool {
        let text = match &target {
            RenameTarget::Pin { node, side, index } => self
                .pins_on(*node, *side)
                .and_then(|pins| pins.get(*index))
                .map(|pin| pin.name.clone()),
            RenameTarget::Group { node } => self
                .nodes
                .get(*node)
                .filter(|n| n.kind == NodeKind::Group)
                .map(|n| n.label().into_owned()),
            RenameTarget::OpenGroup => self.group_path_names().last().cloned(),
            RenameTarget::Macro { node, index } => self
                .nodes
                .get(*node)
                .filter(|n| n.kind == NodeKind::Macro)
                .map(|n| n.macro_name(*index)),
        };
        let Some(text) = text else {
            return false;
        };
        self.menu = None;
        self.dropdown_menu = None;
        self.category_menu = None;
        self.preset_list = None;
        self.group_list = None;
        self.knob_entry = None;
        self.quick_add = None;
        self.rename = Some(Rename {
            target,
            text,
            fresh: true,
            screen,
        });
        true
    }

    /// Closes the rename box and gives its target the name typed in it. A group or a pin keeps
    /// its name if the box is empty. A macro goes back to its own name. Returns true if
    /// something changed.
    pub(super) fn commit_rename(&mut self) -> bool {
        let Some(rename) = self.rename.take() else {
            return false;
        };
        if rename.fresh {
            // Nothing was typed, so the name stays as it was.
            return false;
        }
        let name = rename.text.trim();
        match rename.target {
            RenameTarget::Pin { node, side, index } => self.rename_pin_of(node, side, index, name),
            RenameTarget::Group { node } => self.rename_group_node(node, name),
            RenameTarget::OpenGroup => self.rename_group(name),
            RenameTarget::Macro { node, index } => self
                .nodes
                .get_mut(node)
                .is_some_and(|n| n.set_macro_name(index, name)),
        }
    }
}

impl<'a> NodeEditor<'a> {
    /// Runs a shortcut that works on groups. None for any other shortcut. Returns true if the
    /// graph or what is on show changed. None of them acts while a drag or a placement runs.
    pub(super) fn run_group_shortcut(&mut self, shortcut: Shortcut) -> Option<bool> {
        let idle = matches!(self.graph.interaction, Interaction::Idle);
        let changed = match shortcut {
            Shortcut::Group => {
                idle && {
                    let members = self.graph.selection();
                    self.graph.create_group(&members).is_some()
                }
            }
            Shortcut::Ungroup => {
                idle && self
                    .graph
                    .selected
                    .is_some_and(|index| self.graph.ungroup(index))
            }
            Shortcut::Escape => {
                let menus = self.graph.menu.is_some()
                    || self.graph.drop_choice.is_some()
                    || self.graph.dropdown_menu.is_some()
                    || self.graph.category_menu.is_some()
                    || self.graph.preset_list.is_some()
                    || self.graph.group_list.is_some();
                if menus {
                    self.graph.menu = None;
                    self.graph.drop_choice = None;
                    self.graph.dropdown_menu = None;
                    self.graph.category_menu = None;
                    self.graph.preset_list = None;
                    self.graph.group_list = None;
                    true
                } else {
                    idle && self.graph.close_group()
                }
            }
            _ => return None,
        };
        Some(changed)
    }

    /// Handles a key event while the rename box is open. The box takes every key, so the piano
    /// and the shortcuts stay quiet, and a Tab typed in it is not a shortcut. Enter renames,
    /// and Escape closes the box with no change. A key already held when the box opened still
    /// gets its release, so no note sticks.
    pub(super) fn rename_event(
        &mut self,
        event: &keyboard::Event,
    ) -> (event::Status, Option<Message>) {
        match event {
            keyboard::Event::CharacterReceived(c) => {
                if let Some(rename) = self.graph.rename.as_mut() {
                    // Tab and the other control characters are not text.
                    if !c.is_control() {
                        if rename.fresh {
                            rename.text.clear();
                            rename.fresh = false;
                        }
                        if rename.text.chars().count() < rename.max_len() {
                            rename.text.push(*c);
                        }
                    }
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyPressed { key_code, .. } => match key_code {
                keyboard::KeyCode::Backspace => {
                    if let Some(rename) = self.graph.rename.as_mut() {
                        if rename.fresh {
                            rename.text.clear();
                            rename.fresh = false;
                        } else {
                            rename.text.pop();
                        }
                    }
                    (event::Status::Captured, Some(Message::Hovered))
                }
                keyboard::KeyCode::Escape => {
                    self.graph.rename = None;
                    (event::Status::Captured, Some(Message::Hovered))
                }
                keyboard::KeyCode::Enter | keyboard::KeyCode::NumpadEnter => {
                    let changed = self.graph.commit_rename();
                    let message = if changed {
                        Message::GraphChanged
                    } else {
                        Message::Hovered
                    };
                    (event::Status::Captured, Some(message))
                }
                _ => (event::Status::Captured, Some(Message::Hovered)),
            },
            keyboard::Event::KeyReleased { .. } => {
                let note = self.graph.piano.handle(event.clone());
                let note = note.filter(|_| self.graph.mode == EditorMode::Keyboard);
                (event::Status::Captured, note.map(Message::Note))
            }
            _ => (event::Status::Ignored, None),
        }
    }

    /// A press while the rename box is open. A press in the box does nothing. A press anywhere
    /// else closes the box, keeping the name typed in it, and goes no further.
    pub(super) fn press_rename(
        &mut self,
        screen: Point,
        bounds: Rectangle,
    ) -> (event::Status, Option<Message>) {
        let inside = self
            .graph
            .rename
            .as_ref()
            .is_some_and(|rename| rename_bounds(bounds, rename.screen).contains(screen));
        if inside {
            return (event::Status::Captured, Some(Message::Hovered));
        }
        let changed = self.graph.commit_rename();
        let message = if changed {
            Message::GraphChanged
        } else {
            Message::Hovered
        };
        (event::Status::Captured, Some(message))
    }

    /// A left press while a group is open, on the pane's own parts. The close button closes the
    /// group, and the group's name in the title opens the rename box. A press on the margin
    /// waits for its release, which closes the group if it was a click. Returns None for a press
    /// on the pane's graph, which goes on to the nodes.
    pub(super) fn press_pane(
        &mut self,
        screen: Point,
        bounds: Rectangle,
    ) -> Option<(event::Status, Option<Message>)> {
        if self.graph.group_depth() == 0 {
            return None;
        }
        let pane = pane_bounds(bounds);
        if !pane.contains(screen) {
            if on_margin(bounds, screen) {
                self.graph.margin_press = Some(screen);
                return Some((event::Status::Captured, None));
            }
            return None;
        }
        if close_bounds(pane).contains(screen) {
            self.graph.close_group();
            return Some((event::Status::Captured, Some(Message::GraphChanged)));
        }
        if save_bounds(pane).contains(screen) {
            self.graph.start_open_group_save();
            return Some((event::Status::Captured, Some(Message::Hovered)));
        }
        if title_bounds(pane).contains(screen) {
            let layout = title_layout(pane, &self.graph.group_path_names());
            if layout.name_area.contains(screen) {
                let at = Point::new(layout.name_area.x - 4.0, pane.y + 2.0);
                self.graph.open_rename(RenameTarget::OpenGroup, at);
            }
            return Some((event::Status::Captured, Some(Message::Hovered)));
        }
        None
    }

    /// The release that ends a press on the margin. A click, where the cursor stayed near where
    /// it went down and is still off the pane, closes the group. A drag does nothing. None if no
    /// press on the margin is waiting.
    pub(super) fn release_margin(
        &mut self,
        bounds: Rectangle,
    ) -> Option<(event::Status, Option<Message>)> {
        let start = self.graph.margin_press.take()?;
        let end = self.graph.last_screen;
        let moved = (end.x - start.x).hypot(end.y - start.y);
        let click = moved <= CLICK_SLOP && !pane_bounds(bounds).contains(end);
        if click && self.graph.close_group() {
            return Some((event::Status::Captured, Some(Message::GraphChanged)));
        }
        Some((event::Status::Captured, None))
    }

    /// A press on a group node's header, which a double click or the open glyph turns into
    /// opening the group. Returns true if it opened.
    pub(super) fn press_group_header(&mut self, index: usize, point: Point, screen: Point) -> bool {
        let Some(node) = self.graph.nodes.get(index) else {
            return false;
        };
        if node.kind != NodeKind::Group {
            return false;
        }
        let on_glyph = open_glyph_bounds(node).is_some_and(|glyph| glyph.contains(point));
        let double = super::knob::double_click(&mut self.graph.header_click, screen);
        (on_glyph || double) && self.graph.open_group(index)
    }

    /// A right press on a pin of a group, or on a row of the macro node, opens a menu for it.
    /// Returns true if it did.
    pub(super) fn secondary_group_target(&mut self, point: Point, screen: Point) -> bool {
        if let Some(terminal) = self.graph.terminal_at(point) {
            if let Some((node, side, index)) = pin_of(self.graph, terminal) {
                self.graph.menu = Some(ContextMenu {
                    node,
                    position: screen,
                    items: PIN_MENU_ITEMS,
                    target: MenuTarget::Pin(side, index),
                });
                return true;
            }
        }
        let Some(node) = self.graph.node_at(point) else {
            return false;
        };
        if let Some(index) = macro_row_at(&self.graph.nodes[node], point) {
            if !self.graph.is_selected(node) {
                self.graph.select(node);
            }
            self.graph.menu = Some(ContextMenu {
                node,
                position: screen,
                items: MACRO_MENU_ITEMS,
                target: MenuTarget::Macro(index),
            });
            return true;
        }
        false
    }

    /// Runs an entry of a menu that the group commands own. Returns true if the entry was one of
    /// them, and so is done with.
    pub(super) fn group_menu_item(&mut self, menu: ContextMenu, item: MenuItem) -> bool {
        match item {
            MenuItem::Open => {
                self.graph.open_group(menu.node);
            }
            MenuItem::Ungroup => {
                self.graph.ungroup(menu.node);
            }
            MenuItem::SaveGroup => self.graph.start_group_save_dialog(menu.node),
            MenuItem::Group => {
                let members = self.graph.selection();
                self.graph.create_group(&members);
            }
            MenuItem::RemovePin => {
                if let MenuTarget::Pin(side, index) = menu.target {
                    self.graph.remove_pin_of(menu.node, side, index);
                }
            }
            MenuItem::Rename => {
                let target = match menu.target {
                    MenuTarget::Pin(side, index) => RenameTarget::Pin {
                        node: menu.node,
                        side,
                        index,
                    },
                    MenuTarget::Macro(index) => RenameTarget::Macro {
                        node: menu.node,
                        index,
                    },
                    MenuTarget::Node => RenameTarget::Group { node: menu.node },
                };
                self.graph.open_rename(target, menu.position);
            }
            _ => return false,
        }
        true
    }
}

/// The size of the editor in the form the pane math takes.
pub(super) fn canvas_rect(size: Size) -> Rectangle {
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    }
}
