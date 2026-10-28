//! Tests for the burger menu: the button, its top-level list, and the category submenus nested
//! under it. The preset and group systems' own mechanics are tested in `tests.rs` and
//! `group_file_tests.rs`; these tests cover the new entry point and the menu's own shape.

use super::canvas::{Message, NodeEditor};
use super::graph::{Graph, Interaction};
use super::menus::{
    burger_bounds, burger_list_bounds, burger_row_at, category_list_bounds, category_row_at,
    BurgerRow, BURGER_GROUPS_ROW, BURGER_PRESETS_ROW, BURGER_ROWS,
};
use crate::nodes::{Category, FlodeNode, NodeKind};
use crate::patch::control::ControlLevels;
use nih_plug_iced::mouse;
use nih_plug_iced::widget::canvas::{event::Event, Cursor, Program};
use nih_plug_iced::{Point, Rectangle};

fn bounds() -> Rectangle {
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 712.0,
    }
}

/// A plain graph with an oscillator and an output, wide open for adding anything else.
fn graph() -> Graph {
    Graph::with(
        vec![
            FlodeNode::new(NodeKind::Oscillator, Point::new(0.0, 80.0)),
            FlodeNode::new(NodeKind::Output, Point::new(400.0, 80.0)),
        ],
        Vec::new(),
    )
}

fn canvas(graph: &mut Graph) -> NodeEditor<'_> {
    NodeEditor::new(graph, ControlLevels::default(), [0; 2], [0.0; 2])
}

fn click(editor: &mut NodeEditor, at: Point) -> Option<Message> {
    let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    let message = editor.update(press, bounds(), Cursor::Available(at)).1;
    let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
    editor.update(release, bounds(), Cursor::Available(at));
    message
}

fn center(rect: Rectangle) -> Point {
    Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

fn burger_button() -> Point {
    center(burger_bounds())
}

/// The center of a row of the burger menu's top-level list, accounting for the divider.
fn row_point(row: usize) -> Point {
    center(super::menus::burger_row_bounds(row))
}

// Burger button and its top-level list.

/// The burger button sits clear of the logo, and a click opens the menu. A second click, or a
/// click elsewhere, closes it.
#[test]
fn the_burger_button_opens_and_closes_the_menu() {
    let logo = super::logo::logo_bounds(bounds()).expect("the logo fits the default window");
    let burger = burger_bounds();
    assert!(burger.x >= logo.x + logo.width);

    let mut graph = graph();
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    assert!(editor.graph.burger_menu);
    click(&mut editor, burger_button());
    assert!(!editor.graph.burger_menu);

    click(&mut editor, burger_button());
    assert!(editor.graph.burger_menu);
    click(&mut editor, Point::new(900.0, 600.0));
    assert!(!editor.graph.burger_menu);
}

/// The top-level rows are Add Oscillator, the four categories, then Presets and Groups, in that
/// order, with a divider between Utility and Presets that is not a row of its own.
#[test]
fn the_top_level_rows_are_in_order() {
    assert_eq!(
        BURGER_ROWS,
        [
            BurgerRow::AddOscillator,
            BurgerRow::AddSampler,
            BurgerRow::Category(Category::Midi),
            BurgerRow::Category(Category::Modulation),
            BurgerRow::Category(Category::Wavetable),
            BurgerRow::Category(Category::Effects),
            BurgerRow::Category(Category::Utility),
            BurgerRow::Presets,
            BurgerRow::Groups,
        ]
    );
    for (index, row) in BURGER_ROWS.iter().enumerate() {
        assert_eq!(burger_row_at(row_point(index)), Some(*row), "{index}");
    }
    // The divider sits between Utility and Presets, and is not itself a row.
    let presets = super::menus::burger_row_bounds(BURGER_PRESETS_ROW);
    let gap_point = Point::new(presets.x + 10.0, presets.y - 2.0);
    assert!(burger_list_bounds().contains(gap_point));
    assert_eq!(burger_row_at(gap_point), None);
}

/// Clicking Add Oscillator places an oscillator directly, the same way the old pill's Oscillator
/// button did, and closes the whole menu.
#[test]
fn add_oscillator_places_a_node_directly() {
    let mut graph = graph();
    let before = graph.nodes.len();
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    let message = click(&mut editor, row_point(0));
    assert!(matches!(message, Some(Message::GraphChanged)));
    assert!(!editor.graph.burger_menu);
    assert_eq!(editor.graph.nodes.len(), before + 1);
    assert_eq!(editor.graph.nodes[before].kind, NodeKind::Oscillator);
    assert!(matches!(
        editor.graph.interaction,
        Interaction::Placing { .. }
    ));
}

// Category submenus.

/// Each category's submenu lists exactly the kinds `Category::kinds` gives it, in that order, and
/// picking one places that kind. The burger menu stays open while a category's submenu is open,
/// nested beside it the same way the preset list's own category submenu nests beside the preset
/// list.
#[test]
fn a_category_row_opens_its_submenu_and_places_a_kind() {
    let midi_row = BURGER_ROWS
        .iter()
        .position(|row| *row == BurgerRow::Category(Category::Midi))
        .unwrap();
    let mut graph = graph();
    let before = graph.nodes.len();
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    click(&mut editor, row_point(midi_row));
    assert_eq!(editor.graph.category_menu, Some(Category::Midi));
    assert!(editor.graph.burger_menu);

    let list = category_list_bounds(bounds(), Category::Midi);
    let row0 = Point::new(list.x + 10.0, list.y + 10.0);
    assert_eq!(
        category_row_at(bounds(), Category::Midi, row0),
        Some(Category::Midi.kinds()[0])
    );
    let message = click(&mut editor, row0);
    assert!(matches!(message, Some(Message::GraphChanged)));
    assert_eq!(editor.graph.nodes.len(), before + 1);
    assert_eq!(editor.graph.nodes[before].kind, Category::Midi.kinds()[0]);
    assert!(!editor.graph.burger_menu);
    assert!(editor.graph.category_menu.is_none());
}

/// Clicking an open category's row again closes just its submenu, and leaves the burger menu
/// itself open.
#[test]
fn clicking_the_same_category_again_closes_its_submenu() {
    let midi_row = BURGER_ROWS
        .iter()
        .position(|row| *row == BurgerRow::Category(Category::Midi))
        .unwrap();
    let mut graph = graph();
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    click(&mut editor, row_point(midi_row));
    assert_eq!(editor.graph.category_menu, Some(Category::Midi));
    click(&mut editor, row_point(midi_row));
    assert_eq!(editor.graph.category_menu, None);
    assert!(editor.graph.burger_menu);
}

// Presets and Groups rows.

/// The Presets row opens the same preset list the old pill button did.
#[test]
fn the_presets_row_opens_the_preset_list() {
    let mut graph = graph();
    graph.preset_dir = None;
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    click(&mut editor, row_point(BURGER_PRESETS_ROW));
    assert!(editor.graph.preset_list.is_some());
}

/// The Groups row opens the same list of saved groups the old pill button did.
#[test]
fn the_groups_row_opens_the_group_list() {
    let mut graph = graph();
    graph.group_dir = None;
    let mut editor = canvas(&mut graph);
    click(&mut editor, burger_button());
    click(&mut editor, row_point(BURGER_GROUPS_ROW));
    assert!(editor.graph.group_list.is_some());
}

// Category kinds and the total count.

/// `Category::kinds` returns the new groupings exactly.
#[test]
fn categories_hold_the_new_groupings() {
    assert_eq!(
        Category::Midi.kinds(),
        &[NodeKind::Input, NodeKind::Arpeggiator, NodeKind::Chord]
    );
    assert_eq!(
        Category::Modulation.kinds(),
        &[
            NodeKind::Envelope,
            NodeKind::Lfo,
            NodeKind::Macro,
            NodeKind::Map,
            NodeKind::Math,
        ]
    );
    assert_eq!(
        Category::Effects.kinds(),
        &[
            NodeKind::Filter,
            NodeKind::Eq,
            NodeKind::Distortion,
            NodeKind::Compressor,
            NodeKind::Reverb,
            NodeKind::Delay,
            NodeKind::Chorus,
            NodeKind::Phaser,
            NodeKind::Flanger,
        ]
    );
    assert_eq!(
        Category::Utility.kinds(),
        &[
            NodeKind::Mix,
            NodeKind::Gain,
            NodeKind::Monitor,
            NodeKind::Output
        ]
    );
}

/// Every kind is in exactly one category, except the oscillator and the sampler, which are direct
/// rows, and the three group-internal kinds, which appear in no menu at all. The total matches
/// `NodeKind::ALL`.
#[test]
fn every_kind_is_categorised_or_direct_or_group_internal() {
    let categories = [
        Category::Midi,
        Category::Modulation,
        Category::Wavetable,
        Category::Effects,
        Category::Utility,
    ];
    let mut accounted = 0;
    for kind in NodeKind::ALL {
        let in_category = categories.iter().any(|c| c.kinds().contains(&kind));
        let direct = matches!(kind, NodeKind::Oscillator | NodeKind::Sampler);
        let group_internal = kind.is_group_part();
        assert!(
            in_category ^ direct ^ group_internal,
            "{kind:?} should be in exactly one place"
        );
        accounted += 1;
    }
    assert_eq!(accounted, NodeKind::ALL.len());
    let total: usize = categories.iter().map(|c| c.kinds().len()).sum();
    assert_eq!(total + 2 + 3, NodeKind::ALL.len());
}
