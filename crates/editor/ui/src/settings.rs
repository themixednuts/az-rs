use gpui::Global;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub theme: String,
    pub keymap_profile: String,
    pub window_maximized: bool,
    pub source_navigation: SourceNavigationSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceNavigationSettings {
    pub file_url_template: String,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            theme: "Aether:dark".to_string(),
            keymap_profile: "default".to_string(),
            window_maximized: false,
            source_navigation: SourceNavigationSettings::default(),
        }
    }
}

impl Default for SourceNavigationSettings {
    fn default() -> Self {
        Self {
            file_url_template: "vscode://file/{path}{line_column_suffix}".to_string(),
        }
    }
}

/// Data-only settings store published by the editor shell as a GPUI global.
pub struct SettingsStore {
    pub current: EditorSettings,
}

impl Global for SettingsStore {}

impl SettingsStore {
    #[must_use]
    pub const fn new(current: EditorSettings) -> Self {
        Self { current }
    }

    pub fn replace(&mut self, current: EditorSettings) {
        self.current = current;
    }
}
