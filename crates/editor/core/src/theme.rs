//! Editor-shell theme location ownership.
//!
//! `az-editor-ui` owns theme data and GPUI registration. The standalone
//! editor shell owns platform/user directory resolution so UI components stay
//! detached from host filesystem policy.

use std::path::PathBuf;

use az_editor_ui::theme::ThemeLocation;
use az_filesystem::AzothDataHome;

use crate::EditorResult;
use crate::settings::SettingsStore;

pub fn init(cx: &mut gpui::App) -> EditorResult<()> {
    let locations = vec![ThemeLocation::User(user_theme_dir())];
    az_editor_ui::theme::init_with_locations(cx, &locations)?;
    apply_editor_theme_setting(cx)?;
    watch_user_theme_dir(cx, locations)?;
    Ok(())
}

/// The user theme directory: the `AZOTH_EDITOR_THEME_DIR` override when set,
/// otherwise the platform Azoth data home's themes directory.
fn user_theme_dir() -> PathBuf {
    std::env::var_os("AZOTH_EDITOR_THEME_DIR")
        .map_or_else(|| AzothDataHome::resolve().themes_dir(), PathBuf::from)
}

fn apply_editor_theme_setting(cx: &mut gpui::App) -> EditorResult<()> {
    let setting = if cx.has_global::<SettingsStore>() {
        cx.global::<SettingsStore>().current.theme.clone()
    } else {
        "system".to_string()
    };
    az_editor_ui::theme::apply_theme_setting(cx, &setting, None)?;
    Ok(())
}

fn watch_user_theme_dir(cx: &mut gpui::App, locations: Vec<ThemeLocation>) -> EditorResult<()> {
    let Some(themes_dir) = locations.iter().find_map(|location| match location {
        ThemeLocation::User(path) => Some(path.clone()),
        ThemeLocation::Base(_) | ThemeLocation::Custom(_) => None,
    }) else {
        return Ok(());
    };

    az_editor_ui::ThemeRegistry::watch_dir(themes_dir, cx, move |cx| {
        if let Err(error) = az_editor_ui::theme::load_toml_themes(&locations, cx) {
            eprintln!("Failed to reload themes: {error}");
        }
        if let Err(error) = apply_editor_theme_setting(cx) {
            eprintln!("Failed to apply theme setting after reload: {error}");
        }
    })
    .map_err(|error| az_editor_ui::Error::Io(std::io::Error::other(error.to_string())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_theme_dir_uses_home_azoth_theme_directory() {
        let dir = user_theme_dir();
        if let Some(override_dir) = std::env::var_os("AZOTH_EDITOR_THEME_DIR") {
            assert_eq!(dir, PathBuf::from(override_dir));
        } else {
            assert!(dir.ends_with(PathBuf::from(".azoth").join("themes")));
        }
    }

    #[test]
    fn editor_shell_owns_theme_watcher_registration() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/theme.rs"))
            .expect("read az-editor theme.rs");

        assert!(
            az_architecture_guard::symbols_contain(&source, "ThemeRegistry::watch_dir"),
            "az-editor shell should own user theme hot-reload registration"
        );
        assert!(
            az_architecture_guard::symbols_contain(&source, "load_toml_themes(&locations, cx)"),
            "theme reload should reuse az-editor-ui explicit-location loading"
        );
        assert!(
            az_architecture_guard::symbols_contain(&source, "apply_editor_theme_setting(cx)"),
            "theme init/reload must apply editor settings after loading TOML themes"
        );
    }
}
