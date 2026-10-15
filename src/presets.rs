//! Presets: a graph and the macro values under a name, kept as one file each in a folder. The
//! file name is the preset's name. Saving over an existing name replaces it.

use crate::editor::saved::SavedGraph;
use crate::nodes::macros::MACRO_COUNT;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// The file extension of a preset.
pub const EXTENSION: &str = "flode";

/// The longest a preset name can be, in characters.
pub const MAX_NAME_LEN: usize = 40;

/// A saved patch: the node graph, and the macro knobs, which are host parameters and so are not
/// part of the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub graph: SavedGraph,
    /// The macro knobs, normalized.
    #[serde(default)]
    pub macros: [f32; MACRO_COUNT],
}

/// The folder presets live in, or None if the home folder is unknown.
pub fn directory() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Flode/Presets"))
    } else {
        Some(home.join(".local/share/flode/presets"))
    }
}

/// True for a character a preset name may hold. Names are file names, so the characters that
/// file systems reserve, and control characters, are out.
pub fn valid_char(c: char) -> bool {
    !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// The name as it will be saved: trimmed, and not empty. Returns None for a name with nothing in
/// it, or one with a character that is not allowed.
pub fn clean_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let allowed = name.chars().all(valid_char) && name.chars().count() <= MAX_NAME_LEN;
    // A leading dot would hide the file, and these two names mean something to a file system.
    let reserved = name.starts_with('.');
    (!name.is_empty() && allowed && !reserved).then(|| name.to_string())
}

fn path_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

/// Writes the preset to `dir`, making the folder if it is not there. The file is written whole
/// before it takes the name, so a failed write never leaves half a preset.
pub fn save_in(dir: &Path, preset: &Preset) -> io::Result<()> {
    if clean_name(&preset.name).as_deref() != Some(preset.name.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that is not a usable preset name",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(preset)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let target = path_of(dir, &preset.name);
    let temporary = target.with_extension(format!("{EXTENSION}.tmp"));
    std::fs::write(&temporary, json)?;
    std::fs::rename(&temporary, &target)
}

/// The names of the presets in `dir`, sorted without regard to case. A missing folder has none.
pub fn list_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Reads the preset called `name` from `dir`. The file's name decides the preset's name.
pub fn load_in(dir: &Path, name: &str) -> io::Result<Preset> {
    let bytes = std::fs::read(path_of(dir, name))?;
    let mut preset: Preset = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    preset.name = name.to_string();
    Ok(preset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::graph::Graph;

    /// A folder of its own for one test, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("flode-presets-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn preset(name: &str) -> Preset {
        Preset {
            name: name.to_string(),
            graph: Graph::default().to_saved(),
            macros: [0.1, 0.2, 0.3, 0.4],
        }
    }

    #[test]
    fn a_preset_comes_back_as_it_was_saved() {
        let dir = TempDir::new("round-trip");
        save_in(&dir.0, &preset("Warm pad")).unwrap();
        let loaded = load_in(&dir.0, "Warm pad").unwrap();
        assert_eq!(loaded.name, "Warm pad");
        assert_eq!(loaded.macros, [0.1, 0.2, 0.3, 0.4]);
        // The graph inside is the one that went in, so it compiles to the same patch.
        assert!(Graph::from_saved(&loaded.graph).patch() == Graph::default().patch());
    }

    #[test]
    fn saving_again_replaces_the_preset() {
        let dir = TempDir::new("replace");
        save_in(&dir.0, &preset("Lead")).unwrap();
        let mut second = preset("Lead");
        second.macros = [0.9; MACRO_COUNT];
        save_in(&dir.0, &second).unwrap();
        assert_eq!(list_in(&dir.0), ["Lead"]);
        assert_eq!(load_in(&dir.0, "Lead").unwrap().macros, [0.9; MACRO_COUNT]);
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn the_list_is_sorted_and_ignores_other_files() {
        let dir = TempDir::new("list");
        assert!(list_in(&dir.0).is_empty());
        for name in ["pluck", "Bass", "arp"] {
            save_in(&dir.0, &preset(name)).unwrap();
        }
        std::fs::write(dir.0.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.0.join(".hidden.flode"), "x").unwrap();
        assert_eq!(list_in(&dir.0), ["arp", "Bass", "pluck"]);
    }

    #[test]
    fn a_bad_or_missing_file_is_an_error() {
        let dir = TempDir::new("errors");
        assert!(load_in(&dir.0, "nothing").is_err());
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(path_of(&dir.0, "broken"), "not json").unwrap();
        assert!(load_in(&dir.0, "broken").is_err());
    }

    #[test]
    fn names_are_cleaned_and_unsafe_ones_refused() {
        assert_eq!(clean_name("  Warm pad  "), Some("Warm pad".to_string()));
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("a/b"), None);
        assert_eq!(clean_name("..\\x"), None);
        assert_eq!(clean_name(".hidden"), None);
        assert_eq!(
            clean_name(&"x".repeat(MAX_NAME_LEN)).map(|n| n.len()),
            Some(MAX_NAME_LEN)
        );
        assert_eq!(clean_name(&"x".repeat(MAX_NAME_LEN + 1)), None);
        assert!(valid_char('é') && valid_char(' ') && !valid_char(':'));

        let dir = TempDir::new("refuse");
        assert!(save_in(&dir.0, &preset("a/b")).is_err());
        assert!(save_in(&dir.0, &preset(" padded ")).is_err());
    }
}
