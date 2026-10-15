//! Keyboard shortcuts. Each one is a key, with Shift held or not, bound to an action on the
//! selected node. To add one, add a variant to `Shortcut`, a row to `BINDINGS`, and an arm to the
//! match in `NodeEditor::run_shortcut`. Lookup matches the key and the modifiers. The editor
//! captures the press, its repeats, and its release, so the system does not beep at them.
//!
//! Control, Alt, and Command are left alone. The host and the system use those for their own
//! shortcuts, so a binding never claims a key pressed with one of them.
//!
//! The computer keyboard also plays notes in the standalone app, on the keys A to P and
//! semicolon, and Z and X shift the octave. A shortcut on one of those keys needs Shift, which
//! the piano ignores, and the shortcut wins over the piano when both match.

use nih_plug_iced::keyboard::{KeyCode, Modifiers};

/// What a shortcut does to the selected node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shortcut {
    /// Mutes a source, or bypasses a processor.
    ToggleBypass,
    /// Copies the node.
    Duplicate,
}

/// One key and whether Shift must be held with it.
struct Binding {
    key: KeyCode,
    shift: bool,
    shortcut: Shortcut,
}

/// Every shortcut.
const BINDINGS: [Binding; 2] = [
    Binding {
        key: KeyCode::M,
        shift: false,
        shortcut: Shortcut::ToggleBypass,
    },
    Binding {
        key: KeyCode::D,
        shift: true,
        shortcut: Shortcut::Duplicate,
    },
];

/// The shortcut for a key pressed with these modifiers, if there is one.
pub(super) fn lookup(key: KeyCode, modifiers: Modifiers) -> Option<Shortcut> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    BINDINGS
        .iter()
        .find(|binding| binding.key == key && binding.shift == modifiers.shift())
        .map(|binding| binding.shortcut)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M with no modifier bypasses.
    #[test]
    fn m_toggles_the_bypass() {
        assert_eq!(
            lookup(KeyCode::M, Modifiers::default()),
            Some(Shortcut::ToggleBypass)
        );
    }

    /// Shift+D duplicates, and a plain D is left to the piano.
    #[test]
    fn shift_d_duplicates_and_plain_d_does_not() {
        assert_eq!(
            lookup(KeyCode::D, Modifiers::SHIFT),
            Some(Shortcut::Duplicate)
        );
        assert_eq!(lookup(KeyCode::D, Modifiers::default()), None);
    }

    /// Shift must match each binding, so Shift+M is not the bypass.
    #[test]
    fn shift_has_to_match() {
        assert_eq!(lookup(KeyCode::M, Modifiers::SHIFT), None);
    }

    /// Control, Alt, and Command belong to the host and the system.
    #[test]
    fn other_modifiers_are_left_alone() {
        for modifiers in [
            Modifiers::CTRL,
            Modifiers::ALT,
            Modifiers::LOGO,
            Modifiers::SHIFT | Modifiers::CTRL,
        ] {
            assert_eq!(lookup(KeyCode::M, modifiers), None, "{modifiers:?}");
            assert_eq!(lookup(KeyCode::D, modifiers), None, "{modifiers:?}");
        }
    }

    /// Keys with no shortcut are not claimed.
    #[test]
    fn unbound_keys_are_not_claimed() {
        assert_eq!(lookup(KeyCode::Q, Modifiers::default()), None);
        assert_eq!(lookup(KeyCode::Q, Modifiers::SHIFT), None);
    }

    /// No shortcut shadows a piano key without Shift, or the octave keys at all.
    #[test]
    fn shortcuts_do_not_steal_piano_keys() {
        use crate::editor::piano::Piano;
        use nih_plug_iced::keyboard;
        for binding in &BINDINGS {
            let event = keyboard::Event::KeyPressed {
                key_code: binding.key,
                modifiers: Modifiers::default(),
            };
            if Piano::uses(&event) {
                assert!(binding.shift, "{:?} would shadow a piano key", binding.key);
            }
        }
    }
}
