//! Start-up hooks for screenshots and demos. They read two environment variables when the editor
//! opens, and nothing reads them after that.
//!
//! `FLODE_PRESET_DIR` is a folder to read and write presets in, in place of the usual one.
//!
//! `FLODE_PRESET` is `Category/Name`, or just `Name` for a preset with no category. The editor
//! starts with that preset loaded.
//!
//! `FLODE_NOTE` is a MIDI note number to hold down from the start. It
//! gives the envelope and LFO viewers a live signal to show.
//!
//! `FLODE_UI` opens part of the interface. `presets` opens the preset list, `presets:Bass` opens
//! it with the Bass submenu out, `save` opens the save box, and `select:2` selects the node at
//! index 2.

use super::graph::Graph;

/// The MIDI note `FLODE_NOTE` asks to hold from the start, if it holds a number from 0 to 127.
/// The editor sends it on start, in the plugin and in the standalone app.
pub(super) fn note() -> Option<u8> {
    note_from(std::env::var("FLODE_NOTE").ok().as_deref())
}

/// The note a `FLODE_NOTE` value names.
fn note_from(value: Option<&str>) -> Option<u8> {
    value?.trim().parse::<u8>().ok().filter(|note| *note < 128)
}

/// Applies the hooks the environment asks for, if it asks for any.
pub(super) fn apply(graph: &mut Graph) {
    let dir = std::env::var("FLODE_PRESET_DIR").ok();
    let preset = std::env::var("FLODE_PRESET").ok();
    let ui = std::env::var("FLODE_UI").ok();
    apply_with(graph, dir.as_deref(), preset.as_deref(), ui.as_deref());
}

/// Uses `dir` for presets, loads `preset`, and opens `ui`, as the environment variables would ask.
/// A preset that can't be loaded, or a request this does not know, is ignored.
pub(super) fn apply_with(
    graph: &mut Graph,
    dir: Option<&str>,
    preset: Option<&str>,
    ui: Option<&str>,
) {
    if let Some(dir) = dir {
        graph.preset_dir = Some(dir.into());
    }
    if let Some(spec) = preset {
        let (category, name) = spec.split_once('/').unwrap_or(("", spec));
        graph.load_preset(category, name);
    }
    match ui.map(|ui| ui.split_once(':').unwrap_or((ui, ""))) {
        Some(("presets", category)) => {
            graph.open_preset_list();
            if let Some(menu) = graph.preset_list.as_mut() {
                menu.open = menu
                    .library
                    .categories
                    .iter()
                    .position(|c| c.name == category);
            }
        }
        Some(("save", _)) => graph.start_name_entry(),
        Some(("select", index)) => {
            graph.selected = index
                .parse()
                .ok()
                .filter(|&index| index < graph.nodes.len());
        }
        _ => {}
    }
    // The line about what loaded would only get in the way of a picture.
    graph.status = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use crate::presets::{self, Preset};

    /// A graph whose preset folder holds one preset, Bass/Wet, with a reverb in it.
    fn graph_with_a_preset(tag: &str) -> (Graph, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("flode-demo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut saved = Graph::default();
        saved.add_node(NodeKind::Reverb, nih_plug_iced::Point::new(400.0, 300.0));
        let preset = Preset {
            name: "Wet".to_string(),
            category: "Bass".to_string(),
            graph: saved.to_saved(),
            macros: [0.0; 4],
        };
        presets::save_in(&dir, &preset).unwrap();
        let mut graph = Graph::default();
        graph.preset_dir = Some(dir.clone());
        (graph, dir)
    }

    #[test]
    fn a_preset_and_a_menu_can_be_asked_for() {
        let (mut graph, dir) = graph_with_a_preset("ask");
        apply_with(&mut graph, None, Some("Bass/Wet"), Some("presets:Bass"));
        assert!(graph.nodes.iter().any(|n| n.kind == NodeKind::Reverb));
        assert_eq!(graph.preset_name.as_deref(), Some("Wet"));
        let menu = graph.preset_list.as_ref().expect("the list is open");
        assert_eq!(menu.open, Some(0));
        assert!(graph.status.is_none());

        let (mut graph, _) = graph_with_a_preset("save");
        apply_with(&mut graph, None, None, Some("save"));
        assert!(graph.name_entry.is_some() && graph.preset_list.is_none());

        // A folder can be named, and presets are read from it.
        let mut elsewhere = Graph::default();
        let nodes = elsewhere.nodes.len();
        apply_with(
            &mut elsewhere,
            dir.to_str(),
            Some("Bass/Wet"),
            Some("presets"),
        );
        assert_eq!(elsewhere.preset_dir.as_deref(), Some(dir.as_path()));
        assert!(elsewhere.nodes.len() > nodes);
        assert_eq!(elsewhere.preset_list.unwrap().library.categories.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_node_can_be_selected() {
        let mut graph = Graph::default();
        apply_with(&mut graph, None, None, Some("select:1"));
        assert_eq!(graph.selected, Some(1));
        apply_with(&mut graph, None, None, Some("select:99"));
        assert_eq!(graph.selected, None);
    }

    #[test]
    fn a_note_is_a_number_below_128() {
        assert_eq!(note_from(Some("60")), Some(60));
        assert_eq!(note_from(Some(" 0 ")), Some(0));
        assert_eq!(note_from(Some("128")), None);
        assert_eq!(note_from(Some("c4")), None);
        assert_eq!(note_from(None), None);
    }

    #[test]
    fn what_is_not_known_is_ignored() {
        let (mut graph, dir) = graph_with_a_preset("ignored");
        let nodes = graph.nodes.len();
        apply_with(&mut graph, None, Some("Nope/Missing"), Some("sideways"));
        assert_eq!(graph.nodes.len(), nodes);
        assert!(graph.preset_list.is_none() && graph.name_entry.is_none());
        apply_with(&mut graph, None, None, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
