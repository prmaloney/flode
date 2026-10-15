//! The editor widget. It borrows the graph for a frame, turns mouse and keyboard events into
//! changes to the graph, and draws it.

use crate::editor::knob::{self, Press};
use crate::editor::piano::{self, Piano, PianoEvent};
use crate::nodes::{NodeKind, ROW_KNOB_DIAMETER};
use crate::patch::control::ControlLevels;
use nih_plug_iced::widget::canvas::{
    event::{self, Event},
    Cursor, Geometry, Program,
};
use nih_plug_iced::{keyboard, mouse, Point, Rectangle, Vector};

use super::shortcuts::{self, Shortcut};
use super::{curve::*, draw::*, graph::*, menus::*, view::*};

/// Borrows the graph for one frame of the editor.
pub struct NodeEditor<'a> {
    pub(super) graph: &'a mut Graph,
    /// The live control levels, shown on knobs that a control wire drives.
    pub(super) levels: ControlLevels,
    /// The notes the synth is playing, shown as dots on the key strip.
    pub(super) active_notes: [u64; 2],
    /// The held left and right output levels, shown on the meter.
    pub(super) output: [f32; 2],
}

impl<'a> NodeEditor<'a> {
    pub fn new(
        graph: &'a mut Graph,
        levels: ControlLevels,
        active_notes: [u64; 2],
        output: [f32; 2],
    ) -> Self {
        Self {
            graph,
            levels,
            active_notes,
            output,
        }
    }

    /// `point` is in editor coordinates. `screen` is the mouse position, which knob drags use so
    /// their speed does not change with the zoom.
    pub(super) fn press(
        &mut self,
        point: Point,
        screen: Point,
        bounds: Rectangle,
    ) -> (event::Status, Option<Message>) {
        // A press while placing sets the node down, and nothing else happens on that press.
        if let Interaction::Placing { node, .. } = self.graph.interaction {
            self.graph.interaction = Interaction::Idle;
            if let Some(link) = self.graph.link_target.take() {
                self.graph.insert_into(node, link);
            }
            return (event::Status::Captured, Some(Message::GraphChanged));
        }

        // An open dropdown list takes the press. Its row picks the value. Any other press closes
        // the list and goes through as usual.
        if let Some((index, which)) = self.graph.dropdown_menu.take() {
            let node = &self.graph.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                let rows = dropdown.labels.len();
                let list = dropdown_list_bounds(dropdown.bounds, rows);
                if let Some(row) = list_row_at(list, point, rows) {
                    self.graph.nodes[index].choose(which, row);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
        }

        // An open menu takes the press. Its entry acts on the node. A press anywhere else closes
        // the menu and then goes through as usual.
        if let Some(menu) = self.graph.menu.take() {
            if let Some(item) = menu_item_at(menu, point) {
                match item {
                    MenuItem::Duplicate => {
                        self.duplicate_and_place(menu.node, point);
                    }
                    MenuItem::Bypass => {
                        self.graph.toggle_bypass(menu.node);
                    }
                    MenuItem::Delete => self.graph.delete_node(menu.node),
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // The save box takes every press. One outside the box cancels it.
        if self.graph.name_entry.is_some() {
            if !name_box_bounds(bounds).contains(screen) {
                self.graph.name_entry = None;
            }
            return (event::Status::Captured, Some(Message::GraphChanged));
        }

        // An open preset list takes the press. Its row saves or loads. Any other press closes it.
        let had_presets = self.graph.preset_list.is_some();
        if let Some(names) = self.graph.preset_list.take() {
            match preset_row_at(bounds, names.len(), screen) {
                Some(PresetRow::SaveAs) => {
                    self.graph.start_name_entry();
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(PresetRow::Load(index)) => {
                    self.graph.load_preset(&names[index]);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                None => {}
            }
        }

        // An open category list takes the press. Its row adds the node. Any other press closes it.
        let open_category = self.graph.category_menu.take();
        if let Some(category) = open_category {
            if let Some(kind) = category_row_at(bounds, category, screen) {
                self.place(kind, point);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        match pill_button_at(bounds, screen) {
            Some(PillItem::Kind(kind)) => {
                self.place(kind, point);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            Some(PillItem::Category(category)) => {
                // Pressing the same category again closes its list.
                if open_category != Some(category) {
                    self.graph.category_menu = Some(category);
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            // Pressing the button again closes the list, which the press already did.
            Some(PillItem::Presets) => {
                if !had_presets {
                    self.graph.open_preset_list();
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            None => {}
        }

        match self.graph.terminal_at(point) {
            Some(Terminal::Out(from, out)) => {
                self.graph.interaction = Interaction::Wiring {
                    from,
                    out,
                    cursor: point,
                };
                return (event::Status::Captured, None);
            }
            // Pressing a connected input detaches its wire and drags it from the original output.
            // Releasing on empty space leaves the input disconnected.
            Some(Terminal::In(to, input)) => {
                if let Some(link) = self.graph.disconnect_input(to, input) {
                    self.graph.interaction = Interaction::Wiring {
                        from: link.from,
                        out: link.out,
                        cursor: point,
                    };
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            None => {}
        }

        // Walk nodes from the last drawn (top) to the first.
        for index in self.graph.draw_order().into_iter().rev() {
            let node = &self.graph.nodes[index];
            if let Some(end) = map_handle_at(node, point) {
                self.graph.selected = Some(index);
                self.graph.interaction = Interaction::MapHandle { node: index, end };
                let node = &mut self.graph.nodes[index];
                let current = node.value(MAP_IN[end]);
                let state = &mut node.knob_states[MAP_IN[end] as usize];
                if let Press::Reset = knob::press(state, current, screen) {
                    for id in MAP_IN.into_iter().chain(MAP_OUT) {
                        node.reset_value(id);
                    }
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            for slot in node.knob_slots() {
                // A driven knob is greyed out, so pressing it does nothing to the knob.
                let driven = slot.pinned && self.graph.is_driven(index, slot.id);
                if !driven && knob::contains(slot.center, ROW_KNOB_DIAMETER, point) {
                    self.graph.selected = Some(index);
                    self.graph.interaction = Interaction::Knob {
                        node: index,
                        id: slot.id,
                    };
                    let node = &mut self.graph.nodes[index];
                    let current = node.value(slot.id);
                    let state = &mut node.knob_states[slot.id as usize];
                    if let Press::Reset = knob::press(state, current, screen) {
                        node.reset_value(slot.id);
                    }
                    // A macro knob is a host parameter, so the host hears the whole gesture.
                    self.graph.macro_begin(index, slot.id);
                    self.graph.macro_set(index, slot.id);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if node.toggle_bounds().contains(point) {
                self.graph.selected = Some(index);
                self.graph.dropdown_menu = None;
                self.graph.nodes[index].collapsed = !self.graph.nodes[index].collapsed;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            // A section header folds or opens its section.
            if let Some(section) = node.section_header_at(point) {
                self.graph.selected = Some(index);
                self.graph.dropdown_menu = None;
                self.graph.nodes[index].toggle_section(section);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            {
                let labels = node.switch_labels();
                let hit = (0..labels.len()).find(|&i| node.switch_bounds(i).contains(point));
                if let (Some(hit), false) = (hit, node.collapsed) {
                    self.graph.selected = Some(index);
                    let node = &mut self.graph.nodes[index];
                    let on = !node.switch(hit);
                    node.set_switch(hit, on);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if let Some(which) = node.dropdown_at(point) {
                self.graph.selected = Some(index);
                self.graph.dropdown_menu = Some((index, which));
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if node.bounds().contains(point) {
                self.graph.selected = Some(index);
                if node.header_bounds().contains(point) {
                    let grab = Vector::new(point.x - node.position.x, point.y - node.position.y);
                    self.graph.interaction = Interaction::MovingNode { node: index, grab };
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // Empty canvas clears the selection.
        self.graph.selected = None;
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    /// Handles a key event for the shortcuts, or returns None if no shortcut uses its key. A
    /// shortcut runs once per press. Its key repeats and its release are captured and do
    /// nothing, so the system does not beep at them.
    pub(super) fn shortcut_event(
        &mut self,
        event: &keyboard::Event,
    ) -> Option<(event::Status, Option<Message>)> {
        match event {
            keyboard::Event::KeyPressed {
                key_code,
                modifiers,
            } => {
                let shortcut = shortcuts::lookup(*key_code, *modifiers);
                let Some(shortcut) = shortcut else {
                    // A bound key pressed with the wrong modifiers is not a shortcut. It still
                    // goes on to the piano, which uses D.
                    return None;
                };
                if self.graph.shortcut_held == Some(*key_code) {
                    return Some((event::Status::Captured, None));
                }
                self.graph.shortcut_held = Some(*key_code);
                let changed = self.run_shortcut(shortcut);
                Some((
                    event::Status::Captured,
                    changed.then_some(Message::GraphChanged),
                ))
            }
            keyboard::Event::KeyReleased { key_code, .. }
                if self.graph.shortcut_held == Some(*key_code) =>
            {
                self.graph.shortcut_held = None;
                Some((event::Status::Captured, None))
            }
            _ => None,
        }
    }

    /// Runs a shortcut on the selected node. Returns true if the graph changed. With nothing
    /// selected, or on a node the action does not apply to, nothing happens.
    pub(super) fn run_shortcut(&mut self, shortcut: Shortcut) -> bool {
        let Some(index) = self.graph.selected else {
            return false;
        };
        match shortcut {
            Shortcut::ToggleBypass => self.graph.toggle_bypass(index),
            // The copy appears under the cursor. While a node is already being placed, the
            // shortcut waits, so one press sets down one node.
            Shortcut::Duplicate => {
                if matches!(self.graph.interaction, Interaction::Placing { .. }) {
                    return false;
                }
                let cursor = self.graph.view.to_world(self.graph.last_screen);
                self.duplicate_and_place(index, cursor)
            }
        }
    }

    /// True if an open dropdown list, node menu, or category list covers the cursor. These are
    /// drawn over everything else, so a press on one belongs to it. `point` is in editor
    /// coordinates, and `screen` is the cursor's screen position.
    pub(super) fn overlay_at(&self, point: Point, screen: Point, bounds: Rectangle) -> bool {
        if let Some((index, which)) = self.graph.dropdown_menu {
            let open = self.graph.nodes[index]
                .dropdowns()
                .into_iter()
                .find(|d| d.which == which);
            if let Some(dropdown) = open {
                let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
                if list.contains(point) {
                    return true;
                }
            }
        }
        if let Some(menu) = self.graph.menu {
            if menu_bounds(menu).contains(point) {
                return true;
            }
        }
        if let Some(category) = self.graph.category_menu {
            if category_row_at(bounds, category, screen).is_some() {
                return true;
            }
        }
        if let Some(names) = &self.graph.preset_list {
            if preset_row_at(bounds, names.len(), screen).is_some() {
                return true;
            }
        }
        // The save box takes every press, so a press on the keys does not play one.
        self.graph.name_entry.is_some()
    }

    /// Handles a key event while the save box is open. The box takes every key, so the piano and
    /// the shortcuts stay quiet. A key already held when the box opened still gets its release, so
    /// no note sticks.
    pub(super) fn name_entry_event(
        &mut self,
        event: &keyboard::Event,
    ) -> (event::Status, Option<Message>) {
        match event {
            keyboard::Event::CharacterReceived(c) => {
                let room = self
                    .graph
                    .name_entry
                    .as_ref()
                    .is_some_and(|text| text.chars().count() < crate::presets::MAX_NAME_LEN);
                if room && crate::presets::valid_char(*c) {
                    if let Some(text) = self.graph.name_entry.as_mut() {
                        text.push(*c);
                    }
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyPressed { key_code, .. } => {
                match key_code {
                    keyboard::KeyCode::Backspace => {
                        if let Some(text) = self.graph.name_entry.as_mut() {
                            text.pop();
                        }
                    }
                    keyboard::KeyCode::Escape => self.graph.name_entry = None,
                    keyboard::KeyCode::Enter | keyboard::KeyCode::NumpadEnter => {
                        self.graph.confirm_name_entry();
                        // The editor saves the preset when it hears of this.
                        return (event::Status::Captured, Some(Message::GraphChanged));
                    }
                    _ => {}
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyReleased { .. } => {
                let note = self.graph.piano.handle(event.clone());
                (event::Status::Captured, note.map(Message::Note))
            }
            _ => (event::Status::Ignored, None),
        }
    }

    /// Adds a node of `kind` centered on `point` and makes it follow the cursor until the next
    /// press places it. A kind that is one per graph is not added while the graph has one.
    pub(super) fn place(&mut self, kind: NodeKind, point: Point) {
        if !self.graph.can_add(kind) {
            return;
        }
        let node = self.graph.add_node(kind, point);
        self.graph.start_placing(node, point);
    }

    /// Copies a node and makes the copy follow the cursor, like a node just added from the pill,
    /// until the next press sets it down. Returns false for a node that can't be copied.
    pub(super) fn duplicate_and_place(&mut self, index: usize, point: Point) -> bool {
        match self.graph.duplicate_node(index) {
            Some(copy) => {
                self.graph.start_placing(copy, point);
                true
            }
            None => false,
        }
    }

    /// Right press: selects the node under the cursor and opens its menu. A right press
    /// anywhere else only closes the menus.
    pub(super) fn secondary_press(&mut self, point: Point) -> (event::Status, Option<Message>) {
        self.graph.menu = None;
        self.graph.dropdown_menu = None;
        if let Some(index) = self.graph.node_at(point) {
            self.graph.selected = Some(index);
            self.graph.menu = Some(ContextMenu {
                node: index,
                position: point,
                items: menu_items(&self.graph.nodes[index]),
            });
        }
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    pub(super) fn release(&mut self, point: Point) -> (event::Status, Option<Message>) {
        // Placing ends on the next press, not on the release of the press that started it.
        if let Interaction::Placing { .. } = self.graph.interaction {
            return (event::Status::Captured, None);
        }
        let interaction = std::mem::replace(&mut self.graph.interaction, Interaction::Idle);
        match interaction {
            Interaction::Wiring { from, out, .. } => {
                if let Some(Terminal::In(to, input)) = self.graph.terminal_at(point) {
                    self.graph.connect(from, out, to, input);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Knob { node, id } => {
                knob::release(&mut self.graph.nodes[node].knob_states[id as usize]);
                self.graph.macro_end(node, id);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MapHandle { node, end } => {
                knob::release(&mut self.graph.nodes[node].knob_states[MAP_IN[end] as usize]);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MovingNode { node, .. } => {
                if let Some(link) = self.graph.link_target.take() {
                    self.graph.insert_into(node, link);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Placing { .. } => (event::Status::Captured, None),
            Interaction::Idle => (event::Status::Ignored, None),
        }
    }

    pub(super) fn drag(&mut self, point: Point, screen: Point) -> (event::Status, Option<Message>) {
        match self.graph.interaction {
            Interaction::MovingNode { node, grab } | Interaction::Placing { node, grab } => {
                self.graph.nodes[node].position = Point::new(point.x - grab.x, point.y - grab.y);
                self.graph.link_target = self.graph.link_under(node);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Wiring { from, out, .. } => {
                self.graph.interaction = Interaction::Wiring {
                    from,
                    out,
                    cursor: point,
                };
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MapHandle { node, end } => {
                // The handle follows the cursor over the curve box. Its x is the input, and its y
                // is the output, so both ends move together.
                let bounds = self.graph.nodes[node].curve_bounds();
                let x = ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                let y = (1.0 - (point.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
                let node = &mut self.graph.nodes[node];
                node.set_value(MAP_IN[end], x);
                node.set_value(MAP_OUT[end], y);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Knob { node, id } => {
                let state = &self.graph.nodes[node].knob_states[id as usize];
                if let Some(value) = knob::drag(state, screen) {
                    self.graph.nodes[node].set_value(id, value);
                    self.graph.macro_set(node, id);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Idle => (event::Status::Ignored, None),
        }
    }
}

/// Messages from the node editor. Graph changes republish the audio routing and save the graph.
#[derive(Debug, Clone, Copy)]
pub enum Message {
    /// A note from the computer keyboard. The editor forwards it only in standalone mode.
    Note(PianoEvent),
    GraphChanged,
    /// The cursor moved onto or off a dropdown. Only a redraw is needed.
    Hovered,
}

impl<'a> Program<Message> for NodeEditor<'a> {
    fn update(
        &mut self,
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> (event::Status, Option<Message>) {
        let mouse_event = match event {
            Event::Mouse(mouse_event) => mouse_event,
            Event::Keyboard(key_event) => {
                // The save box takes the keys while it is open.
                if self.graph.name_entry.is_some() {
                    return self.name_entry_event(&key_event);
                }
                // Shortcuts come first, so one on a piano key (Shift+D) is not also a note.
                if let Some(result) = self.shortcut_event(&key_event) {
                    return result;
                }
                let status = if Piano::uses(&key_event) {
                    event::Status::Captured
                } else {
                    event::Status::Ignored
                };
                let note = self.graph.piano.handle(key_event);
                return (status, note.map(Message::Note));
            }
        };
        // Positions are relative to the editor's top-left corner. Drags keep working when the
        // cursor leaves the editor, so only the press checks the bounds. When the cursor is above or
        // beside the window its position is unknown, but a release must still end the drag. So a
        // release uses the last position seen, and a move with no position is ignored.
        let position = cursor.position_from(bounds.position());
        if let Some(position) = position {
            self.graph.last_screen = position;
        }
        let released = matches!(
            mouse_event,
            mouse::Event::ButtonReleased(mouse::Button::Left)
        );
        if position.is_none() && !released {
            return (event::Status::Ignored, None);
        }
        let screen = self.graph.last_screen;
        let point = self.graph.view.to_world(screen);

        // The key strip takes the mouse before the graph does, except under an open list or menu,
        // which is drawn over the strip and has to get the press. A key plays while the mouse
        // holds it.
        let strip = piano::strip_bounds(bounds);
        match mouse_event {
            mouse::Event::ButtonPressed(mouse::Button::Left)
                if strip.contains(screen) && !self.overlay_at(point, screen, bounds) =>
            {
                let note = piano::key_at(strip, screen);
                self.graph.mouse_note = note;
                return match note {
                    Some(note) => (
                        event::Status::Captured,
                        Some(Message::Note(PianoEvent::NoteOn {
                            note,
                            velocity: piano::CLICK_VELOCITY,
                        })),
                    ),
                    None => (event::Status::Captured, None),
                };
            }
            mouse::Event::ButtonReleased(mouse::Button::Left)
                if self.graph.mouse_note.is_some() =>
            {
                let note = self.graph.mouse_note.take();
                return (
                    event::Status::Captured,
                    note.map(|note| Message::Note(PianoEvent::NoteOff { note })),
                );
            }
            _ => {}
        }

        match mouse_event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                self.press(point, screen, bounds)
            }
            mouse::Event::ButtonPressed(mouse::Button::Right) => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                self.secondary_press(point)
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => self.release(point),
            mouse::Event::CursorMoved { .. } => {
                let hover = self.graph.hover_at(point);
                let changed = hover != self.graph.hover;
                self.graph.hover = hover;
                match self.drag(point, screen) {
                    (event::Status::Ignored, None) if changed => {
                        (event::Status::Ignored, Some(Message::Hovered))
                    }
                    other => other,
                }
            }
            mouse::Event::WheelScrolled { delta } => {
                if !cursor.is_over(&bounds) {
                    return (event::Status::Ignored, None);
                }
                match view_scroll(delta) {
                    ViewScroll::Zoom(magnification) => {
                        self.graph.view.zoom_at(screen, 1.0 + magnification)
                    }
                    ViewScroll::Pan(by) => self.graph.view.pan(by),
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            _ => (event::Status::Ignored, None),
        }
    }

    fn draw(&self, bounds: Rectangle, _cursor: Cursor) -> Vec<Geometry> {
        draw_editor(
            &*self.graph,
            self.levels,
            self.active_notes,
            self.output,
            bounds,
        )
    }
}
