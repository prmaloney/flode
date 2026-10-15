//! Presets: a graph and the macro values under a name, kept as one file each in a folder. A preset
//! can have a category, which is a folder inside the preset folder. The file name is the preset's
//! name. Saving over an existing name in the same category replaces it.

use crate::editor::saved::SavedGraph;
use crate::nodes::macros::MACRO_COUNT;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// The file extension of a preset.
pub const EXTENSION: &str = "flode";

/// The longest a preset or category name can be, in characters.
pub const MAX_NAME_LEN: usize = 40;

/// A saved patch: the node graph, and the macro knobs, which are host parameters and so are not
/// part of the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// The folder the preset is in, such as "Bass". Empty for a preset that has none. Where the
    /// file is decides the category, so loading sets this from the path.
    #[serde(default)]
    pub category: String,
    pub graph: SavedGraph,
    /// The macro knobs, normalized.
    #[serde(default)]
    pub macros: [f32; MACRO_COUNT],
}

/// One category and the names of the presets in it, sorted without regard to case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub presets: Vec<String>,
}

/// Every preset in the folder: the ones sorted into categories, and the ones with none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    pub categories: Vec<Category>,
    /// Presets saved with no category, which sit in the folder itself.
    pub loose: Vec<String>,
}

impl Library {
    /// True if there are no presets at all.
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.loose.is_empty()
    }
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

/// True for a character a preset or category name may hold. Names are file names, so the
/// characters that file systems reserve, and control characters, are out.
pub fn valid_char(c: char) -> bool {
    !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// The name as it will be saved: trimmed, and not empty. Returns None for a name with nothing in
/// it, or one with a character that is not allowed.
pub fn clean_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let allowed = name.chars().all(valid_char) && name.chars().count() <= MAX_NAME_LEN;
    // A leading dot would hide the file.
    let hidden = name.starts_with('.');
    (!name.is_empty() && allowed && !hidden).then(|| name.to_string())
}

/// The category as it will be saved. An empty one is allowed and means none. Anything else has to
/// be a usable name, and None means it is not.
pub fn clean_category(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        Some(String::new())
    } else {
        clean_name(raw)
    }
}

fn folder_of(dir: &Path, category: &str) -> PathBuf {
    if category.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(category)
    }
}

fn path_of(dir: &Path, category: &str, name: &str) -> PathBuf {
    folder_of(dir, category).join(format!("{name}.{EXTENSION}"))
}

/// Writes the preset to `dir`, in its category's folder, making folders that are not there. The
/// file is written whole before it takes the name, so a failed write never leaves half a preset.
pub fn save_in(dir: &Path, preset: &Preset) -> io::Result<()> {
    let usable = clean_name(&preset.name).as_deref() == Some(preset.name.as_str())
        && clean_category(&preset.category).as_deref() == Some(preset.category.as_str());
    if !usable {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that is not a usable preset name or category",
        ));
    }
    std::fs::create_dir_all(folder_of(dir, &preset.category))?;
    let json = serde_json::to_vec_pretty(preset)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let target = path_of(dir, &preset.category, &preset.name);
    let temporary = target.with_extension(format!("{EXTENSION}.tmp"));
    std::fs::write(&temporary, json)?;
    std::fs::rename(&temporary, &target)
}

/// The names of the presets in one folder, sorted without regard to case.
fn names_in(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Every preset in `dir`. A folder with presets in it is a category, and a folder with none is
/// left out. A missing folder has no presets.
pub fn library_in(dir: &Path) -> Library {
    let mut library = Library {
        categories: Vec::new(),
        loose: names_in(dir),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return library;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !path.is_dir() || name.starts_with('.') {
            continue;
        }
        let presets = names_in(&path);
        if !presets.is_empty() {
            library.categories.push(Category {
                name: name.to_string(),
                presets,
            });
        }
    }
    library
        .categories
        .sort_by_key(|category| category.name.to_lowercase());
    library
}

/// Reads the preset called `name` in `category` (empty for none) from `dir`. The file's place
/// decides the preset's name and category.
pub fn load_in(dir: &Path, category: &str, name: &str) -> io::Result<Preset> {
    let bytes = std::fs::read(path_of(dir, category, name))?;
    let mut preset: Preset = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    preset.name = name.to_string();
    preset.category = category.to_string();
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

    fn preset(category: &str, name: &str) -> Preset {
        Preset {
            name: name.to_string(),
            category: category.to_string(),
            graph: Graph::default().to_saved(),
            macros: [0.1, 0.2, 0.3, 0.4],
        }
    }

    #[test]
    fn a_preset_comes_back_as_it_was_saved() {
        let dir = TempDir::new("round-trip");
        save_in(&dir.0, &preset("Pad", "Warm pad")).unwrap();
        let loaded = load_in(&dir.0, "Pad", "Warm pad").unwrap();
        assert_eq!(loaded.name, "Warm pad");
        assert_eq!(loaded.category, "Pad");
        assert_eq!(loaded.macros, [0.1, 0.2, 0.3, 0.4]);
        // The graph inside is the one that went in, so it compiles to the same patch.
        assert!(Graph::from_saved(&loaded.graph).patch() == Graph::default().patch());
        // The category is a folder.
        assert!(dir.0.join("Pad").join("Warm pad.flode").is_file());
    }

    /// The same name in two categories is two presets.
    #[test]
    fn a_name_can_repeat_across_categories() {
        let dir = TempDir::new("repeat");
        save_in(&dir.0, &preset("Bass", "Dark")).unwrap();
        let mut other = preset("Pad", "Dark");
        other.macros = [0.9; MACRO_COUNT];
        save_in(&dir.0, &other).unwrap();
        assert_eq!(load_in(&dir.0, "Bass", "Dark").unwrap().macros[0], 0.1);
        assert_eq!(load_in(&dir.0, "Pad", "Dark").unwrap().macros[0], 0.9);
    }

    #[test]
    fn saving_again_replaces_the_preset() {
        let dir = TempDir::new("replace");
        save_in(&dir.0, &preset("Lead", "Pluck")).unwrap();
        let mut second = preset("Lead", "Pluck");
        second.macros = [0.9; MACRO_COUNT];
        save_in(&dir.0, &second).unwrap();
        let library = library_in(&dir.0);
        assert_eq!(library.categories.len(), 1);
        assert_eq!(library.categories[0].presets, ["Pluck"]);
        assert_eq!(
            load_in(&dir.0, "Lead", "Pluck").unwrap().macros,
            [0.9; MACRO_COUNT]
        );
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(dir.0.join("Lead")).unwrap().count(), 1);
    }

    /// The library groups by folder, sorts without regard to case, keeps presets with no category
    /// apart, and ignores everything that is not a preset.
    #[test]
    fn the_library_groups_and_sorts() {
        let dir = TempDir::new("library");
        assert!(library_in(&dir.0).is_empty());
        for (category, name) in [
            ("pad", "Warm"),
            ("Bass", "sub"),
            ("Bass", "Acid"),
            ("", "loose two"),
            ("", "Loose one"),
        ] {
            save_in(&dir.0, &preset(category, name)).unwrap();
        }
        std::fs::write(dir.0.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.0.join(".hidden.flode"), "x").unwrap();
        std::fs::create_dir_all(dir.0.join("Empty")).unwrap();
        std::fs::create_dir_all(dir.0.join(".cache")).unwrap();
        std::fs::write(dir.0.join(".cache").join("x.flode"), "x").unwrap();

        let library = library_in(&dir.0);
        let names: Vec<&str> = library.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Bass", "pad"]);
        assert_eq!(library.categories[0].presets, ["Acid", "sub"]);
        assert_eq!(library.loose, ["Loose one", "loose two"]);
        assert!(!library.is_empty());
    }

    #[test]
    fn a_bad_or_missing_file_is_an_error() {
        let dir = TempDir::new("errors");
        assert!(load_in(&dir.0, "", "nothing").is_err());
        std::fs::create_dir_all(dir.0.join("Bass")).unwrap();
        std::fs::write(path_of(&dir.0, "Bass", "broken"), "not json").unwrap();
        assert!(load_in(&dir.0, "Bass", "broken").is_err());
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

        // A category may be empty, which means none. Otherwise it follows the same rules.
        assert_eq!(clean_category("  "), Some(String::new()));
        assert_eq!(clean_category(" Bass "), Some("Bass".to_string()));
        assert_eq!(clean_category("../up"), None);

        let dir = TempDir::new("refuse");
        assert!(save_in(&dir.0, &preset("", "a/b")).is_err());
        assert!(save_in(&dir.0, &preset("", " padded ")).is_err());
        assert!(save_in(&dir.0, &preset("../out", "x")).is_err());
        assert!(save_in(&dir.0, &preset(" Bass", "x")).is_err());
    }
}
