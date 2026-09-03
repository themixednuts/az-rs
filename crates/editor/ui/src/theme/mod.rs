//! Theme system for the `AZoth` Editor.
//!
//! Provides a TOML-based theming system that uses gpui-component's Theme:
//! - Uses `gpui_component::Theme` (already implements Global trait)
//! - Loads explicit TOML theme locations with light/dark variants
//! - Maps TOML color keys to `ThemeColor` fields
//! - Lets the editor shell own directory discovery and hot-reload watchers
//! - Fully compatible with gpui-component ecosystem

mod color;
mod definition;
mod location;
mod manager;

// Re-export our color key types for type-safe theme access
pub use color::{Color, ThemeConfigKey};

// Re-export our definition types for TOML loading
pub use definition::{Length, ThemeDefinition, ThemeVariant};

// Re-export location and manager
pub use location::ThemeLocation;
pub use manager::{
    AvailableTheme, ThemeSelection, apply_theme_setting, available_theme_names, available_themes,
    init, init_with_dirs, init_with_locations, load_toml_themes, parse_theme_setting, set_theme,
};

// Re-export gpui-component theme types for integration
pub use gpui_component::theme::{
    ActiveTheme, Theme, ThemeColor, ThemeConfig, ThemeMode, ThemeRegistry,
};
