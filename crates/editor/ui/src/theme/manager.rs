//! Theme loading and management using `gpui_component`'s `ThemeRegistry`.

use gpui::{Global, SharedString, Window};
use gpui_component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use std::collections::HashMap;
use std::rc::Rc;

use super::definition::ThemeDefinition;
use super::location::ThemeLocation;
use crate::error::Result;

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled_themes.rs"));
}

/// Simple storage for TOML-loaded themes.
/// Since `ThemeRegistry` only loads JSON files, we store our TOML themes here.
#[derive(Default)]
struct TomlThemeStore {
    themes: HashMap<String, TomlThemeEntry>,
}

#[derive(Clone)]
struct TomlThemeEntry {
    light: Rc<ThemeConfig>,
    dark: Rc<ThemeConfig>,
    has_light: bool,
    has_dark: bool,
}

impl Global for TomlThemeStore {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableTheme {
    pub name: String,
    pub has_light: bool,
    pub has_dark: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSelection {
    System,
    BuiltIn(ThemeMode),
    Custom {
        name: String,
        mode: Option<ThemeMode>,
    },
}

/// Load TOML themes from multiple locations with precedence and register them with `ThemeRegistry`.
/// Precedence order: Custom > User > Base/Bundled (higher precedence overwrites lower)
///
/// # Errors
///
/// Returns an error if a theme directory cannot be read.
///
/// # Panics
///
/// Panics if an internal per-name definition group is unexpectedly empty while
/// merging definitions by precedence.
pub fn load_toml_themes(theme_locations: &[ThemeLocation], cx: &mut gpui::App) -> Result<()> {
    if !cx.has_global::<TomlThemeStore>() {
        cx.set_global(TomlThemeStore::default());
    }
    cx.global_mut::<TomlThemeStore>().themes.clear();

    // Group locations by precedence (Custom highest, then User, then Base)
    let mut locations_by_precedence = Vec::new();

    // Add Custom locations (highest precedence)
    for location in theme_locations {
        if let ThemeLocation::Custom(_) = location {
            locations_by_precedence.push((location.clone(), 3)); // Highest precedence
        }
    }

    // Add User location
    for location in theme_locations {
        if let ThemeLocation::User(_) = location {
            locations_by_precedence.push((location.clone(), 2)); // Medium precedence
        }
    }

    // Add Base location (lowest precedence)
    for location in theme_locations {
        if let ThemeLocation::Base(_) = location {
            locations_by_precedence.push((location.clone(), 1)); // Lowest precedence
        }
    }

    // Sort by precedence (highest first)
    locations_by_precedence.sort_by_key(|entry| std::cmp::Reverse(entry.1));

    // Collect all theme definitions by name and precedence
    let mut theme_definitions_by_name: HashMap<String, Vec<(ThemeDefinition, usize)>> =
        HashMap::new();

    for (name, content) in bundled::BUNDLED_THEMES {
        match ThemeDefinition::from_toml_str(content) {
            Ok(definition) => {
                collect_theme_definition(&mut theme_definitions_by_name, definition, 1);
            }
            Err(error) => {
                eprintln!("Failed to load bundled theme {name}: {error}");
            }
        }
    }

    for (location, precedence) in locations_by_precedence {
        let dir = match location.path() {
            Ok(dir) if dir.exists() => dir,
            _ => continue,
        };

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                match ThemeDefinition::from_file(&path) {
                    Ok(definition) => {
                        collect_theme_definition(
                            &mut theme_definitions_by_name,
                            definition,
                            precedence,
                        );
                    }
                    Err(e) => {
                        eprintln!("Failed to load theme from {}: {}", path.display(), e);
                        // Continue loading other themes even if one fails
                    }
                }
            }
        }
    }

    // Merge theme definitions with precedence (higher precedence overrides lower)
    for (theme_name, definitions_with_precedence) in theme_definitions_by_name {
        // Sort by precedence (highest first)
        let mut sorted_definitions = definitions_with_precedence;
        sorted_definitions.sort_by_key(|entry| std::cmp::Reverse(entry.1));

        // Start with the lowest precedence definition
        let mut merged_definition = sorted_definitions.last().unwrap().0.clone();

        // Apply overrides from higher precedence definitions
        for (higher_def, _) in sorted_definitions.into_iter().rev().skip(1) {
            merged_definition = merge_theme_definitions(merged_definition, higher_def);
        }

        // Convert to ThemeConfig for both light and dark variants
        // Store in our TOML theme store
        let light_config = merged_definition.to_theme_config(ThemeMode::Light);
        let dark_config = merged_definition.to_theme_config(ThemeMode::Dark);

        if let (Ok(mut light), Ok(mut dark)) = (light_config, dark_config) {
            light.name = SharedString::from(format!("{theme_name}_light"));
            light.mode = ThemeMode::Light;
            dark.name = SharedString::from(format!("{theme_name}_dark"));
            dark.mode = ThemeMode::Dark;
            let has_light = merged_definition.has_light_variant();
            let has_dark = merged_definition.has_dark_variant();

            // Store in our TOML theme store
            let store = cx.global_mut::<TomlThemeStore>();
            store.themes.insert(
                theme_name.clone(),
                TomlThemeEntry {
                    light: Rc::new(light),
                    dark: Rc::new(dark),
                    has_light,
                    has_dark,
                },
            );
        } else {
            eprintln!("Failed to convert theme '{theme_name}' to ThemeConfig, skipping");
        }
    }

    Ok(())
}

fn collect_theme_definition(
    theme_definitions_by_name: &mut HashMap<String, Vec<(ThemeDefinition, usize)>>,
    definition: ThemeDefinition,
    precedence: usize,
) {
    let key = theme_store_key(&definition.name);
    theme_definitions_by_name
        .entry(key)
        .or_default()
        .push((definition, precedence));
}

/// Merge two theme definitions, where higher precedence overrides lower precedence.
fn merge_theme_definitions(
    mut base: ThemeDefinition,
    override_def: ThemeDefinition,
) -> ThemeDefinition {
    // Override name if provided
    if !override_def.name.is_empty() {
        base.name = override_def.name;
    }

    // Merge global colors (higher precedence overrides)
    for (key, value) in &override_def.colors {
        base.colors.insert(key.clone(), *value);
    }

    base.font.merge_from(&override_def.font);

    // Override global radius if provided
    if override_def.radius.is_some() {
        base.radius = override_def.radius;
    }

    // Merge light variant
    base.light = merge_theme_variants(base.light, &override_def.light);

    // Merge dark variant
    base.dark = merge_theme_variants(base.dark, &override_def.dark);

    base
}

/// Merge two theme variants, where higher precedence overrides lower precedence.
fn merge_theme_variants(
    mut base: super::definition::ThemeVariant,
    override_var: &super::definition::ThemeVariant,
) -> super::definition::ThemeVariant {
    // Merge variant colors (higher precedence overrides)
    for (key, value) in &override_var.colors {
        base.colors.insert(key.clone(), *value);
    }

    base.font.merge_from(&override_var.font);

    // Override variant radius if provided
    if override_var.radius.is_some() {
        base.radius = override_var.radius;
    }

    base
}

/// Initialize the theme system with theme locations in GPUI context.
///
/// # Errors
///
/// Returns an error if loading TOML themes from the given locations fails.
pub fn init_with_locations(cx: &mut gpui::App, theme_locations: &[ThemeLocation]) -> Result<()> {
    if !cx.has_global::<ThemeRegistry>() || !cx.has_global::<Theme>() {
        gpui_component::theme::init(cx);
    }

    // Initialize our TOML theme store
    cx.set_global(TomlThemeStore::default());

    // Load TOML themes
    load_toml_themes(theme_locations, cx)?;

    Ok(())
}

/// Initialize the theme system without filesystem-backed TOML locations.
///
/// The editor shell owns platform directory discovery and should call
/// `init_with_locations` with explicit paths when it wants user or bundled
/// themes.
///
/// # Errors
///
/// Returns an error if theme system initialization fails.
pub fn init(cx: &mut gpui::App) -> Result<()> {
    init_with_locations(cx, &[])
}

/// Initialize the theme system with custom directories in GPUI context.
///
/// # Errors
///
/// Returns an error if loading themes from the given directories fails.
pub fn init_with_dirs(cx: &mut gpui::App, theme_dirs: Vec<std::path::PathBuf>) -> Result<()> {
    let locations: Vec<ThemeLocation> = theme_dirs.into_iter().map(ThemeLocation::Custom).collect();
    init_with_locations(cx, &locations)
}

/// Set the current theme in GPUI context.
/// This uses `gpui_component`'s `Theme::change()` directly.
///
/// # Errors
///
/// Returns [`crate::error::Error::ThemeNotFound`] if no theme with the given
/// name has been loaded into the store.
pub fn set_theme(
    cx: &mut gpui::App,
    name: &str,
    mode: ThemeMode,
    window: Option<&mut Window>,
) -> Result<()> {
    let theme_name = theme_store_key(name);
    // Get theme configs from store (clone to avoid borrow issues)
    let configs = cx
        .global::<TomlThemeStore>()
        .themes
        .get(&theme_name)
        .map(|entry| (entry.light.clone(), entry.dark.clone()));

    let Some((light_config, dark_config)) = configs else {
        return Err(crate::error::Error::ThemeNotFound(name.to_string()));
    };

    let theme = Theme::global_mut(cx);
    theme.light_theme = light_config;
    theme.dark_theme = dark_config;
    Theme::change(mode, window, cx);
    cx.refresh_windows();
    Ok(())
}

/// Apply a theme setting string, resolving system/built-in/custom selections.
///
/// # Errors
///
/// Returns an error if the setting cannot be parsed or the referenced custom
/// theme cannot be applied.
pub fn apply_theme_setting(
    cx: &mut gpui::App,
    setting: &str,
    window: Option<&mut Window>,
) -> Result<()> {
    match parse_theme_setting(setting)? {
        ThemeSelection::System => {
            Theme::sync_system_appearance(window, cx);
            cx.refresh_windows();
            Ok(())
        }
        ThemeSelection::BuiltIn(mode) => {
            Theme::change(mode, window, cx);
            cx.refresh_windows();
            Ok(())
        }
        ThemeSelection::Custom { name, mode } => {
            let mode = mode.unwrap_or_else(|| {
                window
                    .as_ref()
                    .map_or_else(|| cx.window_appearance(), |window| window.appearance())
                    .into()
            });
            set_theme(cx, &name, mode, window)
        }
    }
}

/// Parse a theme setting string into a [`ThemeSelection`].
///
/// # Errors
///
/// Returns an error if the setting names a custom theme with an empty name or
/// is otherwise invalid.
pub fn parse_theme_setting(setting: &str) -> Result<ThemeSelection> {
    let setting = setting.trim();
    if setting.is_empty() || setting.eq_ignore_ascii_case("system") {
        return Ok(ThemeSelection::System);
    }

    let normalized = theme_store_key(setting);
    match normalized.as_str() {
        "light" | "defaultlight" => return Ok(ThemeSelection::BuiltIn(ThemeMode::Light)),
        "dark" | "defaultdark" => return Ok(ThemeSelection::BuiltIn(ThemeMode::Dark)),
        _ => {}
    }

    for separator in [':', '/', '@'] {
        if let Some((name, mode)) = setting.rsplit_once(separator)
            && let Some(mode) = parse_theme_mode(mode.trim())
        {
            let name = name.trim();
            if name.is_empty() {
                return Err(crate::error::Error::InvalidThemeSetting(
                    setting.to_string(),
                ));
            }
            return Ok(ThemeSelection::Custom {
                name: name.to_string(),
                mode: Some(mode),
            });
        }
    }

    Ok(ThemeSelection::Custom {
        name: setting.to_string(),
        mode: None,
    })
}

fn parse_theme_mode(value: &str) -> Option<ThemeMode> {
    match theme_store_key(value).as_str() {
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        _ => None,
    }
}

fn theme_store_key(name: &str) -> String {
    let key = name
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if key.is_empty() {
        name.trim().to_lowercase()
    } else {
        key
    }
}

/// Get all available theme names from GPUI context.
pub fn available_theme_names(cx: &gpui::App) -> Vec<String> {
    cx.global::<TomlThemeStore>()
        .themes
        .keys()
        .cloned()
        .collect()
}

/// Get all available TOML themes and the variants each theme actually authored.
pub fn available_themes(cx: &gpui::App) -> Vec<AvailableTheme> {
    cx.global::<TomlThemeStore>()
        .themes
        .iter()
        .map(|(name, entry)| AvailableTheme {
            name: name.clone(),
            has_light: entry.has_light,
            has_dark: entry.has_dark,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fmt::Write as _};

    use super::*;
    use crate::theme::ThemeConfigKey;

    #[test]
    fn theme_settings_support_system_builtin_and_custom_variants() {
        assert_eq!(
            parse_theme_setting("system").unwrap(),
            ThemeSelection::System
        );
        assert_eq!(
            parse_theme_setting("default-dark").unwrap(),
            ThemeSelection::BuiltIn(ThemeMode::Dark)
        );
        assert_eq!(
            parse_theme_setting("One Dark:light").unwrap(),
            ThemeSelection::Custom {
                name: "One Dark".to_string(),
                mode: Some(ThemeMode::Light),
            }
        );
        assert_eq!(
            parse_theme_setting("onedark").unwrap(),
            ThemeSelection::Custom {
                name: "onedark".to_string(),
                mode: None,
            }
        );
    }

    #[test]
    fn theme_store_key_matches_human_names_and_file_stems() {
        assert_eq!(theme_store_key("One Dark"), "onedark");
        assert_eq!(theme_store_key("one-dark"), "onedark");
        assert_eq!(theme_store_key("one_dark"), "onedark");
    }

    #[test]
    fn bundled_theme_sources_are_embedded_and_parseable() {
        assert!(
            !bundled::BUNDLED_THEMES.is_empty(),
            "repo resources/themes TOML files should be embedded into az-editor-ui"
        );
        for (name, content) in bundled::BUNDLED_THEMES {
            ThemeDefinition::from_toml_str(content)
                .unwrap_or_else(|error| panic!("bundled theme {name} must parse: {error}"));
        }
    }

    #[test]
    fn theme_definition_reports_only_authored_variants() {
        let dark_only = ThemeDefinition::from_toml_str(
            r##"
            name = "Dark Only"

            [dark.colors]
            background = "#000000"
            foreground = "#ffffff"
            "##,
        )
        .unwrap();

        assert!(!dark_only.has_light_variant());
        assert!(dark_only.has_dark_variant());

        let global_only = ThemeDefinition::from_toml_str(
            r##"
            name = "Global"

            [colors]
            background = "#000000"
            foreground = "#ffffff"
            "##,
        )
        .unwrap();

        assert!(global_only.has_light_variant());
        assert!(global_only.has_dark_variant());
    }

    #[test]
    fn theme_precedence_merges_every_global_and_variant_font_field() {
        let base = ThemeDefinition::from_toml_str(
            r#"
            name = "Layered"

            [font]
            size = 12
            family = "Base UI"
            mono_size = 11
            mono_family = "Base Mono"

            [dark.font]
            size = 13
            family = "Base Dark UI"
            mono_size = 10
            mono_family = "Base Dark Mono"
            "#,
        )
        .unwrap();
        let higher_precedence = ThemeDefinition::from_toml_str(
            r#"
            name = "Layered"

            [font]
            family = "Override UI"
            mono_size = 14
            mono_family = "Override Mono"

            [dark.font]
            family = "Override Dark UI"
            mono_size = 15
            mono_family = "Override Dark Mono"
            "#,
        )
        .unwrap();

        let merged = merge_theme_definitions(base, higher_precedence);

        assert_eq!(
            merged.font.size,
            Some(super::super::definition::Length::px(12.0))
        );
        assert_eq!(merged.font.family.as_deref(), Some("Override UI"));
        assert_eq!(
            merged.font.mono_size,
            Some(super::super::definition::Length::px(14.0))
        );
        assert_eq!(merged.font.mono_family.as_deref(), Some("Override Mono"));
        assert_eq!(
            merged.dark.font.size,
            Some(super::super::definition::Length::px(13.0))
        );
        assert_eq!(merged.dark.font.family.as_deref(), Some("Override Dark UI"));
        assert_eq!(
            merged.dark.font.mono_size,
            Some(super::super::definition::Length::px(15.0))
        );
        assert_eq!(
            merged.dark.font.mono_family.as_deref(),
            Some("Override Dark Mono")
        );

        let config = merged.to_theme_config(ThemeMode::Dark).unwrap();
        assert_eq!(config.font_size, Some(13.0));
        assert_eq!(config.font_family.as_deref(), Some("Override Dark UI"));
        assert_eq!(config.mono_font_size, Some(15.0));
        assert_eq!(
            config.mono_font_family.as_deref(),
            Some("Override Dark Mono")
        );
    }

    #[test]
    fn group_box_title_foreground_is_exposed_and_applied() {
        let definition = ThemeDefinition::from_toml_str(
            r##"
            name = "Group Box Token"

            [colors]
            group_box_title_foreground = "#123456"
            "##,
        )
        .unwrap();

        let config = definition.to_theme_config(ThemeMode::Dark).unwrap();

        assert_eq!(
            config.colors.group_box_title_foreground.as_deref(),
            Some("#123456")
        );
    }

    #[test]
    fn theme_config_key_mapping_covers_every_vendor_color_field() {
        // gpui-component keeps its base fallback palette private; those
        // serialized fields cannot be assigned through ThemeConfigColors' API.
        // Account for them explicitly so any *new* vendor field still fails.
        const VENDOR_INTERNAL_FIELDS: [&str; 12] = [
            "base.blue",
            "base.blue.light",
            "base.cyan",
            "base.cyan.light",
            "base.green",
            "base.green.light",
            "base.magenta",
            "base.magenta.light",
            "base.red",
            "base.red.light",
            "base.yellow",
            "base.yellow.light",
        ];
        let mut source = String::from("name = \"Mapping Coverage\"\n\n[colors]\n");
        let mut unique_keys = BTreeSet::new();
        for key in ThemeConfigKey::ALL {
            assert!(
                unique_keys.insert(key.as_ref()),
                "duplicate editor theme key: {}",
                key.as_ref()
            );
            writeln!(&mut source, "{} = \"#123456\"", key.as_ref()).unwrap();
        }

        let definition = ThemeDefinition::from_toml_str(&source).unwrap();
        let config = definition.to_theme_config(ThemeMode::Dark).unwrap();
        let serialized = serde_json::to_value(&config.colors).unwrap();
        let vendor_fields = serialized.as_object().unwrap();
        let unset_fields = vendor_fields
            .iter()
            .filter_map(|(field, value)| value.is_null().then_some(field.as_str()))
            .collect::<BTreeSet<_>>();
        let vendor_internal_fields = VENDOR_INTERNAL_FIELDS.into_iter().collect::<BTreeSet<_>>();

        assert_eq!(
            unset_fields, vendor_internal_fields,
            "vendor color fields are not fully accounted for by editor mappings"
        );
        assert_eq!(
            vendor_fields.len(),
            ThemeConfigKey::ALL.len() + VENDOR_INTERNAL_FIELDS.len(),
            "vendor/editor color schema field count drifted"
        );
    }
}
