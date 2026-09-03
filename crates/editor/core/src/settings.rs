//! Editor-shell ownership for global and project settings.
//!
//! `az-editor-ui` only defines the GPUI data shape. The standalone editor
//! shell owns path resolution and persistence because those decisions depend on
//! the attached project/workspace context.

pub use az_editor_ui::settings::{EditorSettings, SettingsStore, SourceNavigationSettings};
use az_filesystem::{AzothDataHome, ProjectDataPaths};
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub global: PathBuf,
    pub project: Option<PathBuf>,
}

impl SettingsPaths {
    #[must_use]
    pub fn new(project_root: Option<&Path>) -> Self {
        let data_home = AzothDataHome::resolve();
        if let Err(error) = data_home.prepare() {
            warn!(%error, "failed to migrate global editor preferences");
        }
        let project = project_root.and_then(|root| project_data_paths(&data_home, root));
        Self::from_data_home(&data_home, project.as_ref())
    }

    #[must_use]
    fn from_data_home(data_home: &AzothDataHome, project_paths: Option<&ProjectDataPaths>) -> Self {
        let global = data_home.editor_preferences_path();
        let project = project_paths.map(ProjectDataPaths::editor_settings_path);

        Self { global, project }
    }
}

/// Display-name label for machine-local editor settings when the project
/// manifest is not loaded yet.
///
/// [`AzothDataHome::project`] still isolates by Lore instance id at the project
/// root (or `unlinked-<path-hash>` before Lore exists). This constant only
/// supplies the human `<project-name>` path segment.
const EDITOR_PROJECT_STATE_KEY: &str = "project";

fn project_data_paths(data_home: &AzothDataHome, project_root: &Path) -> Option<ProjectDataPaths> {
    let paths = data_home.project(EDITOR_PROJECT_STATE_KEY, project_root);
    if let Err(error) = paths.prepare() {
        warn!(%error, root = %project_root.display(), "failed to prepare project editor settings directory");
        return None;
    }
    Some(paths)
}

#[must_use]
pub fn load_settings(project_root: Option<&Path>) -> EditorSettings {
    load_settings_from_paths(&SettingsPaths::new(project_root))
}

#[must_use]
pub fn load_settings_store(project_root: Option<&Path>) -> SettingsStore {
    SettingsStore::new(load_settings(project_root))
}

/// Writes `settings` to `path` as TOML, creating the parent directory first.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidData`] if `settings` cannot be serialized to
/// TOML, or any [`io::Error`] from creating the parent directory or writing the
/// file (for example a missing permission or a read-only volume).
pub fn save_settings(path: &Path, settings: &EditorSettings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let text = toml::to_string_pretty(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, text)
}

/// Writes `settings` to the per-user global settings path.
///
/// # Errors
///
/// Returns any error [`save_settings`] returns for the global path.
pub fn save_global_settings(settings: &EditorSettings) -> io::Result<()> {
    let paths = SettingsPaths::new(None);
    save_settings(&paths.global, settings)
}

/// Writes `settings` to the project-local settings path under `project_root`.
///
/// # Errors
///
/// Returns any error [`save_settings`] returns for the project path. Succeeds
/// without writing anything when `project_root` resolves no project settings
/// path.
pub fn save_project_settings(project_root: &Path, settings: &EditorSettings) -> io::Result<()> {
    let paths = SettingsPaths::new(Some(project_root));
    if let Some(project_path) = paths.project {
        return save_settings(&project_path, settings);
    }
    Ok(())
}

fn load_settings_from_paths(paths: &SettingsPaths) -> EditorSettings {
    let mut settings = EditorSettings::default();
    if let Some(global_settings) = read_toml_patch(&paths.global) {
        global_settings.apply_to(&mut settings);
    }
    if let Some(project_path) = &paths.project
        && let Some(project_settings) = read_toml_patch(project_path)
    {
        project_settings.apply_to(&mut settings);
    }
    settings
}

fn read_toml_patch(path: &Path) -> Option<EditorSettingsPatch> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EditorSettingsPatch {
    theme: Option<String>,
    keymap_profile: Option<String>,
    window_maximized: Option<bool>,
    source_navigation: Option<SourceNavigationSettingsPatch>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SourceNavigationSettingsPatch {
    file_url_template: Option<String>,
}

impl EditorSettingsPatch {
    fn apply_to(self, settings: &mut EditorSettings) {
        if let Some(theme) = self.theme {
            settings.theme = theme;
        }
        if let Some(keymap_profile) = self.keymap_profile {
            settings.keymap_profile = keymap_profile;
        }
        if let Some(window_maximized) = self.window_maximized {
            settings.window_maximized = window_maximized;
        }
        if let Some(source_navigation) = self.source_navigation {
            source_navigation.apply_to(settings);
        }
    }
}

impl SourceNavigationSettingsPatch {
    fn apply_to(self, settings: &mut EditorSettings) {
        if let Some(file_url_template) = self.file_url_template {
            settings.source_navigation.file_url_template = file_url_template;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loads_global_settings_with_project_override() {
        let home = TempDir::new().expect("create home tempdir");
        let project = TempDir::new().expect("create project tempdir");
        let data_home = AzothDataHome::new(home.path());
        let project_paths = data_home.project("local.settings_test", project.path());
        let paths = SettingsPaths::from_data_home(&data_home, Some(&project_paths));
        fs::create_dir_all(paths.global.parent().expect("global parent"))
            .expect("create global settings dir");
        fs::create_dir_all(
            paths
                .project
                .as_ref()
                .and_then(|path| path.parent())
                .expect("project parent"),
        )
        .expect("create project settings dir");
        fs::write(
            &paths.global,
            r#"
theme = "dark"
keymap_profile = "vim"
window_maximized = true
"#,
        )
        .expect("write global settings");
        fs::write(
            paths.project.as_ref().expect("project settings path"),
            r#"
theme = "light"
"#,
        )
        .expect("write project settings");

        let settings = load_settings_from_paths(&paths);

        assert_eq!(settings.theme, "light");
        assert_eq!(settings.keymap_profile, "vim");
        assert!(settings.window_maximized);
    }

    #[test]
    fn saves_settings_and_creates_parent_directory() {
        let root = TempDir::new().expect("create tempdir");
        let path = root.path().join("nested").join("settings.toml");
        let settings = EditorSettings {
            theme: "dark".to_string(),
            keymap_profile: "emacs".to_string(),
            window_maximized: true,
            source_navigation: SourceNavigationSettings::default(),
        };

        save_settings(&path, &settings).expect("save settings");

        let loaded = fs::read_to_string(path).expect("read saved settings");
        assert!(loaded.contains(r#"theme = "dark""#));
        assert!(loaded.contains(r#"keymap_profile = "emacs""#));
        assert!(loaded.contains("window_maximized = true"));
    }

    #[test]
    fn settings_store_wraps_editor_loaded_settings() {
        let home = TempDir::new().expect("create home tempdir");
        let data_home = AzothDataHome::new(home.path());
        let paths = SettingsPaths::from_data_home(&data_home, None);
        fs::create_dir_all(paths.global.parent().expect("global parent"))
            .expect("create global settings dir");
        fs::write(&paths.global, r#"theme = "dark""#).expect("write global settings");

        let store = SettingsStore::new(load_settings_from_paths(&paths));

        assert_eq!(store.current.theme, "dark");
    }

    #[test]
    fn loads_source_navigation_settings_patch() {
        let home = TempDir::new().expect("create home tempdir");
        let data_home = AzothDataHome::new(home.path());
        let paths = SettingsPaths::from_data_home(&data_home, None);
        fs::create_dir_all(paths.global.parent().expect("global parent"))
            .expect("create global settings dir");
        fs::write(
            &paths.global,
            r#"
[source_navigation]
file_url_template = "cursor://file/{path}{line_column_suffix}"
"#,
        )
        .expect("write global settings");

        let settings = load_settings_from_paths(&paths);

        assert_eq!(
            settings.source_navigation.file_url_template,
            "cursor://file/{path}{line_column_suffix}"
        );
    }
}
