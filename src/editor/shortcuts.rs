//! Keyboard shortcuts. Each one is a key, with Shift held or not, bound to an action on the
//! selected node. To add one, add a variant to `Shortcut`, a row to `BINDINGS`, and an arm to the
//! match in `NodeEditor::run_shortcut`. Lookup matches the key and the modifiers. The editor
//! captures the press, its repeats, and its release, so the system does not beep at them.
//!
//! Control, Alt, and Command are left alone. The host and the system use those for their own
//! shortcuts, so a binding never claims a key pressed with one of them. The group shortcuts are
//! the exception. Ableton Live takes Command+G and Command+Shift+G for its own track groups, so
//! Shift+G groups and Shift+Alt+G ungroups. The command key, which is Command on macOS and
//! Control elsewhere, groups and ungroups too, for the standalone app and for hosts that pass it
//! on. The other one of the pair still passes through.
//!
//! In Keyboard mode the computer keyboard plays notes, on the keys A to P and semicolon, and Z
//! and X shift the octave. A shortcut on one of those keys needs Shift, which the piano ignores,
//! and the shortcut wins over the piano when both match. `BINDINGS` holds only shortcuts that are
//! safe in Keyboard mode this way.
//!
//! Edit mode turns the piano off and gives plain letters to the editor instead: G grabs the
//! selection, X and Delete remove it, A selects everything, and Alt+A clears the selection.
//! Command+X removes a node and reconnects around it, Blender's node editor trick for deleting
//! without breaking the chain it sat in. `EDIT_BINDINGS` holds these, since they would shadow the
//! piano in Keyboard mode. `lookup_edit` checks them, and `NodeEditor` only calls it in Edit mode.

use nih_plug_iced::keyboard::{KeyCode, Modifiers};

/// What a shortcut does to the selected node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shortcut {
    /// Mutes a source, or bypasses a processor.
    ToggleBypass,
    /// Copies the node.
    Duplicate,
    /// Opens the quick add box at the cursor. It needs no selected node.
    QuickAdd,
    /// Makes a group of the selected nodes.
    Group,
    /// Breaks the selected group node back into its nodes.
    Ungroup,
    /// Closes the open group, or closes an open menu first.
    Escape,
    /// Starts a modal grab of the selection, in Edit mode. Handled on its own in `shortcut_event`
    /// since it starts an interaction rather than changing the graph right away.
    Grab,
    /// Removes the selection, in Edit mode.
    DeleteSelection,
    /// Removes the selection, in Edit mode, reconnecting whatever fed each node straight to
    /// whatever it fed. Blender's node editor calls this Delete and Reconnect.
    DeleteInPlace,
    /// Selects every node, in Edit mode.
    SelectAll,
    /// Clears the selection, in Edit mode.
    DeselectAll,
}

/// One key, and whether Shift, Alt, and the command key must be held with it.
struct Binding {
    key: KeyCode,
    shift: bool,
    alt: bool,
    command: bool,
    shortcut: Shortcut,
}

/// Every shortcut that is safe in Keyboard mode: none of these shadow a piano key without Shift
/// or the command key held.
const BINDINGS: [Binding; 8] = [
    Binding {
        key: KeyCode::A,
        shift: true,
        alt: false,
        command: false,
        shortcut: Shortcut::QuickAdd,
    },
    Binding {
        key: KeyCode::M,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::ToggleBypass,
    },
    Binding {
        key: KeyCode::D,
        shift: true,
        alt: false,
        command: false,
        shortcut: Shortcut::Duplicate,
    },
    Binding {
        key: KeyCode::G,
        shift: true,
        alt: false,
        command: false,
        shortcut: Shortcut::Group,
    },
    Binding {
        key: KeyCode::G,
        shift: true,
        alt: true,
        command: false,
        shortcut: Shortcut::Ungroup,
    },
    Binding {
        key: KeyCode::G,
        shift: false,
        alt: false,
        command: true,
        shortcut: Shortcut::Group,
    },
    Binding {
        key: KeyCode::G,
        shift: true,
        alt: false,
        command: true,
        shortcut: Shortcut::Ungroup,
    },
    Binding {
        key: KeyCode::Escape,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::Escape,
    },
];

/// True if the key that the system uses for its own shortcuts is down: Command on macOS, Control
/// everywhere else.
fn command(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.logo()
    } else {
        modifiers.control()
    }
}

/// True if the other one of Control and Command is down. It never makes a shortcut.
fn foreign_command(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.control()
    } else {
        modifiers.logo()
    }
}

/// The shortcut for a key pressed with these modifiers, if there is one.
pub(super) fn lookup(key: KeyCode, modifiers: Modifiers) -> Option<Shortcut> {
    if foreign_command(modifiers) {
        return None;
    }
    BINDINGS
        .iter()
        .find(|binding| {
            binding.key == key
                && binding.shift == modifiers.shift()
                && binding.alt == modifiers.alt()
                && binding.command == command(modifiers)
        })
        .map(|binding| binding.shortcut)
}

/// The letter shortcuts Edit mode gives to the editor, on keys the piano would otherwise use in
/// Keyboard mode. Kept apart from `BINDINGS` so a Keyboard-mode lookup, and the test that checks
/// `BINDINGS` never shadows a piano key, see none of these.
const EDIT_BINDINGS: [Binding; 6] = [
    Binding {
        key: KeyCode::G,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::Grab,
    },
    Binding {
        key: KeyCode::X,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::DeleteSelection,
    },
    Binding {
        key: KeyCode::Delete,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::DeleteSelection,
    },
    Binding {
        key: KeyCode::X,
        shift: false,
        alt: false,
        command: true,
        shortcut: Shortcut::DeleteInPlace,
    },
    Binding {
        key: KeyCode::A,
        shift: false,
        alt: false,
        command: false,
        shortcut: Shortcut::SelectAll,
    },
    Binding {
        key: KeyCode::A,
        shift: false,
        alt: true,
        command: false,
        shortcut: Shortcut::DeselectAll,
    },
];

/// The Edit-mode shortcut for a key pressed with these modifiers, if there is one. Checked only
/// while the editor is in Edit mode, after `lookup` finds nothing, so Shift+A and the other
/// Keyboard-mode-safe shortcuts still win when they also match.
pub(super) fn lookup_edit(key: KeyCode, modifiers: Modifiers) -> Option<Shortcut> {
    if foreign_command(modifiers) {
        return None;
    }
    EDIT_BINDINGS
        .iter()
        .find(|binding| {
            binding.key == key
                && binding.shift == modifiers.shift()
                && binding.alt == modifiers.alt()
                && binding.command == command(modifiers)
        })
        .map(|binding| binding.shortcut)
}

/// The modifier a test holds to press the command key on this system.
#[cfg(test)]
pub(super) const COMMAND: Modifiers = if cfg!(target_os = "macos") {
    Modifiers::LOGO
} else {
    Modifiers::CTRL
};

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

    /// Shift+A opens the quick add box, and a plain A is left to the piano.
    #[test]
    fn shift_a_opens_quick_add() {
        assert_eq!(
            lookup(KeyCode::A, Modifiers::SHIFT),
            Some(Shortcut::QuickAdd)
        );
        assert_eq!(lookup(KeyCode::A, Modifiers::default()), None);
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

    /// Shift+G groups and Shift+Alt+G ungroups. Live keeps the command versions for itself.
    #[test]
    fn shift_g_groups_and_shift_alt_g_ungroups() {
        assert_eq!(lookup(KeyCode::G, Modifiers::SHIFT), Some(Shortcut::Group));
        assert_eq!(
            lookup(KeyCode::G, Modifiers::SHIFT | Modifiers::ALT),
            Some(Shortcut::Ungroup)
        );
        // A plain G is a piano key, and Alt alone is not bound.
        assert_eq!(lookup(KeyCode::G, Modifiers::default()), None);
        assert_eq!(lookup(KeyCode::G, Modifiers::ALT), None);
        // Control and Command stay with the host with Shift and Alt held.
        assert_eq!(
            lookup(KeyCode::G, Modifiers::SHIFT | Modifiers::ALT | COMMAND),
            None
        );
    }

    /// The command key groups and ungroups too, with Shift for ungroup.
    #[test]
    fn command_g_groups_and_command_shift_g_ungroups() {
        assert_eq!(lookup(KeyCode::G, COMMAND), Some(Shortcut::Group));
        assert_eq!(
            lookup(KeyCode::G, COMMAND | Modifiers::SHIFT),
            Some(Shortcut::Ungroup)
        );
        // The other of Control and Command, and Alt, stay with the system.
        let foreign = if cfg!(target_os = "macos") {
            Modifiers::CTRL
        } else {
            Modifiers::LOGO
        };
        assert_eq!(lookup(KeyCode::G, foreign), None);
        assert_eq!(lookup(KeyCode::G, COMMAND | Modifiers::ALT), None);
    }

    /// Escape is bound with no modifier, and a held one changes nothing about it. Tab is not a
    /// shortcut at all: it is the global mode toggle, handled before shortcuts are looked up.
    #[test]
    fn escape_is_bound_plain() {
        assert_eq!(
            lookup(KeyCode::Escape, Modifiers::default()),
            Some(Shortcut::Escape)
        );
        assert_eq!(lookup(KeyCode::Tab, Modifiers::default()), None);
    }

    /// Keys with no shortcut are not claimed.
    #[test]
    fn unbound_keys_are_not_claimed() {
        assert_eq!(lookup(KeyCode::Q, Modifiers::default()), None);
        assert_eq!(lookup(KeyCode::Q, Modifiers::SHIFT), None);
    }

    /// No shortcut shadows a piano key without Shift or the command key, or the octave keys at
    /// all.
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
                assert!(
                    binding.shift || binding.command,
                    "{:?} would shadow a piano key",
                    binding.key
                );
            }
        }
    }

    /// The Edit-mode letter shortcuts: a plain G, X, Delete, and A, and Alt+A.
    #[test]
    fn edit_bindings_cover_grab_delete_and_select_all() {
        assert_eq!(
            lookup_edit(KeyCode::G, Modifiers::default()),
            Some(Shortcut::Grab)
        );
        assert_eq!(
            lookup_edit(KeyCode::X, Modifiers::default()),
            Some(Shortcut::DeleteSelection)
        );
        assert_eq!(
            lookup_edit(KeyCode::Delete, Modifiers::default()),
            Some(Shortcut::DeleteSelection)
        );
        assert_eq!(
            lookup_edit(KeyCode::A, Modifiers::default()),
            Some(Shortcut::SelectAll)
        );
        assert_eq!(
            lookup_edit(KeyCode::A, Modifiers::ALT),
            Some(Shortcut::DeselectAll)
        );
        // Shift+A is still the quick add box, which `lookup` finds first in `shortcut_event`.
        assert_eq!(lookup_edit(KeyCode::A, Modifiers::SHIFT), None);
    }
}
