//! Saved groups: one group node's graph under a name, kept as one file each in a folder. The
//! folder is `Groups`, next to the preset folder, so a group file can never be taken for a preset
//! category. A folder inside it is a category, the way it is for presets. The file name is the
//! group's name, and saving over an existing name replaces it.

use crate::editor::saved::SavedGroupFile;
use crate::nodes::FlodeNode;
use crate::presets;
use std::io;
use std::path::{Path, PathBuf};

/// The file extension of a saved group.
pub const EXTENSION: &str = "flodegroup";

/// One saved group in the folder: its name, and the folder it is in. The category is empty for a
/// group that sits in the folder itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub category: String,
    pub name: String,
}

impl GroupEntry {
    /// The name with its category in front, if it has one, as the lists show it.
    pub fn label(&self) -> String {
        if self.category.is_empty() {
            self.name.clone()
        } else {
            format!("{} / {}", self.category, self.name)
        }
    }
}

/// The folder saved groups live in, or None if the home folder is unknown.
pub fn directory() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Flode/Groups"))
    } else {
        Some(home.join(".local/share/flode/groups"))
    }
}

/// The path with `.flodegroup` on the end of the whole file name, unless it already ends that way.
pub fn with_extension(path: &Path) -> PathBuf {
    presets::with_extension_of(path, EXTENSION)
}

/// Writes the file to `path`, making the folder it is in if it is not there. The write goes to a
/// temporary file first, so a failed write never leaves half a group.
pub fn save_to(path: &Path, file: &SavedGroupFile) -> io::Result<()> {
    presets::write_json(path, file)
}

/// Every saved group in `dir`, sorted by name without regard to case, with the groups in
/// category folders mixed in. A missing folder has none.
pub fn list_in(dir: &Path) -> Vec<GroupEntry> {
    let library = presets::library_with(dir, EXTENSION);
    let mut entries: Vec<GroupEntry> = library
        .loose
        .into_iter()
        .map(|name| GroupEntry {
            category: String::new(),
            name,
        })
        .collect();
    for category in library.categories {
        for name in category.presets {
            entries.push(GroupEntry {
                category: category.name.clone(),
                name,
            });
        }
    }
    entries.sort_by_key(|entry| (entry.name.to_lowercase(), entry.category.to_lowercase()));
    entries
}

/// Reads the group `entry` from `dir` and builds its node. The file's place decides the group's
/// name. The error says why the file was refused, in words for the status line.
pub fn load_in(dir: &Path, entry: &GroupEntry) -> Result<FlodeNode, String> {
    let folder = if entry.category.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(&entry.category)
    };
    let path = folder.join(format!("{}.{EXTENSION}", entry.name));
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut file: SavedGroupFile =
        serde_json::from_slice(&bytes).map_err(|error| format!("it is not a group ({error})"))?;
    file.rename(&entry.name);
    file.into_node()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::group::GroupPin;
    use crate::nodes::{NodeKind, Port, Signal};
    use nih_plug_iced::Point;

    /// A folder of its own for one test, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("flode-groups-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(category: &str, name: &str) -> GroupEntry {
        GroupEntry {
            category: category.to_string(),
            name: name.to_string(),
        }
    }

    /// A group node with the two edge nodes and nothing else.
    fn empty_group(name: &str) -> FlodeNode {
        let mut node = FlodeNode::new(NodeKind::Group, Point::new(40.0, 50.0));
        node.group = Some(Box::new(crate::nodes::group::GroupData::new(name)));
        node.inputs = vec![GroupPin::new("Audio", Signal::Audio)];
        node
    }

    #[test]
    fn the_folder_is_not_the_preset_folder() {
        let (groups, presets) = (directory(), presets::directory());
        if let (Some(groups), Some(presets)) = (groups, presets) {
            assert_ne!(groups, presets);
            assert!(!groups.starts_with(&presets));
        }
    }

    #[test]
    fn the_extension_is_added_to_the_whole_file_name() {
        assert_eq!(
            with_extension(Path::new("/a/My.Pad")),
            Path::new("/a/My.Pad.flodegroup")
        );
        assert_eq!(
            with_extension(Path::new("/a/Pad.flodegroup")),
            Path::new("/a/Pad.flodegroup")
        );
    }

    #[test]
    fn the_list_sorts_and_includes_categories() {
        let dir = TempDir::new("list");
        assert!(list_in(&dir.0).is_empty());
        let file = SavedGroupFile::new(&empty_group("x"), "x");
        for path in ["b.flodegroup", "Bass/a.flodegroup", "C.flodegroup"] {
            save_to(&dir.0.join(path), &file).unwrap();
        }
        std::fs::write(dir.0.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.0.join(".hidden.flodegroup"), "x").unwrap();
        // A preset in the folder is not a group.
        std::fs::write(dir.0.join("Pad.flode"), "x").unwrap();
        let found = list_in(&dir.0);
        assert_eq!(found, [entry("Bass", "a"), entry("", "b"), entry("", "C")]);
        assert_eq!(found[0].label(), "Bass / a");
        assert_eq!(found[1].label(), "b");
    }

    #[test]
    fn a_bad_or_missing_file_is_an_error() {
        let dir = TempDir::new("errors");
        assert!(load_in(&dir.0, &entry("", "nothing")).is_err());
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join("broken.flodegroup"), "not json").unwrap();
        assert!(load_in(&dir.0, &entry("", "broken")).is_err());
    }

    #[test]
    fn the_file_name_is_the_group_name() {
        let dir = TempDir::new("name");
        let file = SavedGroupFile::new(&empty_group("Old"), "Old");
        save_to(&dir.0.join("New.flodegroup"), &file).unwrap();
        let node = load_in(&dir.0, &entry("", "New")).unwrap();
        assert_eq!(node.group.as_ref().map(|g| g.name.as_str()), Some("New"));
        // The saved position, fold, and bypass do not carry over.
        assert_eq!(node.position, Point::new(0.0, 0.0));
    }

    /// A file with a node that can't be in a group, or a wire to a pin that is not there, is
    /// refused.
    #[test]
    fn a_file_with_an_output_or_a_missing_pin_is_refused() {
        let dir = TempDir::new("refused");
        let mut node = empty_group("Bad");
        let data = node.group.as_mut().unwrap();
        data.nodes
            .push(FlodeNode::new(NodeKind::Output, Point::new(0.0, 0.0)));
        let file = SavedGroupFile::new(&node, "Bad");
        save_to(&dir.0.join("Bad.flodegroup"), &file).unwrap();
        let error = load_in(&dir.0, &entry("", "Bad")).err().expect("refused");
        assert!(error.contains("Output"), "{error}");

        let mut node = empty_group("Pin");
        let data = node.group.as_mut().unwrap();
        data.nodes
            .push(FlodeNode::new(NodeKind::Filter, Point::new(0.0, 0.0)));
        data.links.push(crate::nodes::connection::Connection {
            from: 0,
            out: Port::Pin(Signal::Audio, 3),
            to: 2,
            input: Port::Audio,
        });
        let file = SavedGroupFile::new(&node, "Pin");
        save_to(&dir.0.join("Pin.flodegroup"), &file).unwrap();
        assert!(load_in(&dir.0, &entry("", "Pin")).is_err());

        let mut node = empty_group("Macro");
        let data = node.group.as_mut().unwrap();
        data.nodes
            .push(FlodeNode::new(NodeKind::Macro, Point::new(0.0, 0.0)));
        save_to(
            &dir.0.join("Macro.flodegroup"),
            &SavedGroupFile::new(&node, "Macro"),
        )
        .unwrap();
        assert!(load_in(&dir.0, &entry("", "Macro")).is_err());
    }
}
