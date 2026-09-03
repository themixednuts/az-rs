use gpui::{App, KeyBinding};

pub const KEYMAP_PROFILE_DEFAULT: &str = "default";

pub const ACTION_NEW_PROJECT: &str = "new_project";
pub const ACTION_OPEN_PROJECT: &str = "open_project";
pub const ACTION_SAVE: &str = "save";
pub const ACTION_UNDO: &str = "undo";
pub const ACTION_REDO: &str = "redo";
pub const ACTION_CUT: &str = "cut";
pub const ACTION_COPY: &str = "copy";
pub const ACTION_PASTE: &str = "paste";
pub const ACTION_TOGGLE_OUTLINER: &str = "toggle_outliner";
pub const ACTION_TOGGLE_INSPECTOR: &str = "toggle_inspector";
pub const ACTION_TOGGLE_ASSET_BROWSER: &str = "toggle_asset_browser";
pub const ACTION_TOGGLE_SESSION_PANEL: &str = "toggle_session_panel";
pub const ACTION_TOGGLE_GRAPH_PANEL: &str = "toggle_graph_panel";
pub const ACTION_TOGGLE_CONSOLE: &str = "toggle_console";
pub const ACTION_QUIT: &str = "quit";
pub const ACTION_TOGGLE_FULLSCREEN: &str = "toggle_fullscreen";
pub const ACTION_MINIMIZE: &str = "minimize";
pub const ACTION_ZOOM: &str = "zoom";
pub const ACTION_PREFERENCES: &str = "preferences";
pub const ACTION_DISMISS_OVERLAY: &str = "dismiss_overlay";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeymapProfile {
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyBindingDescriptor {
    action: &'static str,
    keystroke: &'static str,
}

const KEYMAP_PROFILES: &[KeymapProfile] = &[KeymapProfile {
    key: KEYMAP_PROFILE_DEFAULT,
    label: "Default",
}];

const DEFAULT_BINDINGS: &[KeyBindingDescriptor] = &[
    KeyBindingDescriptor {
        action: ACTION_NEW_PROJECT,
        keystroke: "ctrl-shift-n",
    },
    KeyBindingDescriptor {
        action: ACTION_OPEN_PROJECT,
        keystroke: "ctrl-o",
    },
    KeyBindingDescriptor {
        action: ACTION_SAVE,
        keystroke: "ctrl-s",
    },
    KeyBindingDescriptor {
        action: ACTION_UNDO,
        keystroke: "ctrl-z",
    },
    KeyBindingDescriptor {
        action: ACTION_REDO,
        keystroke: "ctrl-y",
    },
    KeyBindingDescriptor {
        action: ACTION_REDO,
        keystroke: "ctrl-shift-z",
    },
    KeyBindingDescriptor {
        action: ACTION_CUT,
        keystroke: "ctrl-x",
    },
    KeyBindingDescriptor {
        action: ACTION_COPY,
        keystroke: "ctrl-c",
    },
    KeyBindingDescriptor {
        action: ACTION_PASTE,
        keystroke: "ctrl-v",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_OUTLINER,
        keystroke: "ctrl-1",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_INSPECTOR,
        keystroke: "ctrl-2",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_ASSET_BROWSER,
        keystroke: "ctrl-3",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_SESSION_PANEL,
        keystroke: "ctrl-4",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_GRAPH_PANEL,
        keystroke: "ctrl-5",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_CONSOLE,
        keystroke: "ctrl-`",
    },
    KeyBindingDescriptor {
        action: ACTION_QUIT,
        keystroke: "ctrl-q",
    },
    KeyBindingDescriptor {
        action: ACTION_TOGGLE_FULLSCREEN,
        keystroke: "f11",
    },
    KeyBindingDescriptor {
        action: ACTION_MINIMIZE,
        keystroke: "ctrl-m",
    },
    KeyBindingDescriptor {
        action: ACTION_ZOOM,
        keystroke: "ctrl-+",
    },
    KeyBindingDescriptor {
        action: ACTION_PREFERENCES,
        keystroke: "ctrl-,",
    },
    KeyBindingDescriptor {
        action: ACTION_DISMISS_OVERLAY,
        keystroke: "escape",
    },
];

/// Bind a minimal default keymap using gpui's `KeyBinding` API.
/// Maps keystrokes to actions through one central keymap.
pub fn bind_default_keymap(cx: &mut App) {
    // Clear any existing bindings before applying ours
    cx.clear_key_bindings();

    let bindings: Vec<KeyBinding> = DEFAULT_BINDINGS
        .iter()
        .filter_map(binding_for_descriptor)
        .collect();

    cx.bind_keys(bindings);
}

pub fn bind_keymap_profile(cx: &mut App, profile: &str) {
    if !profile.eq_ignore_ascii_case(KEYMAP_PROFILE_DEFAULT) {
        tracing::warn!(
            target: "az_editor::keymaps",
            profile,
            "unsupported keymap profile requested; applying default keymap"
        );
    }
    bind_default_keymap(cx);
}

#[must_use]
pub const fn keymap_profiles() -> &'static [KeymapProfile] {
    KEYMAP_PROFILES
}

#[must_use]
pub fn shortcut_label(profile: &str, action: &str) -> Option<String> {
    if !profile.eq_ignore_ascii_case(KEYMAP_PROFILE_DEFAULT) {
        return None;
    }

    DEFAULT_BINDINGS
        .iter()
        .find(|binding| binding.action == action)
        .map(|binding| format_keystroke(binding.keystroke))
}

fn binding_for_descriptor(descriptor: &KeyBindingDescriptor) -> Option<KeyBinding> {
    let keystroke = descriptor.keystroke;
    match descriptor.action {
        ACTION_NEW_PROJECT => Some(KeyBinding::new(keystroke, crate::actions::NewProject, None)),
        ACTION_OPEN_PROJECT => Some(KeyBinding::new(
            keystroke,
            crate::actions::OpenProject,
            None,
        )),
        ACTION_SAVE => Some(KeyBinding::new(keystroke, crate::actions::Save, None)),
        ACTION_UNDO => Some(KeyBinding::new(keystroke, crate::actions::Undo, None)),
        ACTION_REDO => Some(KeyBinding::new(keystroke, crate::actions::Redo, None)),
        ACTION_CUT => Some(KeyBinding::new(keystroke, crate::actions::Cut, None)),
        ACTION_COPY => Some(KeyBinding::new(keystroke, crate::actions::Copy, None)),
        ACTION_PASTE => Some(KeyBinding::new(keystroke, crate::actions::Paste, None)),
        ACTION_TOGGLE_OUTLINER => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleOutliner,
            None,
        )),
        ACTION_TOGGLE_INSPECTOR => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleInspector,
            None,
        )),
        ACTION_TOGGLE_ASSET_BROWSER => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleAssetBrowser,
            None,
        )),
        ACTION_TOGGLE_SESSION_PANEL => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleSessionPanel,
            None,
        )),
        ACTION_TOGGLE_GRAPH_PANEL => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleGraphPanel,
            None,
        )),
        ACTION_TOGGLE_CONSOLE => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleConsole,
            None,
        )),
        ACTION_QUIT => Some(KeyBinding::new(keystroke, crate::actions::Quit, None)),
        ACTION_TOGGLE_FULLSCREEN => Some(KeyBinding::new(
            keystroke,
            crate::actions::ToggleFullscreen,
            None,
        )),
        ACTION_MINIMIZE => Some(KeyBinding::new(keystroke, crate::actions::Minimize, None)),
        ACTION_ZOOM => Some(KeyBinding::new(keystroke, crate::actions::Zoom, None)),
        ACTION_PREFERENCES => Some(KeyBinding::new(
            keystroke,
            crate::actions::Preferences,
            None,
        )),
        ACTION_DISMISS_OVERLAY => Some(KeyBinding::new(
            keystroke,
            crate::actions::DismissOverlay,
            None,
        )),
        _ => None,
    }
}

fn format_keystroke(keystroke: &str) -> String {
    keystroke
        .split('-')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_owned(),
            "shift" => "Shift".to_owned(),
            "alt" => "Alt".to_owned(),
            "cmd" => "Cmd".to_owned(),
            "meta" => "Meta".to_owned(),
            "+" => "Plus".to_owned(),
            "`" => "`".to_owned(),
            "," => ",".to_owned(),
            key if key.starts_with('f') && key[1..].chars().all(|ch| ch.is_ascii_digit()) => {
                key.to_ascii_uppercase()
            }
            key if key.len() == 1 => key.to_ascii_uppercase(),
            key => key.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}
