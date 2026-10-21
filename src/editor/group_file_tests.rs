//! Tests for saving a group to a file and loading it into other patches. Every test uses a folder
//! of its own under the temp directory, and none opens a real dialog.

use super::canvas::NodeEditor;
use super::graph::{Graph, Interaction, DEFAULT_OUTPUT_X};
use super::group::PinSide;
use super::group_ui::{pane_bounds, save_bounds};
use super::menus::{MenuItem, QuickPick};
use super::saved::{SavedGroupFile, SavedNode};
use crate::groups::{self, GroupEntry};
use crate::nodes::connection::Connection;
use crate::nodes::knobs::KnobId;
use crate::nodes::{FlodeNode, NodeKind, Port, Signal};
use crate::patch::control::ControlLevels;
use nih_plug_iced::keyboard::{self, KeyCode, Modifiers};
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
use nih_plug_iced::{Point, Rectangle};
use std::path::PathBuf;

const INPUT: usize = 0;
const OSC: usize = 1;
const FILTER: usize = 2;
const REVERB: usize = 3;
const OUTPUT: usize = 4;
const LFO: usize = 5;
const GROUP: usize = 2;

/// A folder for one test, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("flode-group-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

fn entry(name: &str) -> GroupEntry {
    GroupEntry {
        category: String::new(),
        name: name.to_string(),
    }
}

fn canvas(graph: &mut Graph) -> NodeEditor<'_> {
    NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
}

fn mouse_at(editor: &mut NodeEditor, event: mouse::Event, at: Point) {
    editor.update(Event::Mouse(event), bounds(), Cursor::Available(at));
}

fn click(editor: &mut NodeEditor, at: Point) {
    mouse_at(editor, mouse::Event::ButtonPressed(mouse::Button::Left), at);
    mouse_at(
        editor,
        mouse::Event::ButtonReleased(mouse::Button::Left),
        at,
    );
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

fn type_text(editor: &mut NodeEditor, text: &str) {
    for c in text.chars() {
        editor.update(
            Event::Keyboard(keyboard::Event::CharacterReceived(c)),
            bounds(),
            Cursor::Unavailable,
        );
    }
}

fn press_key(editor: &mut NodeEditor, key_code: KeyCode) {
    for event in [
        keyboard::Event::KeyPressed {
            key_code,
            modifiers: Modifiers::default(),
        },
        keyboard::Event::KeyReleased {
            key_code,
            modifiers: Modifiers::default(),
        },
    ] {
        editor.update(Event::Keyboard(event), bounds(), Cursor::Unavailable);
    }
}

/// The middle of row `row` of the open menu.
fn menu_row(graph: &Graph, row: usize) -> Point {
    let menu = graph.menu.expect("a menu is open");
    Point::new(
        menu.position.x + 10.0,
        menu.position.y + (row as f32 + 0.5) * crate::editor::style::MENU_ITEM_HEIGHT,
    )
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

/// Sends the open dialog's answer, as the worker thread would, and polls for it.
fn answer_dialog(graph: &mut Graph, path: Option<PathBuf>) {
    let sender = graph.save_dialog_sender.clone().expect("a dialog is open");
    sender.send(path).unwrap();
    graph.poll_save_dialog();
}

fn status(graph: &Graph) -> String {
    graph
        .status
        .as_ref()
        .map(|(text, _)| text.clone())
        .unwrap_or_default()
}

/// The usual chain with the filter and the reverb in a group, which has an audio pin in and out
/// and a control pin on the cutoff. The cutoff pin is free, set to 0.8 on the face and named
/// "Bright", and the audio pin in is named "Dry in". The reverb sits in a group of its own inside.
fn source() -> Graph {
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
    graph.nodes[FILTER].set_value(KnobId::Cutoff, 0.3);
    graph.refresh_slots();
    assert_eq!(graph.create_group(&[FILTER, REVERB]), Some(GROUP));
    graph
        .connections
        .retain(|c| c.to != GROUP || c.input != Port::Pin(Signal::Control, 1));
    graph.nodes[GROUP].set_value(crate::nodes::group::GROUP_KNOBS[0], 0.8);
    assert!(graph.rename_pin_of(GROUP, PinSide::Input, 0, "Dry in"));
    assert!(graph.rename_pin_of(GROUP, PinSide::Input, 1, "Bright"));
    // Inside, the edge nodes are 0 and 1, the filter is 2 and the reverb is 3.
    assert!(graph.open_group(GROUP));
    assert!(graph.create_group(&[3]).is_some());
    assert!(graph.close_group());
    graph
}

/// An oscillator into an output, with the saved groups in `dir`.
fn bare(dir: &TempDir) -> Graph {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Input, 0.0),
            node(NodeKind::Oscillator, 200.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![wire(0, Port::Notes, 1, Port::Notes)],
    );
    graph.refresh_slots();
    graph.group_dir = Some(dir.0.clone());
    graph
}

/// Saves group node `GROUP` of `graph` under `name` in `dir`, the way the menu does.
fn save_as(graph: &mut Graph, dir: &TempDir, name: &str) {
    graph.group_dir = Some(dir.0.clone());
    graph.start_group_save_dialog(GROUP);
    answer_dialog(graph, Some(dir.0.join(format!("{name}.flodegroup"))));
}

fn node_json(node: &FlodeNode) -> serde_json::Value {
    serde_json::to_value(SavedNode::new(node)).expect("a node writes")
}

#[test]
fn a_group_round_trips_with_nesting_pins_and_face_values() {
    let dir = TempDir::new("round-trip");
    let mut graph = source();
    save_as(&mut graph, &dir, "Group");
    assert!(dir.0.join("Group.flodegroup").is_file());
    assert_eq!(status(&graph), "Saved group Group");

    let mut target = bare(&dir);
    let index = target
        .add_saved_group(&entry("Group"), Point::new(40.0, 50.0))
        .expect("the file loads");
    let loaded = &target.nodes[index];
    let original = &graph.nodes[GROUP];
    assert_eq!(loaded.kind, NodeKind::Group);
    assert_eq!(loaded.position, Point::new(40.0, 50.0));
    // The pins keep their names and signals, and the face knob keeps its value.
    let names: Vec<&str> = loaded.inputs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Dry in", "Bright"]);
    assert!((loaded.inputs[1].value - 0.8).abs() < 1e-4);
    assert_eq!(loaded.inputs[1].knob, Some(KnobId::Cutoff));
    // The graph inside is the same, nested group included.
    let inner = loaded.group.as_ref().expect("a group");
    assert_eq!(
        inner
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Group)
            .count(),
        1
    );
    // A patch load fills in a few defaults, such as an empty dropdown row. The file load does the
    // same, so the comparison is with a node that went through a patch load.
    let mut expected = node_json(&SavedNode::new(original).into_node());
    let mut actual = node_json(loaded);
    for json in [&mut expected, &mut actual] {
        json["x"] = 0.into();
        json["y"] = 0.into();
    }
    assert_eq!(actual, expected);
}

#[test]
fn a_loaded_copy_compiles_to_the_same_patch() {
    let dir = TempDir::new("compile");
    let mut graph = source();
    save_as(&mut graph, &dir, "Group");

    // The same wiring around a copy made in memory and around the one loaded from the file.
    let wire_up = |graph: &mut Graph, at: usize| {
        graph.connect(1, Port::Audio, at, audio_pin(0));
        graph.connect(at, audio_pin(0), 2, Port::Slot(0));
        graph.refresh_slots();
    };
    let mut by_hand = bare(&dir);
    by_hand
        .nodes
        .push(graph.nodes[GROUP].duplicate(Point::new(0.0, 0.0)));
    wire_up(&mut by_hand, 3);

    let mut from_file = bare(&dir);
    canvas(&mut from_file).place_group(&entry("Group"), Point::new(300.0, 300.0), None, None);
    assert!(matches!(
        from_file.interaction,
        Interaction::Placing { node: 3, .. }
    ));
    wire_up(&mut from_file, 3);

    assert!(by_hand.has_groups() && from_file.has_groups());
    assert_eq!(
        format!("{:?}", from_file.patch()),
        format!("{:?}", by_hand.patch())
    );
    // The patch holds the filter and the reverb from inside the group.
    let (nodes, links) = from_file.root_snapshot();
    let flat = crate::patch::flatten::flatten(&nodes, &links);
    for kind in [NodeKind::Filter, NodeKind::Reverb] {
        assert!(flat.nodes.iter().any(|n| n.kind == kind), "{kind:?}");
    }
}

#[test]
fn the_file_name_is_the_name_and_the_node_keeps_its_own() {
    let dir = TempDir::new("name");
    let mut graph = source();
    save_as(&mut graph, &dir, "Warm pad");
    assert_eq!(status(&graph), "Saved group Warm pad");
    assert_eq!(graph.nodes[GROUP].label(), "Group");
    let node = groups::load_in(&dir.0, &entry("Warm pad")).expect("loads");
    assert_eq!(node.label(), "Warm pad");
    // An extension left off the typed name is added.
    graph.start_group_save_dialog(GROUP);
    answer_dialog(&mut graph, Some(dir.0.join("Bare")));
    assert!(dir.0.join("Bare.flodegroup").is_file());
}

#[test]
fn a_canceled_dialog_writes_nothing() {
    let dir = TempDir::new("cancel");
    let mut graph = source();
    graph.group_dir = Some(dir.0.clone());
    graph.start_group_save_dialog(GROUP);
    assert_eq!(graph.save_dialog_requests, 1);
    answer_dialog(&mut graph, None);
    assert!(graph.save_dialog.is_none() && graph.group_save.is_none());
    assert!(groups::list_in(&dir.0).is_empty());
}

#[test]
fn the_menu_entry_opens_the_dialog() {
    let dir = TempDir::new("menu");
    let mut graph = source();
    graph.group_dir = Some(dir.0.clone());
    graph.clear_selection();
    let header = graph.nodes[GROUP].position;
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, Point::new(header.x + 90.0, header.y + 14.0));
    choose(&mut editor, MenuItem::SaveGroup);
    assert_eq!(graph.save_dialog_requests, 1);
    assert!(graph.group_save.is_some() && graph.pending_save.is_none());
    answer_dialog(&mut graph, Some(dir.0.join("Menu.flodegroup")));
    assert!(dir.0.join("Menu.flodegroup").is_file());
    // A preset dialog is not started, so the editor has no preset to save.
    assert!(graph.pending_save.is_none());
}

#[test]
fn the_group_is_saved_as_it_was_when_the_menu_entry_was_chosen() {
    let dir = TempDir::new("snapshot");
    let mut graph = source();
    graph.group_dir = Some(dir.0.clone());
    graph.start_group_save_dialog(GROUP);
    assert!(graph.rename_group_node(GROUP, "Renamed"));
    graph.nodes[GROUP].set_value(crate::nodes::group::GROUP_KNOBS[0], 0.1);
    answer_dialog(&mut graph, Some(dir.0.join("Then.flodegroup")));
    let node = groups::load_in(&dir.0, &entry("Then")).expect("loads");
    assert!((node.inputs[1].value - 0.8).abs() < 1e-4);
}

#[test]
fn a_nested_group_saves_from_inside_its_parent() {
    let dir = TempDir::new("nested");
    let mut graph = source();
    graph.group_dir = Some(dir.0.clone());
    assert!(graph.open_group(GROUP));
    let nested = graph
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Group)
        .expect("the nested group");
    graph.clear_selection();
    let header = graph.view.to_screen(Point::new(
        graph.nodes[nested].position.x + 90.0,
        graph.nodes[nested].position.y + 14.0,
    ));
    let mut editor = canvas(&mut graph);
    right_click(&mut editor, header);
    choose(&mut editor, MenuItem::SaveGroup);
    answer_dialog(&mut graph, Some(dir.0.join("Inner.flodegroup")));
    let node = groups::load_in(&dir.0, &entry("Inner")).expect("loads");
    let inner = node.group.as_ref().expect("a group");
    assert_eq!(
        inner
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Reverb)
            .count(),
        1
    );
    // The group on show is still open, with its graph in place.
    assert_eq!(graph.group_depth(), 1);
}

#[test]
fn the_save_icon_saves_the_open_group_with_its_edits() {
    let dir = TempDir::new("pane");
    let mut graph = source();
    graph.group_dir = Some(dir.0.clone());
    assert!(graph.open_group(GROUP));
    // An edit inside the open group, which the group node does not hold yet.
    let extra = graph.add_node(NodeKind::Lfo, Point::new(0.0, 300.0));
    let pane = pane_bounds(bounds());
    let icon = save_bounds(pane);
    let mut editor = canvas(&mut graph);
    click(
        &mut editor,
        Point::new(icon.x + icon.width / 2.0, icon.y + icon.height / 2.0),
    );
    assert_eq!(graph.save_dialog_requests, 1);
    answer_dialog(&mut graph, Some(dir.0.join("Open.flodegroup")));
    let node = groups::load_in(&dir.0, &entry("Open")).expect("loads");
    let inner = node.group.as_ref().expect("a group");
    assert!(inner.nodes.iter().any(|n| n.kind == NodeKind::Lfo));
    assert!(inner.nodes.iter().any(|n| n.kind == NodeKind::Filter));
    // The graph on show is the one that was open.
    assert_eq!(graph.group_depth(), 1);
    assert_eq!(graph.nodes[extra].kind, NodeKind::Lfo);
}

#[test]
fn saving_with_no_group_folder_says_so() {
    let mut graph = source();
    graph.group_dir = None;
    graph.start_group_save_dialog(GROUP);
    assert_eq!(graph.save_dialog_requests, 0);
    assert_eq!(status(&graph), "Could not save: no group folder");
}

/// Saves two files into `dir` and returns a graph that reads them.
fn with_saved(dir: &TempDir, names: &[&str]) -> Graph {
    let mut graph = source();
    for name in names {
        save_as(&mut graph, dir, name);
    }
    bare(dir)
}

#[test]
fn quick_add_lists_saved_groups_beside_the_kinds() {
    let dir = TempDir::new("list");
    let mut graph = with_saved(&dir, &["Lead", "Reverb pad", "Big reverb"]);
    graph.open_quick_add();
    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "lead");
    let picks = graph.quick_add_picks();
    // A prefix match on a group beats the loose letter matches of the kinds.
    assert_eq!(picks.first(), Some(&QuickPick::Group(1)));

    // The same ranking as the kinds. Reverb is a prefix match, so it comes before the group
    // with the same rank, then the group with a prefix word, then the one that only contains it.
    graph.open_quick_add();
    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "rev");
    let open = graph.quick_add.as_ref().expect("open");
    let name = |pick: &QuickPick| match pick {
        QuickPick::Kind(kind) => kind.title().to_string(),
        QuickPick::Group(index) => open.groups[*index].entry.name.clone(),
    };
    let order: Vec<String> = graph.quick_add_picks().iter().map(name).collect();
    assert_eq!(order[..3], ["Reverb", "Reverb pad", "Big reverb"]);
}

#[test]
fn the_list_is_read_when_the_box_opens() {
    let dir = TempDir::new("fresh");
    let mut graph = bare(&dir);
    graph.open_quick_add();
    assert!(graph.quick_add.as_ref().unwrap().groups.is_empty());
    let mut other = source();
    save_as(&mut other, &dir, "Late");
    graph.open_quick_add();
    assert_eq!(graph.quick_add.as_ref().unwrap().groups.len(), 1);
    graph.open_group_list();
    assert_eq!(graph.group_list, Some(vec![entry("Late")]));
}

#[test]
fn choosing_a_group_places_a_copy_that_follows_the_cursor() {
    let dir = TempDir::new("place");
    let mut graph = with_saved(&dir, &["Lead"]);
    let before = graph.nodes.len();
    graph.open_quick_add();
    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "lead");
    press_key(&mut editor, KeyCode::Enter);
    assert_eq!(graph.nodes.len(), before + 1);
    let placed = graph.nodes.len() - 1;
    assert_eq!(graph.nodes[placed].kind, NodeKind::Group);
    assert_eq!(graph.nodes[placed].label(), "Lead");
    assert!(matches!(
        graph.interaction,
        Interaction::Placing { node, .. } if node == placed
    ));
    assert!(graph.quick_add.is_none());
}

#[test]
fn clicking_a_group_row_places_it_too() {
    let dir = TempDir::new("click");
    let mut graph = with_saved(&dir, &["Lead"]);
    let before = graph.nodes.len();
    graph.open_quick_add();
    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "lead");
    let open = editor.graph.quick_add.clone().expect("open");
    let rows = editor.graph.quick_add_picks().len();
    let area = super::menus::quick_add_bounds(bounds(), open.screen, rows);
    let at = Point::new(
        area.x + 20.0,
        area.y + super::menus::QUICK_ADD_FIELD_HEIGHT + 10.0,
    );
    click(&mut editor, at);
    assert_eq!(graph.nodes.len(), before + 1);
    assert_eq!(graph.nodes[before].kind, NodeKind::Group);
}

#[test]
fn a_loaded_copy_does_not_follow_the_file() {
    let dir = TempDir::new("copy");
    let mut graph = with_saved(&dir, &["Lead"]);
    let index = graph
        .add_saved_group(&entry("Lead"), Point::new(0.0, 0.0))
        .expect("loads");
    let before = node_json(&graph.nodes[index]);
    // The file is saved again as a different group, and then deleted.
    let mut other = Graph::with(vec![node(NodeKind::Oscillator, 0.0)], Vec::new());
    other.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
    let single = other.create_group(&[1]).expect("a group");
    let file = SavedGroupFile::new(&other.nodes[single], "Lead");
    groups::save_to(&dir.0.join("Lead.flodegroup"), &file).unwrap();
    assert_eq!(node_json(&graph.nodes[index]), before);
    // A second load reads the new file, so the two copies differ.
    let second = graph
        .add_saved_group(&entry("Lead"), Point::new(0.0, 0.0))
        .expect("loads");
    assert_ne!(node_json(&graph.nodes[second]), before);
    std::fs::remove_file(dir.0.join("Lead.flodegroup")).unwrap();
    assert_eq!(node_json(&graph.nodes[index]), before);
}

#[test]
fn a_bad_file_is_refused_with_a_message() {
    let dir = TempDir::new("bad");
    let mut graph = bare(&dir);
    std::fs::create_dir_all(&dir.0).unwrap();
    std::fs::write(dir.0.join("Junkfile.flodegroup"), "not json").unwrap();

    // A file that holds an Output node.
    let mut holder = source();
    let data = holder.nodes[GROUP].group.as_mut().unwrap();
    data.nodes
        .push(FlodeNode::new(NodeKind::Output, Point::new(0.0, 0.0)));
    let file = SavedGroupFile::new(&holder.nodes[GROUP], "Outnode");
    groups::save_to(&dir.0.join("Outnode.flodegroup"), &file).unwrap();

    // A file with a wire to a pin that is not there.
    let mut holder = source();
    let data = holder.nodes[GROUP].group.as_mut().unwrap();
    let edge = data.find(NodeKind::GroupInput).unwrap();
    data.links.push(wire(edge, audio_pin(9), 2, Port::Audio));
    let file = SavedGroupFile::new(&holder.nodes[GROUP], "Pinless");
    groups::save_to(&dir.0.join("Pinless.flodegroup"), &file).unwrap();

    // Each is listed, and choosing it adds nothing and says why.
    let before = graph.nodes.len();
    for name in ["Junkfile", "Outnode", "Pinless"] {
        graph.open_quick_add();
        let mut editor = canvas(&mut graph);
        type_text(&mut editor, &name.to_lowercase());
        press_key(&mut editor, KeyCode::Enter);
        assert_eq!(graph.nodes.len(), before, "{name}");
        assert!(
            !matches!(graph.interaction, Interaction::Placing { .. }),
            "{name}"
        );
        let said = status(&graph);
        assert!(
            said.starts_with(&format!("Could not load {name}")),
            "{said}"
        );
    }

    // A file that is gone since the list was read.
    assert!(graph
        .add_saved_group(&entry("Gone"), Point::new(0.0, 0.0))
        .is_none());
    assert!(status(&graph).starts_with("Could not load Gone"));
}

#[test]
fn a_group_loaded_while_another_is_open_goes_inside_it() {
    let dir = TempDir::new("inside");
    let mut graph = source();
    save_as(&mut graph, &dir, "Part");
    assert!(graph.open_group(GROUP));
    let before = graph.nodes.len();
    let outer_before = graph.open_groups[0].nodes.len();
    graph.open_quick_add();
    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "part");
    press_key(&mut editor, KeyCode::Enter);
    assert_eq!(graph.nodes.len(), before + 1);
    assert_eq!(graph.nodes[before].kind, NodeKind::Group);
    assert_eq!(graph.open_groups[0].nodes.len(), outer_before);
    // Coming out, the copy is inside the group that was open.
    assert!(graph.close_group());
    let inner = graph.nodes[GROUP].group.as_ref().expect("a group");
    assert_eq!(
        inner
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Group)
            .count(),
        2
    );
}

/// An oscillator, a filter, and an output, with the filter's audio input free.
fn feedable(dir: &TempDir) -> (Graph, usize) {
    let mut graph = Graph::with(
        vec![
            node(NodeKind::Oscillator, 0.0),
            node(NodeKind::Filter, 300.0),
            node(NodeKind::Output, DEFAULT_OUTPUT_X),
        ],
        vec![wire(1, Port::Audio, 2, Port::Slot(0))],
    );
    graph.refresh_slots();
    graph.group_dir = Some(dir.0.clone());
    (graph, 1)
}

#[test]
fn a_dragged_back_wire_offers_the_groups_that_can_feed_it() {
    let dir = TempDir::new("feed");
    let mut saver = source();
    save_as(&mut saver, &dir, "Verb");
    // A group with no output pin cannot feed anything.
    let mut silent = Graph::with(vec![node(NodeKind::Oscillator, 0.0)], Vec::new());
    silent.add_node(NodeKind::Filter, Point::new(0.0, 0.0));
    let made = silent.create_group(&[1]).expect("a group");
    assert!(silent.nodes[made].outputs.is_empty());
    groups::save_to(
        &dir.0.join("Silent.flodegroup"),
        &SavedGroupFile::new(&silent.nodes[made], "Silent"),
    )
    .unwrap();

    let (mut graph, filter) = feedable(&dir);
    graph.open_quick_add_feeding(filter, Port::Audio, Point::new(100.0, 400.0));
    let names: Vec<String> = graph
        .quick_add
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .map(|g| g.entry.name.clone())
        .collect();
    assert_eq!(names, ["Verb"]);
    assert_eq!(
        graph.quick_add.as_ref().unwrap().groups[0].out,
        Some(audio_pin(0))
    );

    let mut editor = canvas(&mut graph);
    type_text(&mut editor, "verb");
    press_key(&mut editor, KeyCode::Enter);
    let placed = graph.nodes.len() - 1;
    assert_eq!(graph.nodes[placed].kind, NodeKind::Group);
    assert!(graph
        .connections
        .contains(&wire(placed, audio_pin(0), filter, Port::Audio)));
    assert!(matches!(graph.interaction, Interaction::Placing { .. }));
}

#[test]
fn a_control_input_lists_only_groups_with_a_control_output() {
    let dir = TempDir::new("feed-control");
    let mut saver = source();
    save_as(&mut saver, &dir, "Verb");
    let (mut graph, filter) = feedable(&dir);
    graph.open_quick_add_feeding(filter, Port::Knob(KnobId::Cutoff), Point::new(100.0, 400.0));
    assert!(graph.quick_add.as_ref().unwrap().groups.is_empty());
}

#[test]
fn the_groups_button_lists_saved_groups_and_places_one() {
    let dir = TempDir::new("pill");
    let mut graph = with_saved(&dir, &["Lead", "Pad"]);
    let before = graph.nodes.len();
    let index = super::menus::PILL_ITEMS
        .iter()
        .position(|item| *item == super::menus::PillItem::Groups)
        .expect("the pill has a Groups button");
    let segment = super::menus::pill_segment(bounds(), index);
    let button = Point::new(
        segment.x + segment.width / 2.0,
        segment.y + segment.height / 2.0,
    );
    click(&mut canvas(&mut graph), button);
    assert_eq!(graph.group_list, Some(vec![entry("Lead"), entry("Pad")]));
    let list = super::menus::group_list_bounds(bounds(), 2);
    let second = Point::new(
        list.x + 20.0,
        list.y + 1.5 * crate::editor::style::MENU_ITEM_HEIGHT,
    );
    click(&mut canvas(&mut graph), second);
    assert!(graph.group_list.is_none());
    assert_eq!(graph.nodes.len(), before + 1);
    assert_eq!(graph.nodes[before].label(), "Pad");
    assert!(matches!(graph.interaction, Interaction::Placing { .. }));

    // Pressing the button again closes the list.
    graph.interaction = Interaction::Idle;
    click(&mut canvas(&mut graph), button);
    assert!(graph.group_list.is_some());
    click(&mut canvas(&mut graph), button);
    assert!(graph.group_list.is_none());
}

#[test]
fn a_long_list_is_cut_off_above_the_keys() {
    let rows = super::menus::group_rows_shown(bounds(), 500);
    assert!(rows >= 4 && rows < 500);
    let list = super::menus::group_list_bounds(bounds(), 500);
    assert!(list.y + list.height <= bounds().height - super::piano::STRIP_HEIGHT);
    // A row past the cut is not a row.
    let below = Point::new(list.x + 10.0, list.y + list.height + 5.0);
    assert!(super::menus::group_row_at(bounds(), 500, below).is_none());
}
