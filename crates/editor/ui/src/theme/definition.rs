//! Theme definitions, variants, and resolved theme structures.

use std::collections::HashMap;

use gpui::SharedString;
use serde::{Deserialize, Serialize};

use super::color::{Color, ThemeConfigKey};
use crate::error::Result;

/// Font configuration for themes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThemeFont {
    /// Font size
    #[serde(default)]
    pub size: Option<Length>,
    /// Font family
    #[serde(default)]
    pub family: Option<SharedString>,
    /// Monospace font size.
    #[serde(default)]
    pub mono_size: Option<Length>,
    /// Monospace font family.
    #[serde(default)]
    pub mono_family: Option<SharedString>,
}

impl ThemeFont {
    #[must_use]
    pub const fn is_authored(&self) -> bool {
        self.size.is_some()
            || self.family.is_some()
            || self.mono_size.is_some()
            || self.mono_family.is_some()
    }

    pub(crate) fn merge_from(&mut self, higher_precedence: &Self) {
        if higher_precedence.size.is_some() {
            self.size = higher_precedence.size;
        }
        if higher_precedence.family.is_some() {
            self.family.clone_from(&higher_precedence.family);
        }
        if higher_precedence.mono_size.is_some() {
            self.mono_size = higher_precedence.mono_size;
        }
        if higher_precedence.mono_family.is_some() {
            self.mono_family.clone_from(&higher_precedence.mono_family);
        }
    }
}

/// A length that can be specified in pixels or rems.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Length {
    /// Length in pixels (bare number defaults to pixels)
    Pixels(f32),
    /// Length in rems (string with "rem" suffix)
    #[serde(deserialize_with = "deserialize_rems")]
    Rems(f32),
}

fn deserialize_rems<'de, D>(deserializer: D) -> std::result::Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let s: String = Deserialize::deserialize(deserializer)?;
    if s.ends_with("rem") {
        let num_str = s.trim_end_matches("rem");
        num_str
            .parse()
            .map_err(|_| D::Error::custom(format!("Invalid rem value: {s}")))
    } else {
        Err(D::Error::custom(format!(
            "Rem value must end with 'rem': {s}"
        )))
    }
}

impl Length {
    /// Create a length from pixels.
    #[must_use]
    pub const fn px(value: f32) -> Self {
        Self::Pixels(value)
    }

    /// Create a length from rems.
    #[must_use]
    pub const fn rem(value: f32) -> Self {
        Self::Rems(value)
    }

    /// Convert to GPUI's `AbsoluteLength`.
    #[must_use]
    pub const fn to_absolute_length(self) -> gpui::AbsoluteLength {
        match self {
            Self::Pixels(px) => gpui::AbsoluteLength::Pixels(gpui::px(px)),
            Self::Rems(rem) => gpui::AbsoluteLength::Rems(gpui::rems(rem)),
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Self::rem(0.0)
    }
}

impl Length {
    /// Convert to GPUI Pixels (assuming 16px root font size for rems)
    #[must_use]
    pub fn to_pixels(self) -> gpui::Pixels {
        match self {
            Self::Pixels(px) => gpui::px(px),
            Self::Rems(rem) => gpui::px(rem * 16.0), // Convert rems to pixels
        }
    }
}

impl From<Length> for gpui::DefiniteLength {
    fn from(length: Length) -> Self {
        Self::Absolute(length.to_absolute_length())
    }
}

impl From<Length> for gpui::AbsoluteLength {
    fn from(length: Length) -> Self {
        length.to_absolute_length()
    }
}

/// A theme definition loaded from TOML.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub name: SharedString,

    /// Global color palette (can be overridden by variants)
    #[serde(default)]
    pub colors: HashMap<String, Color>,

    /// Font configuration
    #[serde(default)]
    pub font: ThemeFont,

    /// Global border radius (can be overridden by variants)
    #[serde(default)]
    pub radius: Option<Length>,

    /// Light variant of the theme
    #[serde(default)]
    pub light: ThemeVariant,

    /// Dark variant of the theme
    #[serde(default)]
    pub dark: ThemeVariant,
}

/// A theme variant (light or dark).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThemeVariant {
    /// Color palette (overrides global colors)
    #[serde(default)]
    pub colors: HashMap<String, Color>,

    /// Font configuration (overrides global font)
    #[serde(default)]
    pub font: ThemeFont,

    /// Border radius (overrides global radius)
    #[serde(default)]
    pub radius: Option<Length>,
}

impl ThemeVariant {
    #[must_use]
    pub fn is_authored(&self) -> bool {
        !self.colors.is_empty() || self.font.is_authored() || self.radius.is_some()
    }
}

// Note: We now use gpui_component::theme::ThemeMode directly instead of our own enum
// since gpui-component handles system appearance detection through WindowAppearance -> ThemeMode conversion

impl ThemeDefinition {
    #[must_use]
    pub fn has_light_variant(&self) -> bool {
        self.light.is_authored() || self.has_global_only_colors()
    }

    #[must_use]
    pub fn has_dark_variant(&self) -> bool {
        self.dark.is_authored() || self.has_global_only_colors()
    }

    fn has_global_only_colors(&self) -> bool {
        !self.colors.is_empty() && !self.light.is_authored() && !self.dark.is_authored()
    }

    /// Load a theme definition from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or its contents fail to
    /// deserialize as a valid theme definition.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml_str(&content)
    }

    /// Load a theme definition from embedded or dynamically supplied TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if `content` is not valid TOML for a theme definition.
    pub fn from_toml_str(content: &str) -> Result<Self> {
        Ok(toml::from_str(content)?)
    }

    /// Save a theme definition to a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the definition cannot be serialized to TOML or the
    /// file cannot be written.
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Convert this theme definition to `gpui_component`'s `ThemeConfig` format.
    ///
    /// # Errors
    ///
    /// Returns an error if a color value in the definition cannot be parsed
    /// into the target `ThemeConfig` representation.
    pub fn to_theme_config(
        &self,
        mode: gpui_component::theme::ThemeMode,
    ) -> Result<gpui_component::theme::ThemeConfig> {
        use gpui::SharedString;
        use gpui_component::theme::{ThemeConfig, ThemeConfigColors, ThemeMode};

        let variant = match mode {
            ThemeMode::Light => &self.light,
            ThemeMode::Dark => &self.dark,
        };

        // Helper to convert our Color to hex string
        let to_hex = |color: &Color| -> String {
            let rgba = color.rgba();
            // Channels are authored in 0.0..=1.0; clamp + round before scaling
            // so the value is always within 0..=255 before the u8 cast.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "value is clamped to 0..=255 and rounded before the cast"
            )]
            let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!(
                "#{:02x}{:02x}{:02x}",
                channel(rgba.r),
                channel(rgba.g),
                channel(rgba.b)
            )
        };

        // Helper to get color from colors with fallback
        let get_color = |key: &str| -> Option<SharedString> {
            variant
                .colors
                .get(key)
                .or_else(|| self.colors.get(key))
                .map(|c| SharedString::from(to_hex(c)))
        };

        // Helper to get color using ThemeConfigKey enum
        let get_theme_color =
            |key: ThemeConfigKey| -> Option<SharedString> { get_color(key.as_ref()) };

        let mut colors = ThemeConfigColors::default();
        for &key in ThemeConfigKey::ALL {
            key.set_vendor_color(&mut colors, get_theme_color(key));
        }

        let font_family = variant
            .font
            .family
            .clone()
            .or_else(|| self.font.family.clone());
        let mono_font_family = variant
            .font
            .mono_family
            .clone()
            .or_else(|| self.font.mono_family.clone())
            .or_else(|| font_family.clone());
        let font_size = variant.font.size.or(self.font.size);
        let mono_font_size = variant.font.mono_size.or(self.font.mono_size).or(font_size);
        let radius = variant.radius.or(self.radius);

        Ok(ThemeConfig {
            is_default: false,
            name: self.name.clone(),
            mode,
            colors,
            highlight: None,
            font_family,
            mono_font_family,
            font_size: font_size.map(|size| size.to_pixels().into()),
            mono_font_size: mono_font_size.map(|size| size.to_pixels().into()),
            radius: radius.map(|radius| radius.to_pixels().into()),
            radius_lg: Option::default(),
            shadow: Some(true),
        })
    }
}
