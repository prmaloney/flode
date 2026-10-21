//! Tests for the group interface: the shortcuts, the pane, the empty pins, and renaming. They
//! drive the canvas the way the window does, with key and mouse events.

use super::canvas::{Message, NodeEditor};
use super::graph::{Graph, Interaction, DEFAULT_OUTPUT_X};
use super::group::PinSide;
use super::group_ui::{
    canvas_rect, close_bounds, inner_bounds, open_glyph_bounds, pane_bounds, title_bounds,
    title_layout, RenameTarget, CLICK_SLOP,
};
use super::menus::{pill_bounds, pill_segment, MenuItem, MenuTarget};
use super::piano;
use super::shortcuts::COMMAND;
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::macros::MACRO_COUNT;
use crate::nodes::{FlodeNode, NodeKind, Port, Signal};
use crate::patch::control::ControlLevels;
use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, event::Status, Cursor, Program};
use nih_plug_iced::{Point, Rectangle, Size};

const INPUT: usize = 0;
const OSC: usize = 1;
const FILTER: usize = 2;
const REVERB: usize = 3;
const OUTPUT: usize = 4;
const LFO: usize = 5;

/// The group node's place after `FILTER` and `REVERB` go into it.
const GROUP: usize = 2;

fn bounds() -> Rectangle {
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    }
}

fn node(kind: NodeKind, x: f32) -> FlodeNode {
    FlodeNode::new(kind, Point::new(x, 80.0))
}

fn wire(from: usize, out: Port, to: usize, input: Port) -> Connection {
    Connection {
        from,
        out,
        to,
        input,
    }
}

fn audio_pin(n: usize) -> Port {
    Port::Pin(Signal::Audio, n)
}

fn control_pin(n: usize) -> Port {
    Port::Pin(Signal::Control, n)
}

/// A MIDI input into an oscillator, through a filter and a reverb to the output, with an LFO
/// driving the filter's cutoff.
fn chain() -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Oscillator, 200.0),
            node(NodeKind::Filter, 400.0),
            node(NodeKind::Reverb, 600.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
            node(NodeKind::Lfo, 300.0),
        ],
        vec![
            wire(INPUT, Port::Notes, OSC, Port::Notes),
            wire(OSC, Port::Audio, FILTER, Port::Audio),
            wire(FILTER, Port::Audio, REVERB, Port::Audio),
            wire(REVERB, Port::Audio, OUTPUT, Port::Slot(0)),
            wire(LFO, Port::Shape, FILTER, Port::Knob(KnobId::Cutoff)),
        ],
    );
    graph.refresh_slots();
    graph
}

/// The chain with the filter and the reverb in a group, which is node 2 and is selected.
fn grouped() -> Graph {
    let mut graph = chain();
    assert_eq!(graph.create_group(&[FILTER, REVERB]), Some(GROUP));
    graph
}

/// A grouped chain with the group open. Inside, the edge nodes are 0 and 1, the filter is 2 and
/// the reverb is 3.
fn opened() -> Graph {
    let mut graph = grouped();
    assert!(graph.open_group(GROUP));
    graph
}

fn canvas(graph: &mut Graph) -> NodeEditor<'_> {
    NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
}

/// Presses a key and lets it go, and returns what the press sent.
fn key(editor: &mut NodeEditor, key_code: KeyCode, modifiers: Modifiers) -> Option<Message> {
    let message = editor
        .update(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key_code,
                modifiers,
            }),
            bounds(),
            Cursor::Unavailable,
        )
        .1;
    editor.update(
        Event::Keyboard(keyboard::Event::KeyReleased {
            key_code,
            modifiers: Modifiers::default(),
        }),
        bounds(),
        Cursor::Unavailable,
    );
    message
}

fn plain(editor: &mut NodeEditor, key_code: KeyCode) -> Option<Message> {
    key(editor, key_code, Modifiers::default())
}

fn type_text(editor: &mut NodeEditor, text: &str) {
    for c in text.chars() {
        editor.update(
            Event::Keyboard(keyboard::Event::CharacterReceived(c)),
            bounds(),
            Cursor::Unavailable,
        );
    }
}

fn mouse_at(editor: &mut NodeEditor, event: mouse::Event, at: Point) -> (Status, Option<Message>) {
    editor.update(Event::Mouse(event), bounds(), Cursor::Available(at))
}

fn press(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(editor, mouse::Event::ButtonPressed(mouse::Button::Left), at).1
}

fn release(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    mouse_at(
        editor,
        mouse::Event::ButtonReleased(mouse::Button::Left),
        at,
    )
    .1
}

fn click(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    let message = press(editor, at);
    release(editor, at);
    message
}

/// A drag from one point to another.
fn drag(editor: &mut NodeEditor, from: Point, to: Point) {
    press(editor, from);
    mouse_at(editor, mouse::Event::CursorMoved { position: to }, to);
    release(editor, to);
}

fn right_click(editor: &mut NodeEditor, at: Point) {
    mouse_at(
        editor,
        mouse::Event::ButtonPressed(mouse::Button::Right),
        at,
    );
    mouse_at(
        editor,
        mouse::Event::ButtonReleased(mouse::Button::Right),
        at,
    );
}

/// Where the screen shows a point of the graph.
fn shown(graph: &Graph, world: Point) -> Point {
    graph.view.to_screen(world)
}

fn out_pin(graph: &Graph, node: usize, port: Port) -> Point {
    shown(graph, graph.nodes[node].output_point(port).expect("pin"))
}

fn in_pin(graph: &Graph, node: usize, port: Port) -> Point {
    shown(graph, graph.nodes[node].input_point(port).expect("pin"))
}

/// The middle of row `row` of the open menu, in screen coordinates.
fn menu_row(graph: &Graph, row: usize) -> Point {
    let menu = graph.menu.expect("a menu is open");
    Point::new(
        menu.position.x + 10.0,
        menu.position.y + (row as f32 + 0.5) * crate::editor::style::MENU_ITEM_HEIGHT,
    )
}

fn menu_has(graph: &Graph, item: MenuItem) -> bool {
    graph.menu.is_some_and(|menu| menu.items.contains(&item))
}

/// Clicks the entry `item` of the open menu.
fn choose(editor: &mut NodeEditor, item: MenuItem) {
    let menu = editor.graph.menu.expect("a menu is open");
    let row = menu
        .items
        .iter()
        .position(|&candidate| candidate == item)
        .expect("the menu has the entry");
    let at = menu_row(editor.graph, row);
    click(editor, at);
}

fn pin_names(pins: &[crate::nodes::group::GroupPin]) -> Vec<String> {
    pins.iter().map(|pin| pin.name.clone()).collect()
}

fn changed(message: Option<Message>) -> bool {
    matches!(message, Some(Message::GraphChanged))
}

// Making and breaking groups.

#[test]
fn the_command_key_and_g_makes_a_group_of_the_selection() {
    let mut graph = chain();
    graph.set_selection(vec![FILTER, REVERB]);
    let mut editor = canvas(&mut graph);
    assert!(changed(key(&mut editor, KeyCode::G, COMMAND)));
    let kinds: Vec<NodeKind> = graph.nodes.iter().map(|n| n.kind).collect();
    assert_eq!(kinds.iter().filter(|&&k| k == NodeKind::Group).count(), 1);
    assert!(!kinds.contains(&NodeKind::Filter));
    assert_eq!(graph.selection(), vec![GROUP]);
    assert_eq!(graph.group_depth(), 0);
}

#[test]
fn a_plain_g_is_not_a_group_shortcut() {
    let mut graph = chain();
    graph.set_selection(vec![FILTER, REVERB]);
    let mut editor = canvas(&mut graph);
    plain(&mut editor, KeyCode::G);
    assert!(graph.nodes.iter().all(|n| n.kind != NodeKind::Group));
}

#[test]
fn grouping_with_nothing_groupable_does_nothing() {
    let mut graph = chain();
    let before = graph.nodes.len();
    let mut editor = canvas(&mut graph);
    // Nothing is selected.
    assert!(!changed(key(&mut editor, KeyCode::G, COMMAND)));
    // The output node can't go in a group.
    editor.graph.select(OUTPUT);
    assert!(!changed(key(&mut editor, KeyCode::G, COMMAND)));
    assert_eq!(graph.nodes.len(), before);
    assert!(graph.nodes.iter().all(|n| n.kind != NodeKind::Group));
}

#[test]
fn command_shift_g_ungroups_the_selected_group() {
    let mut graph = grouped();
    let mut editor = canvas(&mut graph);
    let both = COMMAND | Modifiers::SHIFT;
    assert!(changed(key(&mut editor, KeyCode::G, both)));
    let kinds: Vec<NodeKind> = graph.nodes.iter().map(|n| n.kind).collect();
    assert!(!kinds.contains(&NodeKind::Group));
    assert!(kinds.contains(&NodeKind::Filter) && kinds.contains(&NodeKind::Reverb));
}

#[test]
fn ungrouping_a_node_that_is_not_a_group_does_nothing() {
    let mut graph = chain();
    graph.select(FILTER);
    let mut editor = canvas(&mut graph);
    assert!(!changed(key(
        &mut editor,
        KeyCode::G,
        COMMAND | Modifiers::SHIFT
    )));
    assert_eq!(graph.nodes.len(), 6);
}

#[test]
fn the_menus_offer_group_and_ungroup() {
    // A group node offers open, rename, and ungroup.
    let mut graph = grouped();
    graph.clear_selection();
    let header = graph.nodes[GROUP].position;
    let at = Point::new(header.x + 90.0, header.y + 14.0);
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    for item in [MenuItem::Open, MenuItem::Rename, MenuItem::Ungroup] {
        assert!(menu_has(editor.graph, item), "{item:?}");
    }
    choose(&mut editor, MenuItem::Ungroup);
    assert!(graph.nodes.iter().all(|n| n.kind != NodeKind::Group));

    // Several selected nodes offer group.
    let mut graph = chain();
    graph.set_selection(vec![FILTER, REVERB]);
    let at = Point::new(480.0, 94.0);
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    assert!(menu_has(editor.graph, MenuItem::Group));
    choose(&mut editor, MenuItem::Group);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Group)
            .count(),
        1
    );

    // A selection of nodes that can't go in a group does not.
    let mut graph = chain();
    graph.nodes[LFO].position = Point::new(700.0, 300.0);
    let macros = graph.add_node(NodeKind::Macro, Point::new(700.0, 500.0));
    graph.set_selection(vec![macros, OUTPUT]);
    let output = graph.nodes[OUTPUT].position;
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, Point::new(output.x + 60.0, output.y + 14.0));
    assert!(editor.graph.menu.is_some());
    assert!(!menu_has(editor.graph, MenuItem::Group));
}

// Opening and closing.

#[test]
fn tab_opens_the_selected_group_and_closes_the_open_one() {
    let mut graph = grouped();
    let mut editor = canvas(&mut graph);
    assert!(changed(plain(&mut editor, KeyCode::Tab)));
    assert_eq!(editor.graph.group_depth(), 1);
    // Nothing is selected inside, so Tab closes it.
    assert!(changed(plain(&mut editor, KeyCode::Tab)));
    assert_eq!(editor.graph.group_depth(), 0);
}

#[test]
fn tab_with_no_group_selected_and_none_open_does_nothing() {
    let mut graph = chain();
    graph.select(FILTER);
    let mut editor = canvas(&mut graph);
    let message = plain(&mut editor, KeyCode::Tab);
    assert!(message.is_none());
    assert_eq!(graph.group_depth(), 0);
    assert_eq!(graph.selection(), vec![FILTER]);
}

#[test]
fn tab_does_not_play_a_note() {
    let mut graph = grouped();
    let mut editor = canvas(&mut graph);
    assert!(!matches!(
        plain(&mut editor, KeyCode::Tab),
        Some(Message::Note(_))
    ));
}

#[test]
fn tab_inside_a_group_opens_a_nested_group_and_leaves_one_level_at_a_time() {
    let mut graph = opened();
    // The filter goes into a group of its own, inside the open one.
    assert_eq!(graph.create_group(&[2]), Some(2));
    assert_eq!(graph.selection(), vec![2]);
    let mut editor = canvas(&mut graph);
    plain(&mut editor, KeyCode::Tab);
    assert_eq!(editor.graph.group_depth(), 2);
    // Nothing is selected in the nested group, so Tab closes it. The group is selected again in
    // the graph outside, which is where it was opened from.
    plain(&mut editor, KeyCode::Tab);
    assert_eq!(editor.graph.group_depth(), 1);
    assert_eq!(editor.graph.selection(), vec![2]);
    plain(&mut editor, KeyCode::Escape);
    assert_eq!(editor.graph.group_depth(), 0);
}

#[test]
fn escape_closes_the_open_group_but_a_menu_goes_first() {
    let mut graph = opened();
    let mut editor = canvas(&mut graph);
    // A right press on a node inside opens its menu.
    let filter = editor.graph.nodes[2].position;
    let at = shown(editor.graph, Point::new(filter.x + 60.0, filter.y + 14.0));
    right_click(&mut editor, at);
    assert!(editor.graph.menu.is_some());
    plain(&mut editor, KeyCode::Escape);
    assert!(editor.graph.menu.is_none());
    assert_eq!(editor.graph.group_depth(), 1);
    plain(&mut editor, KeyCode::Escape);
    assert_eq!(editor.graph.group_depth(), 0);
    // At the root, Escape has nothing to close.
    assert!(!changed(plain(&mut editor, KeyCode::Escape)));
}

#[test]
fn a_text_box_keeps_tab_and_escape_for_itself() {
    // The quick add box.
    let mut graph = opened();
    let mut editor = canvas(&mut graph);
    key(&mut editor, KeyCode::A, Modifiers::SHIFT);
    assert!(editor.graph.quick_add.is_some());
    plain(&mut editor, KeyCode::Tab);
    assert_eq!(editor.graph.group_depth(), 1);
    assert!(editor.graph.quick_add.is_some());
    plain(&mut editor, KeyCode::Escape);
    assert!(editor.graph.quick_add.is_none());
    assert_eq!(editor.graph.group_depth(), 1);

    // The number box.
    editor.graph.open_knob_entry(2, KnobId::Cutoff);
    plain(&mut editor, KeyCode::Tab);
    assert_eq!(editor.graph.group_depth(), 1);
    assert!(editor.graph.knob_entry.is_some());
    plain(&mut editor, KeyCode::Escape);
    assert!(editor.graph.knob_entry.is_none());
    assert_eq!(editor.graph.group_depth(), 1);
}

#[test]
fn a_double_click_on_a_group_header_opens_it() {
    let mut graph = grouped();
    graph.clear_selection();
    let header = graph.nodes[GROUP].position;
    let at = Point::new(header.x + 90.0, header.y + 14.0);
    let mut editor = canvas(&mut graph);
    click(&mut editor, at);
    assert_eq!(editor.graph.group_depth(), 0);
    assert!(changed(press(&mut editor, at)));
    assert_eq!(editor.graph.group_depth(), 1);
}

#[test]
fn a_single_click_on_a_group_header_only_selects_it() {
    let mut graph = grouped();
    graph.clear_selection();
    let header = graph.nodes[GROUP].position;
    let at = Point::new(header.x + 90.0, header.y + 14.0);
    let mut editor = canvas(&mut graph);
    click(&mut editor, at);
    assert_eq!(editor.graph.group_depth(), 0);
    assert_eq!(editor.graph.selection(), vec![GROUP]);
}

#[test]
fn the_open_glyph_in_the_header_opens_the_group() {
    let mut graph = grouped();
    let glyph = open_glyph_bounds(&graph.nodes[GROUP]).expect("a group has a glyph");
    assert!(open_glyph_bounds(&graph.nodes[OSC]).is_none());
    let middle = Point::new(glyph.x + glyph.width / 2.0, glyph.y + glyph.height / 2.0);
    let mut editor = canvas(&mut graph);
    click(&mut editor, middle);
    assert_eq!(editor.graph.group_depth(), 1);
}

// The pane.

#[test]
fn the_pane_fills_the_editor_between_the_pill_and_the_strip() {
    let pane = pane_bounds(bounds());
    let area = bounds();
    // About four to five percent of the width on each side.
    assert!(pane.x >= 16.0 && pane.x <= area.width * 0.06);
    assert!((pane.x + pane.width - (area.width - pane.x)).abs() < 0.01);
    assert!(pane.y >= pill_bounds(area).y + pill_bounds(area).height);
    assert!(pane.y + pane.height <= piano::strip_bounds(area).y);
    assert!(pane.width > area.width * 0.85 && pane.height > area.height * 0.7);
    // It follows a smaller editor, and never goes negative.
    let small = pane_bounds(Rectangle {
        width: 300.0,
        height: 160.0,
        ..area
    });
    assert!(small.width >= 0.0 && small.height >= 0.0);
}

#[test]
fn the_title_shows_the_breadcrumb_and_a_close_button() {
    let pane = pane_bounds(bounds());
    let layout = title_layout(pane, &["E Piano".to_string(), "Reverb Tail".to_string()]);
    assert_eq!(layout.prefix, "Main > E Piano > ");
    assert_eq!(layout.name, "Reverb Tail");
    assert!(title_bounds(pane).contains(Point::new(
        layout.name_area.x + 2.0,
        layout.name_area.y + 2.0
    )));
    let close = close_bounds(pane);
    assert!(title_bounds(pane).contains(Point::new(close.x + 1.0, close.y + 1.0)));
    assert!(layout.name_area.x + layout.name_area.width <= close.x);
    // A long path drops its outer names.
    let deep: Vec<String> = (0..12).map(|n| format!("Level number {n}")).collect();
    let layout = title_layout(pane, &deep);
    assert!(layout.prefix.starts_with("... > "));
    assert!(layout.name_area.x + layout.name_area.width <= close.x);
}

#[test]
fn the_close_button_closes_the_group() {
    let mut graph = opened();
    let close = close_bounds(pane_bounds(bounds()));
    let middle = Point::new(close.x + close.width / 2.0, close.y + close.height / 2.0);
    let mut editor = canvas(&mut graph);
    assert!(changed(click(&mut editor, middle)));
    assert_eq!(editor.graph.group_depth(), 0);
}

#[test]
fn a_click_on_the_margin_closes_the_group() {
    let mut graph = opened();
    let mut editor = canvas(&mut graph);
    assert!(pane_bounds(bounds()).x > 8.0);
    click(&mut editor, Point::new(5.0, 300.0));
    assert_eq!(editor.graph.group_depth(), 0);
}

#[test]
fn a_drag_that_starts_on_the_margin_does_not_close_the_group() {
    let mut graph = opened();
    let pane = pane_bounds(bounds());
    let mut editor = canvas(&mut graph);
    // Out past the slop and back onto the pane.
    drag(
        &mut editor,
        Point::new(5.0, 300.0),
        Point::new(pane.x + 100.0, 300.0),
    );
    assert_eq!(editor.graph.group_depth(), 1);
    // Along the margin, past the slop, and let go there.
    drag(
        &mut editor,
        Point::new(5.0, 300.0),
        Point::new(5.0, 300.0 + CLICK_SLOP * 3.0),
    );
    assert_eq!(editor.graph.group_depth(), 1);
    // A click that wobbles inside the slop is still a click.
    drag(
        &mut editor,
        Point::new(5.0, 300.0),
        Point::new(5.0, 300.0 + CLICK_SLOP / 2.0),
    );
    assert_eq!(editor.graph.group_depth(), 0);
}

#[test]
fn a_click_on_the_pane_or_the_pill_leaves_the_group_open() {
    let mut graph = opened();
    let pane = pane_bounds(bounds());
    let mut editor = canvas(&mut graph);
    // Empty canvas in the pane.
    click(
        &mut editor,
        Point::new(pane.x + 5.0, pane.y + pane.height - 5.0),
    );
    assert_eq!(editor.graph.group_depth(), 1);
    // The title strip.
    click(
        &mut editor,
        Point::new(pane.x + pane.width / 2.0, pane.y + 5.0),
    );
    assert_eq!(editor.graph.group_depth(), 1);
    // The pill adds a node inside the group, and the group stays open.
    let before = editor.graph.nodes.len();
    let segment = pill_segment(bounds(), 0);
    click(
        &mut editor,
        Point::new(
            segment.x + segment.width / 2.0,
            segment.y + segment.height / 2.0,
        ),
    );
    assert_eq!(editor.graph.group_depth(), 1);
    assert_eq!(editor.graph.nodes.len(), before + 1);
}

#[test]
fn a_right_click_on_the_margin_opens_nothing() {
    let mut graph = opened();
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, Point::new(5.0, 300.0));
    assert!(editor.graph.menu.is_none());
    assert_eq!(editor.graph.group_depth(), 1);
}

#[test]
fn the_wheel_works_inside_the_pane_and_not_on_the_margin() {
    let mut graph = opened();
    let pane = pane_bounds(bounds());
    let mut editor = canvas(&mut graph);
    let before = editor.graph.view;
    let scroll = mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels { x: 0.0, y: 20.0 },
    };
    mouse_at(&mut editor, scroll, Point::new(5.0, 300.0));
    assert_eq!(editor.graph.view, before);
    mouse_at(&mut editor, scroll, Point::new(pane.x + 50.0, 300.0));
    assert_ne!(editor.graph.view, before);
    // A pinch zooms.
    let pinch = mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines {
            x: f32::NAN,
            y: 0.25,
        },
    };
    let zoom = editor.graph.view.zoom;
    mouse_at(&mut editor, pinch, Point::new(pane.x + 50.0, 300.0));
    assert!(editor.graph.view.zoom > zoom);
}

#[test]
fn opening_fits_the_graph_to_the_pane_of_the_real_editor() {
    let mut graph = grouped();
    graph.canvas_size = Size::new(640.0, 460.0);
    assert!(graph.open_group(GROUP));
    let room = inner_bounds(pane_bounds(canvas_rect(graph.canvas_size)));
    for node in &graph.nodes {
        let at = node.bounds();
        let corner = shown(&graph, Point::new(at.x, at.y));
        let far = shown(&graph, Point::new(at.x + at.width, at.y + at.height));
        assert!(
            room.contains(corner) && room.contains(far),
            "{:?}",
            node.kind
        );
    }
}

#[test]
fn a_resize_while_a_group_is_open_fits_the_graph_again() {
    let mut graph = opened();
    let before = graph.view;
    let mut editor = canvas(&mut graph);
    let wide = Rectangle {
        width: 1400.0,
        height: 900.0,
        ..bounds()
    };
    editor.update(
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(300.0, 300.0),
        }),
        wide,
        Cursor::Available(Point::new(300.0, 300.0)),
    );
    assert_eq!(editor.graph.canvas_size, Size::new(1400.0, 900.0));
    assert_ne!(editor.graph.view, before);
}

// Pins.

#[test]
fn the_edge_nodes_show_one_empty_pin_below_the_real_ones() {
    let graph = opened();
    let input = &graph.nodes[0];
    assert_eq!(input.output_ports().last(), Some(&Port::NewPin));
    assert_eq!(input.output_ports().len(), input.outputs.len() + 1);
    let output = &graph.nodes[1];
    assert_eq!(output.input_ports().last(), Some(&Port::NewPin));
    assert_eq!(output.input_ports().len(), output.inputs.len() + 1);
    // The group node has no empty pin, since its pins come from inside.
    let mut graph = grouped();
    assert!(!graph.nodes[GROUP].input_ports().contains(&Port::NewPin));
    graph.close_group();
}

#[test]
fn dropping_a_wire_on_the_empty_output_pin_makes_a_pin() {
    let mut graph = opened();
    let from = out_pin(&graph, 2, Port::Audio);
    let to = in_pin(&graph, 1, Port::NewPin);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    let edge = &graph.nodes[1];
    assert_eq!(edge.inputs.len(), 2);
    assert_eq!(edge.inputs[1].signal, Signal::Audio);
    // The pin takes the name of the wire's port, and a second "Audio" gets a number.
    assert_eq!(edge.inputs[1].name, "Audio 2");
    assert!(graph
        .connections
        .contains(&wire(2, Port::Audio, 1, audio_pin(1))));
    assert!(graph.connections.iter().all(|c| c.is_valid(&graph.nodes)));
    // The group node outside has the pin too.
    assert!(graph.close_group());
    assert_eq!(graph.nodes[GROUP].outputs.len(), 2);
}

#[test]
fn a_pin_made_by_a_drop_has_the_signal_of_the_wire() {
    let mut graph = opened();
    // The cutoff pin carries control, and it can run straight across to an output.
    let from = out_pin(&graph, 0, control_pin(1));
    let to = in_pin(&graph, 1, Port::NewPin);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    assert_eq!(
        graph.nodes[1].inputs.last().map(|p| p.signal),
        Some(Signal::Control)
    );
    assert!(graph
        .connections
        .contains(&wire(0, control_pin(1), 1, control_pin(1))));
}

#[test]
fn dragging_back_from_an_input_onto_the_empty_input_pin_makes_a_pin() {
    let mut graph = opened();
    let mix = Port::Knob(KnobId::Mix);
    let from = in_pin(&graph, 3, mix);
    let to = out_pin(&graph, 0, Port::NewPin);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    let edge = &graph.nodes[0];
    assert_eq!(edge.outputs.len(), 3);
    assert_eq!(edge.outputs[2].signal, Signal::Control);
    assert!(graph.connections.contains(&wire(0, control_pin(2), 3, mix)));
    assert!(graph.close_group());
    assert_eq!(graph.nodes[GROUP].inputs.len(), 3);
}

#[test]
fn a_wire_dragged_out_of_the_empty_input_pin_makes_a_pin() {
    let mut graph = opened();
    let mix = Port::Knob(KnobId::Mix);
    let from = out_pin(&graph, 0, Port::NewPin);
    let to = in_pin(&graph, 3, mix);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    assert_eq!(graph.nodes[0].outputs.len(), 3);
    assert!(graph.connections.contains(&wire(0, control_pin(2), 3, mix)));
}

#[test]
fn dragging_back_from_the_empty_output_pin_makes_a_pin() {
    let mut graph = opened();
    let from = in_pin(&graph, 1, Port::NewPin);
    let to = out_pin(&graph, 2, Port::Audio);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    assert_eq!(graph.nodes[1].inputs.len(), 2);
    assert!(graph
        .connections
        .contains(&wire(2, Port::Audio, 1, audio_pin(1))));
}

#[test]
fn a_drop_that_fits_nothing_makes_no_pin() {
    let mut graph = opened();
    // From the empty input pin onto an empty spot, and onto the other empty pin.
    let from = out_pin(&graph, 0, Port::NewPin);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, Point::new(from.x + 80.0, from.y + 150.0));
    let to = in_pin(editor.graph, 1, Port::NewPin);
    drag(&mut editor, from, to);
    assert_eq!(graph.nodes[0].outputs.len(), 2);
    assert_eq!(graph.nodes[1].inputs.len(), 1);
    assert!(matches!(graph.interaction, Interaction::Idle));
    // A drop of the empty pin's wire on the empty pin directly is refused by the model too.
    assert!(!graph.drop_wire(0, Port::NewPin, 1, Port::NewPin));
}

#[test]
fn a_drop_from_a_node_outside_the_open_group_is_refused() {
    // At the root there is no group open, so the model has no pin to add.
    let mut graph = grouped();
    assert!(!graph.drop_wire(GROUP, audio_pin(0), OUTPUT, Port::NewPin));
    assert!(graph.close_group() || graph.group_depth() == 0);
}

#[test]
fn a_real_input_pin_feeds_several_inputs_and_an_output_pin_takes_one_wire() {
    let mut graph = opened();
    // Pin 0 feeds the filter already. It feeds the reverb's audio input as well.
    let from = out_pin(&graph, 0, audio_pin(0));
    let to = in_pin(&graph, 3, Port::Audio);
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    let fed = graph
        .connections
        .iter()
        .filter(|c| c.from == 0 && c.out == audio_pin(0))
        .count();
    assert_eq!(fed, 2);
    // The output pin has the reverb's wire. Another wire to it replaces that one.
    let from = out_pin(&graph, 0, audio_pin(0));
    let to = in_pin(&graph, 1, audio_pin(0));
    let mut editor = canvas(&mut graph);
    drag(&mut editor, from, to);
    let into = graph
        .connections
        .iter()
        .filter(|c| c.to == 1 && c.input == audio_pin(0))
        .count();
    assert_eq!(into, 1);
    assert!(graph.connections.iter().all(|c| c.is_valid(&graph.nodes)));
}

// Renaming.

#[test]
fn a_pin_menu_renames_a_pin_on_both_sides() {
    let mut graph = opened();
    let at = out_pin(&graph, 0, audio_pin(0));
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    let menu = editor.graph.menu.expect("a pin menu");
    assert_eq!(menu.target, MenuTarget::Pin(PinSide::Input, 0));
    assert!(menu_has(editor.graph, MenuItem::Rename));
    assert!(menu_has(editor.graph, MenuItem::RemovePin));
    choose(&mut editor, MenuItem::Rename);
    let rename = editor.graph.rename.as_ref().expect("the box opens");
    assert_eq!(rename.text, "Audio");
    assert!(rename.fresh);
    type_text(&mut editor, "Dry");
    // The first key replaced the whole name.
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.as_str()),
        Some("Dry")
    );
    assert!(changed(plain(&mut editor, KeyCode::Enter)));
    assert!(editor.graph.rename.is_none());
    assert_eq!(pin_names(&graph.nodes[0].outputs)[0], "Dry");
    graph.close_group();
    assert_eq!(pin_names(&graph.nodes[GROUP].inputs)[0], "Dry");
}

#[test]
fn escape_cancels_a_rename_and_an_empty_name_keeps_the_old_one() {
    let mut graph = opened();
    let at = in_pin(&graph, 1, audio_pin(0));
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Rename);
    type_text(&mut editor, "Wet");
    plain(&mut editor, KeyCode::Escape);
    assert!(editor.graph.rename.is_none());
    assert_eq!(pin_names(&graph.nodes[1].inputs)[0], "Audio");
    // Back to empty and Enter.
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Rename);
    plain(&mut editor, KeyCode::Backspace);
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.as_str()),
        Some("")
    );
    assert!(!changed(plain(&mut editor, KeyCode::Enter)));
    assert_eq!(pin_names(&graph.nodes[1].inputs)[0], "Audio");
    // The box also takes backspace after typing, and a press outside keeps the name.
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Rename);
    type_text(&mut editor, "Wetx");
    plain(&mut editor, KeyCode::Backspace);
    assert!(changed(click(&mut editor, Point::new(5.0, 400.0))));
    assert_eq!(pin_names(&graph.nodes[1].inputs)[0], "Wet");
    assert_eq!(graph.group_depth(), 1);
}

#[test]
fn a_pin_renamed_from_the_group_face_is_renamed_inside_too() {
    let mut graph = grouped();
    graph.clear_selection();
    let at = shown(
        &graph,
        graph.nodes[GROUP]
            .input_point(audio_pin(0))
            .expect("the face pin"),
    );
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    let menu = editor.graph.menu.expect("a pin menu");
    assert_eq!(menu.target, MenuTarget::Pin(PinSide::Input, 0));
    choose(&mut editor, MenuItem::Rename);
    type_text(&mut editor, "Source");
    plain(&mut editor, KeyCode::Enter);
    // The view and the selection are as they were, and the group is still shut.
    assert_eq!(editor.graph.group_depth(), 0);
    assert_eq!(pin_names(&graph.nodes[GROUP].inputs)[0], "Source");
    assert!(graph.open_group(GROUP));
    assert_eq!(pin_names(&graph.nodes[0].outputs)[0], "Source");
}

#[test]
fn removing_a_pin_from_the_face_drops_its_wires_on_both_sides() {
    let mut graph = grouped();
    graph.clear_selection();
    let at = shown(
        &graph,
        graph.nodes[GROUP]
            .output_point(audio_pin(0))
            .expect("the face pin"),
    );
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    assert_eq!(
        editor.graph.menu.map(|m| m.target),
        Some(MenuTarget::Pin(PinSide::Output, 0))
    );
    choose(&mut editor, MenuItem::RemovePin);
    assert!(graph.nodes[GROUP].outputs.is_empty());
    assert!(graph.connections.iter().all(|c| c.from != GROUP));
    assert!(graph.open_group(GROUP));
    assert!(graph.nodes[1].inputs.is_empty());
    assert!(graph.connections.iter().all(|c| c.to != 1));
}

#[test]
fn removing_a_pin_from_inside_renumbers_the_rest() {
    let mut graph = opened();
    let at = out_pin(&graph, 0, audio_pin(0));
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::RemovePin);
    assert_eq!(graph.nodes[0].outputs.len(), 1);
    assert_eq!(graph.nodes[0].outputs[0].name, "Cutoff");
    assert!(graph
        .connections
        .contains(&wire(0, control_pin(0), 2, Port::Knob(KnobId::Cutoff))));
}

#[test]
fn a_group_is_renamed_from_its_menu() {
    let mut graph = grouped();
    graph.clear_selection();
    let header = graph.nodes[GROUP].position;
    let at = Point::new(header.x + 90.0, header.y + 14.0);
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Rename);
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.target.clone()),
        Some(RenameTarget::Group { node: GROUP })
    );
    type_text(&mut editor, "E Piano");
    plain(&mut editor, KeyCode::Enter);
    assert_eq!(graph.nodes[GROUP].label(), "E Piano");
}

#[test]
fn the_open_group_is_renamed_from_its_title() {
    let mut graph = opened();
    let layout = title_layout(pane_bounds(bounds()), &graph.group_path_names());
    let at = Point::new(
        layout.name_area.x + layout.name_area.width / 2.0,
        layout.name_area.y + layout.name_area.height / 2.0,
    );
    let mut editor = canvas(&mut graph);
    click(&mut editor, at);
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.clone()),
        Some("Group".to_string())
    );
    type_text(&mut editor, "Reverb Tail");
    plain(&mut editor, KeyCode::Enter);
    assert_eq!(graph.group_path_names(), vec!["Reverb Tail".to_string()]);
    assert!(graph.close_group());
    assert_eq!(graph.nodes[GROUP].label(), "Reverb Tail");
}

#[test]
fn a_tab_typed_into_the_rename_box_does_not_toggle_a_group() {
    let mut graph = opened();
    assert!(graph.open_rename(RenameTarget::OpenGroup, Point::new(300.0, 300.0)));
    let mut editor = canvas(&mut graph);
    // The key, and the character the system sends with it.
    assert!(!matches!(
        plain(&mut editor, KeyCode::Tab),
        Some(Message::Note(_))
    ));
    type_text(&mut editor, "\t");
    assert_eq!(editor.graph.group_depth(), 1);
    let rename = editor.graph.rename.as_ref().expect("the box stays open");
    assert_eq!(rename.text, "Group");
    assert!(rename.fresh);
    // Typing still works afterward, and Escape closes the box before the group.
    type_text(&mut editor, "A\tB");
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.as_str()),
        Some("AB")
    );
    plain(&mut editor, KeyCode::Escape);
    assert!(editor.graph.rename.is_none());
    assert_eq!(editor.graph.group_depth(), 1);
}

#[test]
fn the_rename_box_takes_letters_that_are_piano_keys() {
    let mut graph = opened();
    assert!(graph.open_rename(RenameTarget::OpenGroup, Point::new(300.0, 300.0)));
    let mut editor = canvas(&mut graph);
    let message = plain(&mut editor, KeyCode::A);
    assert!(!matches!(message, Some(Message::Note(_))));
    // The box is as long as a name may be and no longer.
    type_text(&mut editor, &"x".repeat(40));
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.chars().count()),
        Some(24)
    );
}

// The macro names.

fn macro_graph() -> Graph {
    Graph::with(
        vec![
            node(NodeKind::Macro, 100.0),
            node(NodeKind::Oscillator, 400.0),
            node(NodeKind::Output, 700.0),
        ],
        Vec::new(),
    )
}

#[test]
fn a_macro_row_is_renamed_from_its_menu_and_resets_when_emptied() {
    let mut graph = macro_graph();
    let row = 2;
    let place = graph.nodes[0].row_places()[row].y;
    let at = Point::new(
        graph.nodes[0].position.x + 60.0,
        graph.nodes[0].position.y + place,
    );
    let default = KnobId::MACROS[row].name();
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    assert_eq!(
        editor.graph.menu.map(|m| m.target),
        Some(MenuTarget::Macro(row))
    );
    choose(&mut editor, MenuItem::Rename);
    assert_eq!(
        editor.graph.rename.as_ref().map(|r| r.text.as_str()),
        Some(default)
    );
    type_text(&mut editor, "Cutoff");
    assert!(changed(plain(&mut editor, KeyCode::Enter)));
    assert_eq!(graph.nodes[0].knob_name(KnobId::MACROS[row]), "Cutoff");
    // The other macros keep their own names.
    assert_eq!(
        graph.nodes[0].knob_name(KnobId::MACROS[0]),
        KnobId::MACROS[0].name()
    );
    // Emptying the name puts the default back.
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Rename);
    plain(&mut editor, KeyCode::Backspace);
    assert!(changed(plain(&mut editor, KeyCode::Enter)));
    assert_eq!(graph.nodes[0].knob_name(KnobId::MACROS[row]), default);
    assert!(graph.nodes[0].names.is_empty());
}

#[test]
fn a_macro_menu_still_deletes_the_node() {
    let mut graph = macro_graph();
    let place = graph.nodes[0].row_places()[0].y;
    let at = Point::new(
        graph.nodes[0].position.x + 60.0,
        graph.nodes[0].position.y + place,
    );
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, at);
    choose(&mut editor, MenuItem::Delete);
    assert!(graph.nodes.iter().all(|n| n.kind != NodeKind::Macro));
}

#[test]
fn macro_names_survive_saving_loading_and_copying() {
    let mut graph = macro_graph();
    assert!(graph.nodes[0].set_macro_name(1, "  Drive "));
    assert!(graph.nodes[0].set_macro_name(3, "Space"));
    assert!(!graph.nodes[1].set_macro_name(0, "Nope"));
    assert!(!graph.nodes[0].set_macro_name(MACRO_COUNT, "Nope"));
    // Through the project save, as JSON.
    let json = serde_json::to_string(&graph.to_saved()).expect("writes");
    let saved = serde_json::from_str(&json).expect("reads");
    let loaded = Graph::from_saved(&saved);
    let macros = &loaded.nodes[0];
    assert_eq!(macros.knob_name(KnobId::MACROS[1]), "Drive");
    assert_eq!(macros.knob_name(KnobId::MACROS[3]), "Space");
    assert_eq!(
        macros.knob_name(KnobId::MACROS[0]),
        KnobId::MACROS[0].name()
    );
    // Through a copy of the node.
    let copy = graph.nodes[0].duplicate(Point::new(0.0, 0.0));
    assert_eq!(copy.knob_name(KnobId::MACROS[1]), "Drive");
    // Inside a group, which saves and copies its nodes the same way.
    let mut inner = macro_graph();
    inner.nodes[0].set_macro_name(0, "Mod");
    let again = Graph::from_saved(&inner.to_saved());
    assert_eq!(again.nodes[0].knob_name(KnobId::MACROS[0]), "Mod");
}

#[test]
fn a_graph_with_no_macro_names_saves_no_names() {
    let graph = macro_graph();
    let json = serde_json::to_string(&graph.to_saved()).expect("writes");
    assert!(!json.contains("\"names\""));
    // And a save from before the names loads with the default ones.
    let loaded = Graph::from_saved(&serde_json::from_str(&json).expect("reads"));
    assert!(loaded.nodes[0].names.is_empty());
}

#[test]
fn macro_names_go_into_a_preset() {
    let mut graph = macro_graph();
    graph.nodes[0].set_macro_name(2, "Wobble");
    let preset = crate::presets::Preset {
        name: "Named".to_string(),
        category: String::new(),
        graph: graph.to_saved(),
        macros: [0.0; MACRO_COUNT],
    };
    let json = serde_json::to_string(&preset).expect("writes");
    let back: crate::presets::Preset = serde_json::from_str(&json).expect("reads");
    let loaded = Graph::from_saved(&back.graph);
    assert_eq!(loaded.nodes[0].knob_name(KnobId::MACROS[2]), "Wobble");
}

#[test]
fn a_group_keeps_a_macro_name_inside_a_save_made_while_it_is_open() {
    // The macro node is at the root, and a save made inside a group still has the name.
    let mut graph = grouped();
    let macros = graph.add_node(NodeKind::Macro, Point::new(10.0, 300.0));
    graph.nodes[macros].set_macro_name(0, "Mod");
    assert!(graph.open_group(GROUP));
    let loaded = Graph::from_saved(&graph.to_saved());
    let found = loaded
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Macro)
        .expect("the macro node");
    assert_eq!(found.knob_name(KnobId::MACROS[0]), "Mod");
}
