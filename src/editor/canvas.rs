//! The editor widget. It borrows the graph for a frame, turns mouse and keyboard events into
//! changes to the graph, and draws it.

use crate::dsp::curve::MAX_CURVE_POINTS;
use crate::editor::knob::{self, Press};
use crate::editor::piano::{self, Piano, PianoEvent};
use crate::groups::GroupEntry;
use crate::nodes::{envelope, eq};
use crate::nodes::{NodeKind, Port, ROW_KNOB_DIAMETER};
use crate::patch::control::ControlLevels;
use nih_plug_iced::widget::canvas::{
    event::{self, Event},
    Cursor, Geometry, Program,
};
use nih_plug_iced::{keyboard, mouse, Point, Rectangle, Vector};

use super::group_ui::{canvas_rect, pane_bounds, rename_bounds};
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

        // A left click while grabbing commits the grab at the current position, the same way a
        // press sets down a placed node, and nothing else happens on that press.
        if matches!(self.graph.interaction, Interaction::Grabbing { .. }) {
            self.graph.commit_grab();
            return (event::Status::Captured, Some(Message::GraphChanged));
        }

        // The rename box takes every press. One outside it keeps the name and closes the box.
        if self.graph.rename.is_some() {
            return self.press_rename(screen, bounds);
        }

        // The quick add box takes every press. A result adds its node, which then follows the
        // cursor. A press in the box off the results does nothing, and one outside closes it.
        if self.graph.quick_add.is_some() {
            let results = self.graph.quick_add_picks();
            if let Some(open) = self.graph.quick_add.take() {
                let area = quick_add_bounds(bounds, open.screen, results.len());
                if let Some(row) = quick_add_row_at(area, screen, results.len()) {
                    self.place_pick(results[row], open.world, open.feed, &open.groups);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                if area.contains(screen) {
                    self.graph.quick_add = Some(open);
                }
            }
            return (event::Status::Captured, Some(Message::Hovered));
        }

        // An open dropdown list takes the press. Its row picks the value. Any other press closes
        // the list and goes through as usual.
        if let Some((index, which)) = self.graph.dropdown_menu.take() {
            let node = &self.graph.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                let rows = dropdown.labels.len();
                let list = dropdown_list_bounds(dropdown.bounds, rows);
                if let Some(row) = list_row_at(list, point, rows) {
                    // A row that can't be used closes the list and changes nothing.
                    if self.graph.dropdown_row_disabled(index, which, row) {
                        return (event::Status::Captured, None);
                    }
                    self.graph.nodes[index].choose(which, row);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
        }

        // An open menu takes the press. Its entry acts on the node. A press anywhere else closes
        // the menu and then goes through as usual.
        if let Some(menu) = self.graph.menu.take() {
            if let Some(item) = menu_item_at(menu, screen) {
                match item {
                    MenuItem::Duplicate => {
                        self.duplicate_and_place(menu.node, point);
                    }
                    // On a node of a group, bypass and delete act on the whole group.
                    MenuItem::Bypass => {
                        for index in self.graph.menu_targets(menu.node) {
                            self.graph.toggle_bypass(index);
                        }
                    }
                    MenuItem::AutoPack => self.graph.auto_pack(),
                    MenuItem::Open
                    | MenuItem::Rename
                    | MenuItem::Ungroup
                    | MenuItem::Group
                    | MenuItem::SaveGroup
                    | MenuItem::RemovePin => {
                        self.group_menu_item(menu, item);
                    }
                    MenuItem::Delete => {
                        if self.graph.is_selected(menu.node) {
                            self.graph.delete_selected();
                        } else {
                            self.graph.delete_node(menu.node);
                        }
                    }
                    MenuItem::DeleteInPlace => {
                        if self.graph.is_selected(menu.node) {
                            self.graph.delete_selected_in_place();
                        } else {
                            self.graph.delete_in_place(menu.node);
                        }
                    }
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // A press anywhere closes the number box. It keeps what was typed, if anything was, and
        // then the press goes on as usual.
        if self
            .graph
            .knob_entry
            .as_ref()
            .is_some_and(|entry| !entry.fresh)
        {
            self.graph.confirm_knob_entry();
        }
        self.graph.knob_entry = None;

        // An open preset list takes the press. Its row saves, loads, or opens a category's
        // submenu. Any other press closes it, and closes the burger menu with it.
        let had_presets = self.graph.preset_list.is_some();
        if let Some(mut menu) = self.graph.preset_list.take() {
            let library = &menu.library;
            match preset_row_at(bounds, &menu, screen) {
                Some(PresetRow::SaveAs) => {
                    self.graph.start_save_dialog();
                    self.graph.burger_menu = false;
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                // Init drops what is on the canvas for the starting graph, saved or not.
                Some(PresetRow::Init) => {
                    self.graph.init_default();
                    self.graph.burger_menu = false;
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                // A press on a category opens its submenu, or closes it if it is open.
                Some(PresetRow::Category(index)) => {
                    menu.open = (menu.open != Some(index)).then_some(index);
                    self.graph.preset_list = Some(menu);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(PresetRow::Loose(index)) => {
                    self.graph.load_preset("", &library.loose[index]);
                    self.graph.burger_menu = false;
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(PresetRow::Sub(category, index)) => {
                    let category = &library.categories[category];
                    self.graph
                        .load_preset(&category.name, &category.presets[index]);
                    self.graph.burger_menu = false;
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                None => {}
            }
        }

        // An open list of saved groups takes the press. Its row adds a copy of the group, which
        // then follows the cursor. Any other press closes it, and closes the burger menu with it.
        let had_groups = self.graph.group_list.is_some();
        if let Some(entries) = self.graph.group_list.take() {
            if let Some(row) = group_row_at(bounds, entries.len(), screen) {
                self.place_group(&entries[row], point, None, None);
                self.graph.burger_menu = false;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // An open category list takes the press. Its row adds the node, and closes the burger
        // menu with it. Any other press closes just the category list.
        let open_category = self.graph.category_menu.take();
        if let Some(category) = open_category {
            if let Some(kind) = category_row_at(bounds, category, screen) {
                self.place(kind, point);
                self.graph.burger_menu = false;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // The burger menu's own top-level list takes the press next. Add Oscillator places a node
        // directly and closes the whole menu. A category, Presets, or Groups row opens its list
        // underneath, nested beside the burger menu the same way a preset's own category submenu
        // nests beside the preset list, and the burger menu stays open so its row still shows
        // which list is open. Any other press, including one on the burger button itself while the
        // menu is open, closes it.
        let was_burger_open = self.graph.burger_menu;
        self.graph.burger_menu = false;
        if was_burger_open {
            match burger_row_at(screen) {
                Some(BurgerRow::AddOscillator) => {
                    self.place(NodeKind::Oscillator, point);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(BurgerRow::AddSampler) => {
                    self.place(NodeKind::Sampler, point);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(BurgerRow::Category(category)) => {
                    self.graph.burger_menu = true;
                    if open_category != Some(category) {
                        self.graph.category_menu = Some(category);
                    }
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(BurgerRow::Presets) => {
                    self.graph.burger_menu = true;
                    if !had_presets {
                        self.graph.open_preset_list();
                    }
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                Some(BurgerRow::Groups) => {
                    self.graph.burger_menu = true;
                    if !had_groups {
                        self.graph.open_group_list();
                    }
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                None => {}
            }
        }
        if burger_bounds().contains(screen) {
            // Pressing the button again closes the menu, which the press already did above.
            if !was_burger_open {
                self.graph.burger_menu = true;
            }
            return (event::Status::Captured, Some(Message::GraphChanged));
        }

        // While a group is open the pane has its own parts, and a press beside it is on the
        // margin.
        if let Some(result) = self.press_pane(screen, bounds) {
            return result;
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
                // An input with no wire starts one from the input end. Dropping it on an output
                // joins them, and dropping it on empty canvas offers the nodes that can feed it.
                self.graph.interaction = Interaction::BackWiring {
                    to,
                    input,
                    cursor: point,
                };
                return (event::Status::Captured, None);
            }
            None => {}
        }

        // Walk nodes from the last drawn (top) to the first.
        for index in self.graph.draw_order().into_iter().rev() {
            if !self.graph.shift {
                if let Some(result) = self.press_lfo_viewer(index, point, screen) {
                    return result;
                }
            }
            let node = &self.graph.nodes[index];
            // Shift+click adds the node to the selection, or drops it from it.
            if self.graph.shift && node.bounds().contains(point) {
                self.graph.toggle_selected(index);
                self.graph.dropdown_menu = None;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            let live = live_knobs(self.graph, index, &self.levels);
            if let Some(end) = map_handle_at(&self.graph.nodes[index], &live, point) {
                self.graph.select(index);
                self.graph.interaction = Interaction::MapHandle { node: index, end };
                let node = &mut self.graph.nodes[index];
                let current = node.value(MAP_IN[end]);
                let state = &mut node.knob_states[MAP_IN[end] as usize];
                if let Press::Reset = knob::press(state, current, screen) {
                    for id in MAP_IN.into_iter().chain(MAP_OUT).chain(MAP_CENTER_RANGE) {
                        node.reset_value(id);
                    }
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if let Some(band) = eq_handle_at(&self.graph.nodes[index], &live, point) {
                self.graph.select(index);
                self.graph.interaction = Interaction::EqPoint { node: index, band };
                let node = &mut self.graph.nodes[index];
                let current = node.value(eq::FREQ[band]);
                let state = &mut node.knob_states[eq::FREQ[band] as usize];
                if let Press::Reset = knob::press(state, current, screen) {
                    // A double click puts the band back where a new node has it.
                    for id in [eq::FREQ[band], eq::GAIN[band], eq::Q[band]] {
                        node.reset_value(id);
                    }
                    self.graph.interaction = Interaction::Idle;
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if let Some(handle) = envelope_handle_at(&self.graph.nodes[index], &live, point) {
                self.graph.select(index);
                self.graph.interaction = Interaction::EnvelopeHandle {
                    node: index,
                    handle,
                };
                let node = &mut self.graph.nodes[index];
                let knobs = envelope::HANDLE_KNOBS[handle];
                let current = node.value(knobs[0]);
                let state = &mut node.knob_states[knobs[0] as usize];
                if let Press::Reset = knob::press(state, current, screen) {
                    // A double click puts the handle's knobs back where a new node has them.
                    for id in knobs {
                        node.reset_value(*id);
                    }
                    self.graph.interaction = Interaction::Idle;
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if let Some(band) = node.eq_toggle_at(point) {
                self.graph.select(index);
                let node = &mut self.graph.nodes[index];
                let on = !node.switch(eq::band_switch(band));
                node.set_switch(eq::band_switch(band), on);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            for slot in node.knob_slots() {
                // A driven knob is greyed out, so pressing it does nothing to the knob.
                let driven = slot.pinned && self.graph.is_driven(index, slot.id);
                if !driven && knob::contains(slot.center, ROW_KNOB_DIAMETER, point) {
                    self.graph.select(index);
                    self.graph.interaction = Interaction::Knob {
                        node: index,
                        id: slot.id,
                    };
                    // Alt+click puts the knob back to its default, and the drag starts from there.
                    let reset = self.graph.alt;
                    let node = &mut self.graph.nodes[index];
                    if reset {
                        node.reset_value(slot.id);
                    }
                    let current = node.value(slot.id);
                    let state = &mut node.knob_states[slot.id as usize];
                    // A double click opens a box to type the value.
                    let typed = matches!(knob::press(state, current, screen), Press::Reset);
                    if typed {
                        self.graph.open_knob_entry(index, slot.id);
                    }
                    // A macro knob is a host parameter, so the host hears the whole gesture.
                    self.graph.macro_begin(index, slot.id);
                    self.graph.macro_set(index, slot.id);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if node.toggle_bounds().contains(point) {
                self.graph.select(index);
                self.graph.dropdown_menu = None;
                self.graph.nodes[index].collapsed = !self.graph.nodes[index].collapsed;
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            // A section header folds or opens its section.
            if let Some(section) = node.section_header_at(point) {
                self.graph.select(index);
                self.graph.dropdown_menu = None;
                self.graph.nodes[index].toggle_section(section);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            {
                let labels = node.switch_labels();
                let hit = (0..labels.len()).find(|&i| node.switch_bounds(i).contains(point));
                if let (Some(hit), false) = (hit, node.collapsed) {
                    self.graph.select(index);
                    // A map's Bipolar switch swaps two of its knobs, so it converts their values
                    // and drops the wires of the pair that leaves.
                    if self.graph.nodes[index].kind == NodeKind::Map && hit == 0 {
                        self.graph.toggle_map_polarity(index);
                        return (event::Status::Captured, Some(Message::GraphChanged));
                    }
                    let node = &mut self.graph.nodes[index];
                    let on = !node.switch(hit);
                    node.set_switch(hit, on);
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
            }
            if let Some(which) = node.dropdown_at(point) {
                self.graph.select(index);
                self.graph.dropdown_menu = Some((index, which));
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if node
                .sample_button_bounds()
                .is_some_and(|bounds| bounds.contains(point))
            {
                self.graph.select(index);
                self.graph.start_sampler_dialog(index);
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
            if node.bounds().contains(point) {
                let in_header = node.header_bounds().contains(point);
                let is_group = node.kind == NodeKind::Group;
                let grab = Vector::new(point.x - node.position.x, point.y - node.position.y);
                // A node of the selection keeps its group, so its header drags all of it.
                if !self.graph.is_selected(index) {
                    self.graph.select(index);
                }
                // A double click on a group's header, or a press on its open glyph, opens it.
                if is_group && in_header && self.press_group_header(index, point, screen) {
                    return (event::Status::Captured, Some(Message::GraphChanged));
                }
                if in_header {
                    // A drag takes the node over from any slide it is in.
                    self.graph.finish_slides();
                    self.graph.interaction = Interaction::MovingNode { node: index, grab };
                }
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }

        // Empty canvas starts a selection box. It clears the selection, unless Shift is held.
        self.graph.box_base = if self.graph.shift {
            self.graph.selection()
        } else {
            self.graph.clear_selection();
            Vec::new()
        };
        self.graph.interaction = Interaction::Boxing {
            start: point,
            cursor: point,
        };
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    /// A press on the viewer of an LFO on Custom. A press on a point starts dragging it, and a
    /// double click on one removes it. A double click on empty space adds a point there. Either
    /// double click leaves no drag running. None if the press is not for the curve, or is the
    /// first press on empty space, which does what a press on the node's body does.
    fn press_lfo_viewer(
        &mut self,
        index: usize,
        point: Point,
        screen: Point,
    ) -> Option<(event::Status, Option<Message>)> {
        let node = &self.graph.nodes[index];
        lfo_handles(node)?;
        let hit = lfo_handle_at(node, point);
        if hit.is_none() && !node.viewer_bounds()?.contains(point) {
            return None;
        }
        let double = knob::double_click(&mut self.graph.viewer_click, screen);
        match (hit, double) {
            (Some(handle), false) => {
                self.graph.select(index);
                self.graph.interaction = Interaction::LfoPoint {
                    node: index,
                    point: handle,
                };
            }
            (Some(handle), true) => {
                self.graph.select(index);
                remove_lfo_point(&mut self.graph.nodes[index], handle);
                self.graph.interaction = Interaction::Idle;
            }
            (None, true) => {
                self.graph.select(index);
                if add_lfo_point(&mut self.graph.nodes[index], point) == LfoAdd::Full {
                    self.graph.say(format!(
                        "A custom wave holds up to {MAX_CURVE_POINTS} points"
                    ));
                }
                self.graph.interaction = Interaction::Idle;
            }
            (None, false) => return None,
        }
        Some((event::Status::Captured, Some(Message::GraphChanged)))
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
        // The group shortcuts act on the selection, or on the open group.
        if let Some(changed) = self.run_group_shortcut(shortcut) {
            return changed;
        }
        // The quick add box needs no selection. It waits while a node is being placed, so one
        // press sets down one node.
        if shortcut == Shortcut::QuickAdd {
            if matches!(self.graph.interaction, Interaction::Placing { .. }) {
                return false;
            }
            self.graph.open_quick_add();
            return true;
        }
        let Some(index) = self.graph.selected else {
            return false;
        };
        match shortcut {
            Shortcut::QuickAdd | Shortcut::Group | Shortcut::Ungroup | Shortcut::Escape => false,
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
            // Edit mode's letter shortcuts never reach here: `shortcut_event` looks a key up
            // with `lookup`, which does not know about them. `edit_shortcut_event` runs them.
            Shortcut::Grab
            | Shortcut::DeleteSelection
            | Shortcut::DeleteInPlace
            | Shortcut::SelectAll
            | Shortcut::DeselectAll => false,
        }
    }

    /// Handles a key event for Edit mode's own shortcuts: G, X, Delete, A, and Alt+A. None if no
    /// Edit-mode shortcut uses the key, so it is simply swallowed (Edit mode runs no piano).
    pub(super) fn edit_shortcut_event(
        &mut self,
        event: &keyboard::Event,
    ) -> Option<(event::Status, Option<Message>)> {
        match event {
            keyboard::Event::KeyPressed {
                key_code,
                modifiers,
            } => {
                let shortcut = shortcuts::lookup_edit(*key_code, *modifiers)?;
                if self.graph.shortcut_held == Some(*key_code) {
                    return Some((event::Status::Captured, None));
                }
                self.graph.shortcut_held = Some(*key_code);
                let changed = self.run_edit_shortcut(shortcut);
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

    /// Runs an Edit-mode shortcut. Returns true if the graph changed. Grab only starts a modal
    /// interaction, so it never reports a change here: only committing or canceling it does, and
    /// those go through `commit_grab` and `cancel_grab` instead.
    fn run_edit_shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Grab => {
                // A plain G with nothing selected does nothing.
                if !self.graph.selection().is_empty() {
                    self.graph.start_grab();
                }
                false
            }
            Shortcut::DeleteSelection => {
                if self.graph.selection().is_empty() {
                    false
                } else {
                    self.graph.delete_selected();
                    true
                }
            }
            Shortcut::DeleteInPlace => {
                if self.graph.selection().is_empty() {
                    false
                } else {
                    self.graph.delete_selected_in_place();
                    true
                }
            }
            Shortcut::SelectAll => {
                self.graph.select_all();
                true
            }
            Shortcut::DeselectAll => {
                self.graph.deselect_all();
                true
            }
            _ => false,
        }
    }

    /// True if an open dropdown list, node menu, or burger menu list covers the cursor. These are
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
            if menu_bounds(menu).contains(screen) {
                return true;
            }
        }
        if self.graph.burger_menu && burger_row_at(screen).is_some() {
            return true;
        }
        if let Some(category) = self.graph.category_menu {
            if category_row_at(bounds, category, screen).is_some() {
                return true;
            }
        }
        if let Some(menu) = &self.graph.preset_list {
            if preset_row_at(bounds, menu, screen).is_some() {
                return true;
            }
        }
        if let Some(entries) = &self.graph.group_list {
            if group_row_at(bounds, entries.len(), screen).is_some() {
                return true;
            }
        }
        if let Some(rename) = &self.graph.rename {
            if rename_bounds(bounds, rename.screen).contains(screen) {
                return true;
            }
        }
        self.graph.quick_add.is_some()
    }

    /// Handles a key event while the quick add box is open. The box takes every key, so the piano
    /// and the shortcuts stay quiet. Typing narrows the results, the arrows move the lit one, and
    /// Enter adds its node, which then follows the cursor. Escape closes the box.
    pub(super) fn quick_add_event(
        &mut self,
        event: &keyboard::Event,
    ) -> (event::Status, Option<Message>) {
        let results = self.graph.quick_add_picks();
        match event {
            keyboard::Event::CharacterReceived(c) => {
                if let Some(open) = self.graph.quick_add.as_mut() {
                    let opening_key = open.skip_char && matches!(c, 'a' | 'A');
                    open.skip_char = false;
                    if !opening_key && !c.is_control() && open.query.chars().count() < 24 {
                        open.query.push(*c);
                        open.selected = 0;
                    }
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyPressed { key_code, .. } => {
                // A held Shift+A repeats its press, which is not typing.
                if self.graph.shortcut_held == Some(*key_code) {
                    return (event::Status::Captured, None);
                }
                let Some(open) = self.graph.quick_add.as_mut() else {
                    return (event::Status::Ignored, None);
                };
                open.skip_char = false;
                match key_code {
                    keyboard::KeyCode::Backspace => {
                        open.query.pop();
                        open.selected = 0;
                    }
                    keyboard::KeyCode::Down if !results.is_empty() => {
                        open.selected = (open.selected + 1) % results.len();
                    }
                    keyboard::KeyCode::Up if !results.is_empty() => {
                        open.selected = (open.selected + results.len() - 1) % results.len();
                    }
                    keyboard::KeyCode::Escape => self.graph.quick_add = None,
                    keyboard::KeyCode::Enter | keyboard::KeyCode::NumpadEnter => {
                        let (world, feed) = (open.world, open.feed);
                        let pick = results.get(open.selected).copied();
                        let groups = std::mem::take(&mut open.groups);
                        self.graph.quick_add = None;
                        if let Some(pick) = pick {
                            self.place_pick(pick, world, feed, &groups);
                            return (event::Status::Captured, Some(Message::GraphChanged));
                        }
                    }
                    _ => {}
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyReleased { key_code, .. } => {
                if self.graph.shortcut_held == Some(*key_code) {
                    self.graph.shortcut_held = None;
                }
                let note = self.graph.piano.handle(event.clone());
                let note = note.filter(|_| self.graph.mode == EditorMode::Keyboard);
                (event::Status::Captured, note.map(Message::Note))
            }
            _ => (event::Status::Ignored, None),
        }
    }

    /// Opens the submenu of the category under the cursor, and closes it when the cursor moves onto
    /// another row of the list. Over the submenu itself, or off the list, it stays as it is, so
    /// the cursor can cross the gap to reach it. Returns true if the submenu changed.
    pub(super) fn hover_preset_menu(&mut self, bounds: Rectangle, screen: Point) -> bool {
        let Some(menu) = self.graph.preset_list.as_mut() else {
            return false;
        };
        let open = match preset_row_at(bounds, menu, screen) {
            Some(PresetRow::Category(index)) => Some(index),
            Some(PresetRow::SaveAs | PresetRow::Init | PresetRow::Loose(_)) => None,
            Some(PresetRow::Sub(..)) | None => menu.open,
        };
        let changed = open != menu.open;
        menu.open = open;
        changed
    }

    /// Handles a key event while the number box is open. It takes digits, a sign, a point, and
    /// "inf" for the silent end of a gain, and the box takes every key, so the piano and the
    /// shortcuts stay quiet. Enter sets the knob, and Escape closes the box. A key already held
    /// when the box opened still gets its release, so no note sticks.
    pub(super) fn knob_entry_event(
        &mut self,
        event: &keyboard::Event,
    ) -> (event::Status, Option<Message>) {
        match event {
            keyboard::Event::CharacterReceived(c) => {
                if let Some(entry) = self.graph.knob_entry.as_mut() {
                    if "0123456789.+-infINF".contains(*c) {
                        // The first key replaces the selected value.
                        if entry.fresh {
                            entry.text.clear();
                            entry.fresh = false;
                        }
                        if entry.text.chars().count() < KNOB_ENTRY_MAX_LEN {
                            entry.text.push(*c);
                        }
                    }
                }
                (event::Status::Captured, Some(Message::Hovered))
            }
            keyboard::Event::KeyPressed { key_code, .. } => match key_code {
                keyboard::KeyCode::Backspace => {
                    if let Some(entry) = self.graph.knob_entry.as_mut() {
                        if entry.fresh {
                            entry.text.clear();
                            entry.fresh = false;
                        } else {
                            entry.text.pop();
                        }
                    }
                    (event::Status::Captured, Some(Message::Hovered))
                }
                keyboard::KeyCode::Escape => {
                    self.graph.knob_entry = None;
                    (event::Status::Captured, Some(Message::Hovered))
                }
                keyboard::KeyCode::Enter | keyboard::KeyCode::NumpadEnter => {
                    let changed = self.graph.confirm_knob_entry();
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

    /// Adds a node of `kind` centered on `point` and makes it follow the cursor until the next
    /// press places it. A kind that is one per graph is not added while the graph has one.
    pub(super) fn place(&mut self, kind: NodeKind, point: Point) {
        if !self.graph.can_add(kind) {
            return;
        }
        let node = self.graph.add_node(kind, point);
        self.graph.start_placing(node, point);
    }

    /// `place`, and when the node is for an input a wire was dragged back from, wires its output
    /// to that input. The wire stays on the node while it follows the cursor.
    pub(super) fn place_fed(&mut self, kind: NodeKind, point: Point, feed: Option<(usize, Port)>) {
        let before = self.graph.nodes.len();
        self.place(kind, point);
        let Some((to, input)) = feed else {
            return;
        };
        if self.graph.nodes.len() == before {
            return;
        }
        if let Some(out) = self.graph.feeding_port(kind, to, input) {
            self.graph.connect(before, out, to, input);
        }
    }

    /// Places the quick add result `pick`. A kind is added with `place_fed`. A saved group is
    /// loaded from its file, and wired to the input a wire was dragged back from if there was one.
    pub(super) fn place_pick(
        &mut self,
        pick: QuickPick,
        point: Point,
        feed: Option<(usize, Port)>,
        groups: &[QuickGroup],
    ) {
        match pick {
            QuickPick::Kind(kind) => self.place_fed(kind, point, feed),
            QuickPick::Group(index) => {
                if let Some(group) = groups.get(index) {
                    self.place_group(&group.entry, point, feed, group.out);
                }
            }
        }
    }

    /// Adds a copy of a saved group and makes it follow the cursor, like a node just added from
    /// the burger menu. With a feed and an output pin, the first pin that fits is wired to the
    /// input. A
    /// file that can't be loaded adds nothing and says why.
    pub(super) fn place_group(
        &mut self,
        entry: &GroupEntry,
        point: Point,
        feed: Option<(usize, Port)>,
        out: Option<Port>,
    ) {
        let Some(node) = self.graph.add_saved_group(entry, point) else {
            return;
        };
        self.graph.start_placing(node, point);
        if let (Some((to, input)), Some(out)) = (feed, out) {
            self.graph.connect(node, out, to, input);
        }
    }

    /// Copies a node and makes the copy follow the cursor, like a node just added from the burger
    /// menu, until the next press sets it down. Returns false for a node that can't be copied.
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
    pub(super) fn secondary_press(
        &mut self,
        point: Point,
        screen: Point,
    ) -> (event::Status, Option<Message>) {
        // A right click while grabbing cancels it, putting the selection back where it started,
        // instead of opening a menu.
        if self.graph.cancel_grab() {
            return (event::Status::Captured, Some(Message::Hovered));
        }
        self.graph.menu = None;
        self.graph.quick_add = None;
        self.graph.dropdown_menu = None;
        // Shift and right drag from a node wires it to the node the drag ends on.
        if self.graph.shift {
            if let Some(from) = self.graph.node_at(point) {
                self.graph.interaction = Interaction::AutoLinking {
                    from,
                    start: point,
                    cursor: point,
                };
                return (event::Status::Captured, Some(Message::GraphChanged));
            }
        }
        // A press on the margin of an open pane is not on the graph.
        if self.graph.group_depth() > 0
            && !pane_bounds(canvas_rect(self.graph.canvas_size)).contains(screen)
        {
            return (event::Status::Captured, Some(Message::Hovered));
        }
        // A pin of a group, or a row of the macro node, has a menu of its own.
        if self.secondary_group_target(point, screen) {
            return (event::Status::Captured, Some(Message::GraphChanged));
        }
        if let Some(index) = self.graph.node_at(point) {
            // The edge nodes of a group can't be copied, switched off, or deleted, so they have
            // no menu.
            if matches!(
                self.graph.nodes[index].kind,
                NodeKind::GroupInput | NodeKind::GroupOutput
            ) {
                return (event::Status::Captured, Some(Message::Hovered));
            }
            // A right press on a node of a group keeps the group, so the menu acts on all of it.
            if !self.graph.is_selected(index) {
                self.graph.select(index);
            }
            let node = &self.graph.nodes[index];
            let items = if self.graph.selection().len() > 1 {
                let groupable = self.graph.selection().iter().any(|&member| {
                    !matches!(
                        self.graph.nodes[member].kind,
                        NodeKind::Output
                            | NodeKind::Macro
                            | NodeKind::GroupInput
                            | NodeKind::GroupOutput
                    )
                });
                group_menu_items(node, groupable)
            } else {
                menu_items(node)
            };
            self.graph.menu = Some(ContextMenu {
                node: index,
                position: screen,
                items,
                target: MenuTarget::Node,
            });
        }
        (event::Status::Captured, Some(Message::GraphChanged))
    }

    /// Right release: ends an auto connect drag. Over a node other than the one it began on, that
    /// wires the two.
    pub(super) fn release_secondary(&mut self, point: Point) -> (event::Status, Option<Message>) {
        let Interaction::AutoLinking { from, .. } = self.graph.interaction else {
            return (event::Status::Ignored, None);
        };
        self.graph.interaction = Interaction::Idle;
        let changed = self
            .graph
            .node_at(point)
            .filter(|&to| to != from)
            .is_some_and(|to| self.graph.auto_connect(from, to));
        (
            event::Status::Captured,
            Some(if changed {
                Message::GraphChanged
            } else {
                Message::Hovered
            }),
        )
    }

    pub(super) fn release(
        &mut self,
        point: Point,
        bounds: Rectangle,
    ) -> (event::Status, Option<Message>) {
        // A click on the margin round an open pane closes the group.
        if let Some(result) = self.release_margin(bounds) {
            return result;
        }
        // Placing ends on the next press, not on the release of the press that started it.
        if let Interaction::Placing { .. } = self.graph.interaction {
            return (event::Status::Captured, None);
        }
        let interaction = std::mem::replace(&mut self.graph.interaction, Interaction::Idle);
        match interaction {
            Interaction::Wiring { from, out, .. } => {
                if let Some(Terminal::In(to, input)) = self.graph.terminal_at(point) {
                    // A drop on a group's empty pin, or a drag from one, makes a pin first.
                    self.graph.drop_wire(from, out, to, input);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::BackWiring { to, input, .. } => {
                match self.graph.terminal_at(point) {
                    Some(Terminal::Out(from, out)) => {
                        self.graph.drop_wire(from, out, to, input);
                    }
                    // Over another pin, or over a node, nothing happens.
                    Some(Terminal::In(..)) => {}
                    None if self.graph.node_at(point).is_some() => {}
                    // The empty pin has no kind of node that feeds it.
                    None if input == Port::NewPin => {}
                    // Empty canvas. The cursor is where the box opens and the node lands.
                    None => {
                        let screen = self.graph.last_screen;
                        if !self.overlay_at(point, screen, bounds)
                            && !piano::strip_bounds(bounds).contains(screen)
                        {
                            self.graph.open_quick_add_feeding(to, input, point);
                        }
                    }
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
            Interaction::EqPoint { node, band } => {
                knob::release(&mut self.graph.nodes[node].knob_states[eq::FREQ[band] as usize]);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::EnvelopeHandle { node, handle } => {
                let first = envelope::HANDLE_KNOBS[handle][0];
                knob::release(&mut self.graph.nodes[node].knob_states[first as usize]);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::LfoPoint { .. } => (event::Status::Captured, Some(Message::GraphChanged)),
            Interaction::MovingNode { node, .. } => {
                if let Some(link) = self.graph.link_target.take() {
                    self.graph.insert_into(node, link);
                }
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::Boxing { .. } => {
                self.graph.box_base.clear();
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            // The left button ending an auto connect drag cancels it.
            Interaction::AutoLinking { .. } => (event::Status::Captured, Some(Message::Hovered)),
            Interaction::Placing { .. } => (event::Status::Captured, None),
            // A grab commits on the press that starts a click, like Placing does, so its own
            // release here does nothing. Reaching this arm at all would mean a button was
            // already down when the grab started, which the editor does not let happen.
            Interaction::Grabbing { .. } => (event::Status::Ignored, None),
            Interaction::Idle => (event::Status::Ignored, None),
        }
    }

    pub(super) fn drag(&mut self, point: Point, screen: Point) -> (event::Status, Option<Message>) {
        match self.graph.interaction {
            Interaction::MovingNode { node, grab }
            | Interaction::Placing { node, grab }
            | Interaction::Grabbing { node, grab } => {
                let target = Point::new(point.x - grab.x, point.y - grab.y);
                let at = self.graph.nodes[node].position;
                // The rest of a selection moves by as much as the dragged node does.
                self.graph
                    .move_selection(Vector::new(target.x - at.x, target.y - at.y));
                self.graph.nodes[node].position = target;
                // Only a lone node splices into a wire.
                self.graph.link_target = if self.graph.selection().len() == 1 {
                    self.graph.link_under(node)
                } else {
                    None
                };
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::AutoLinking { from, start, .. } => {
                self.graph.interaction = Interaction::AutoLinking {
                    from,
                    start,
                    cursor: point,
                };
                (event::Status::Captured, Some(Message::Hovered))
            }
            Interaction::Boxing { start, .. } => {
                self.graph.interaction = Interaction::Boxing {
                    start,
                    cursor: point,
                };
                self.graph.select_in_box(start, point);
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
            Interaction::BackWiring { to, input, .. } => {
                self.graph.interaction = Interaction::BackWiring {
                    to,
                    input,
                    cursor: point,
                };
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::MapHandle { node, end } => {
                // The handle follows the cursor over the curve box. Its x is the input, and its y
                // is the output. A value a wire drives stays where the wire puts it.
                // A bipolar map sets its output from Center and Range together, so a wire on
                // either one leaves the output alone.
                let output_knobs = if crate::nodes::map::bipolar(&self.graph.nodes[node]) {
                    MAP_CENTER_RANGE.to_vec()
                } else {
                    vec![MAP_OUT[end]]
                };
                let driven = MapDriven {
                    input: self.graph.is_driven(node, MAP_IN[end]),
                    output: output_knobs
                        .into_iter()
                        .any(|id| self.graph.is_driven(node, id)),
                };
                let live = live_knobs(self.graph, node, &self.levels);
                let offset = crate::nodes::map::offset(&self.graph.nodes[node], &live);
                drag_map_handle(&mut self.graph.nodes[node], end, point, offset, driven);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::EqPoint { node, band } => {
                let driven = EqDriven {
                    freq: self.graph.is_driven(node, eq::FREQ[band]),
                    gain: self.graph.is_driven(node, eq::GAIN[band]),
                };
                drag_eq_point(&mut self.graph.nodes[node], band, point, driven);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::EnvelopeHandle { node, handle } => {
                let knobs = envelope::HANDLE_KNOBS[handle];
                let driven = EnvelopeDriven {
                    time: self.graph.is_driven(node, knobs[0]),
                    level: knobs
                        .get(1)
                        .is_some_and(|id| self.graph.is_driven(node, *id)),
                };
                let live = live_knobs(self.graph, node, &self.levels);
                drag_envelope_handle(&mut self.graph.nodes[node], handle, point, &live, driven);
                (event::Status::Captured, Some(Message::GraphChanged))
            }
            Interaction::LfoPoint { node, point: index } => {
                drag_lfo_point(&mut self.graph.nodes[node], index, point);
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
        // The pane of an open group follows the editor's size.
        if bounds.width > 0.0 && bounds.height > 0.0 && bounds.size() != self.graph.canvas_size {
            self.graph.canvas_size = bounds.size();
            self.graph.refit_view();
        }
        let mouse_event = match event {
            Event::Mouse(mouse_event) => mouse_event,
            Event::Keyboard(key_event) => {
                // Shift+click and Shift+drag read the modifiers from the last key event.
                match &key_event {
                    keyboard::Event::KeyPressed { modifiers, .. }
                    | keyboard::Event::KeyReleased { modifiers, .. }
                    | keyboard::Event::ModifiersChanged(modifiers) => {
                        self.graph.shift = modifiers.shift();
                        self.graph.alt = modifiers.alt();
                    }
                    _ => {}
                }
                // The number box takes the keys while it is open.
                if self.graph.knob_entry.is_some() {
                    return self.knob_entry_event(&key_event);
                }
                // So does the rename box.
                if self.graph.rename.is_some() {
                    return self.rename_event(&key_event);
                }
                // So does the quick add box.
                if self.graph.quick_add.is_some() {
                    return self.quick_add_event(&key_event);
                }
                // A grab in progress takes every key: Enter commits it, Escape cancels it, and
                // anything else is swallowed so no shortcut or note fires in the middle of one.
                if matches!(self.graph.interaction, Interaction::Grabbing { .. }) {
                    return match &key_event {
                        keyboard::Event::KeyPressed {
                            key_code: keyboard::KeyCode::Enter | keyboard::KeyCode::NumpadEnter,
                            ..
                        } => {
                            self.graph.commit_grab();
                            (event::Status::Captured, Some(Message::GraphChanged))
                        }
                        keyboard::Event::KeyPressed {
                            key_code: keyboard::KeyCode::Escape,
                            ..
                        } => {
                            self.graph.cancel_grab();
                            (event::Status::Captured, Some(Message::Hovered))
                        }
                        _ => (event::Status::Captured, None),
                    };
                }
                // Tab switches between Keyboard and Edit mode everywhere, so it is checked before
                // any shortcut and before the piano sees the key. The boxes above still take it
                // first: the knob entry and quick add boxes already swallow Tab without using it
                // for anything, and the rename box's own doc comment already says a Tab typed in
                // it is not a shortcut, so toggling the mode mid-edit would be a surprise with
                // nothing to show for it besides the box staying open anyway.
                if let keyboard::Event::KeyPressed {
                    key_code: keyboard::KeyCode::Tab,
                    ..
                } = key_event
                {
                    if self.graph.shortcut_held == Some(keyboard::KeyCode::Tab) {
                        return (event::Status::Captured, None);
                    }
                    self.graph.shortcut_held = Some(keyboard::KeyCode::Tab);
                    self.graph.mode = self.graph.mode.toggled();
                    return (event::Status::Captured, Some(Message::Hovered));
                }
                if let keyboard::Event::KeyReleased {
                    key_code: keyboard::KeyCode::Tab,
                    ..
                } = key_event
                {
                    if self.graph.shortcut_held == Some(keyboard::KeyCode::Tab) {
                        self.graph.shortcut_held = None;
                    }
                    return (event::Status::Captured, None);
                }
                // Shortcuts come first, so one on a piano key (Shift+D) is not also a note.
                if let Some(result) = self.shortcut_event(&key_event) {
                    return result;
                }
                // Edit mode hands the keyboard to the editor instead of the piano.
                if self.graph.mode == EditorMode::Edit {
                    if let Some(result) = self.edit_shortcut_event(&key_event) {
                        return result;
                    }
                    return (event::Status::Ignored, None);
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
        self.graph.cursor_in = position.is_some();
        let released = matches!(
            mouse_event,
            mouse::Event::ButtonReleased(mouse::Button::Left | mouse::Button::Right)
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
                // The octave buttons shift the keys, as Z and X do.
                if let Some(direction) = piano::octave_button_at(strip, screen) {
                    self.graph.piano.shift_octave(direction);
                    return (event::Status::Captured, None);
                }
                let note = piano::key_at(strip, piano::low_note(self.graph.piano.octave()), screen);
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
                self.secondary_press(point, screen)
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => self.release(point, bounds),
            mouse::Event::ButtonReleased(mouse::Button::Right) => self.release_secondary(point),
            mouse::Event::CursorMoved { .. } => {
                let mut submenu_changed = self.hover_preset_menu(bounds, screen);
                // The result under the cursor is the lit one.
                let rows = self.graph.quick_add_picks().len();
                if let Some(open) = self.graph.quick_add.as_mut() {
                    let area = quick_add_bounds(bounds, open.screen, rows);
                    if let Some(row) = quick_add_row_at(area, screen, rows) {
                        submenu_changed |= open.selected != row;
                        open.selected = row;
                    }
                }
                let hover = self.graph.hover_at(point);
                let changed = hover != self.graph.hover || submenu_changed;
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
                // Over an open pane the wheel works on the pane's graph, and nowhere else.
                if self.graph.group_depth() > 0 && !pane_bounds(bounds).contains(screen) {
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
