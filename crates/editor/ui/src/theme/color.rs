//! Color types and parsing for the theme system.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A color in the theme system that can be either RGBA or HSLA.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Color {
    Rgba(gpui::Rgba),
    Hsla(gpui::Hsla),
}

impl Color {
    /// Get the RGBA representation of this color.
    #[must_use]
    pub fn rgba(&self) -> gpui::Rgba {
        match self {
            Self::Rgba(rgba) => *rgba,
            Self::Hsla(hsla) => (*hsla).into(),
        }
    }

    /// Get the HSLA representation of this color.
    #[must_use]
    pub fn hsla(&self) -> gpui::Hsla {
        match self {
            Self::Rgba(rgba) => (*rgba).into(),
            Self::Hsla(hsla) => *hsla,
        }
    }

    /// Parse a color from a string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ColorParse`] if the string is neither a valid hex RGBA
    /// color nor a recognized `hsl()`/`hsla()` function value.
    pub fn parse(s: &str) -> Result<Self> {
        // Try parsing as RGBA first (hex colors using GPUI's TryFrom)
        if let Ok(rgba) = gpui::Rgba::try_from(s) {
            return Ok(Self::Rgba(rgba));
        }

        // Try parsing as HSLA function using GPUI's hsla() helper
        if let Some(hsla) = parse_hsla_with_gpui_helper(s) {
            return Ok(Self::Hsla(hsla));
        }

        Err(Error::ColorParse(format!("Invalid color format: {s}")))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Parse HSLA color from strings like "hsl(360, 100%, 50%)" or "hsla(360, 100%, 50%, 1.0)".
/// Uses GPUI's `hsla()` helper function.
fn parse_hsla_with_gpui_helper(s: &str) -> Option<gpui::Hsla> {
    let s = s.trim();

    // Check if it starts with hsl( or hsla(
    let (func_name, content) = if let Some(content) = s.strip_prefix("hsl(") {
        ("hsl", content)
    } else {
        let content = s.strip_prefix("hsla(")?;
        ("hsla", content)
    };

    // Check if it ends with )
    if !content.ends_with(')') {
        return None;
    }

    let content = &content[..content.len() - 1]; // Remove closing )

    // Split by comma
    let parts: Vec<&str> = content.split(',').map(str::trim).collect();

    // hsl() needs 3 parts, hsla() needs 4 parts
    let expected_parts = if func_name == "hsl" { 3 } else { 4 };
    if parts.len() != expected_parts {
        return None;
    }

    // Parse hue (0-360)
    let h = parse_number(parts[0])? / 360.0;

    // Parse saturation (0-100%)
    let s = if parts[1].ends_with('%') {
        parse_number(&parts[1][..parts[1].len() - 1])? / 100.0
    } else {
        parse_number(parts[1])?
    };

    // Parse lightness (0-100%)
    let l = if parts[2].ends_with('%') {
        parse_number(&parts[2][..parts[2].len() - 1])? / 100.0
    } else {
        parse_number(parts[2])?
    };

    // Parse alpha (0-1) - default to 1.0 if not specified
    let a = if parts.len() == 4 {
        parse_number(parts[3])?
    } else {
        1.0
    };

    // Use GPUI's hsla() helper function
    Some(gpui::hsla(h, s, l, a))
}

/// Parse a number from string, handling optional decimal part.
fn parse_number(s: &str) -> Option<f32> {
    s.trim().parse::<f32>().ok()
}

impl From<Color> for gpui::Rgba {
    fn from(color: Color) -> Self {
        color.rgba()
    }
}

impl From<Color> for gpui::Hsla {
    fn from(color: Color) -> Self {
        color.hsla()
    }
}

impl From<Color> for gpui::Fill {
    fn from(color: Color) -> Self {
        Self::from(color.rgba())
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::Rgba(gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }) // White
    }
}

macro_rules! define_theme_config_keys {
    ($( $variant:ident => ($key:literal, $field:ident), )+) => {
        /// Keys exposed by the editor's TOML theme loader.
        ///
        /// The declaration also generates the vendor-field assignment used by
        /// [`ThemeDefinition::to_theme_config`](super::definition::ThemeDefinition::to_theme_config),
        /// keeping the editor key and vendor destination in one place.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ThemeConfigKey {
            $( $variant, )+
        }

        impl ThemeConfigKey {
            /// Every editor theme key handled by the vendor conversion.
            pub const ALL: &'static [Self] = &[$( Self::$variant, )+];

            pub(crate) fn set_vendor_color(
                self,
                colors: &mut gpui_component::theme::ThemeConfigColors,
                value: Option<gpui::SharedString>,
            ) {
                match self {
                    $( Self::$variant => colors.$field = value, )+
                }
            }
        }

        impl AsRef<str> for ThemeConfigKey {
            fn as_ref(&self) -> &str {
                match self {
                    $( Self::$variant => $key, )+
                }
            }
        }
    };
}

define_theme_config_keys! {
    Background => ("background", background),
    Foreground => ("foreground", foreground),
    Border => ("border", border),
    Accent => ("accent", accent),
    AccentForeground => ("accent_foreground", accent_foreground),
    Primary => ("primary", primary),
    PrimaryHover => ("primary_hover", primary_hover),
    PrimaryActive => ("primary_active", primary_active),
    PrimaryForeground => ("primary_foreground", primary_foreground),
    Secondary => ("secondary", secondary),
    SecondaryHover => ("secondary_hover", secondary_hover),
    SecondaryActive => ("secondary_active", secondary_active),
    SecondaryForeground => ("secondary_foreground", secondary_foreground),
    DangerBackground => ("danger", danger),
    DangerHoverBackground => ("danger_hover", danger_hover),
    DangerActiveBackground => ("danger_active", danger_active),
    DangerForeground => ("danger_foreground", danger_foreground),
    SuccessBackground => ("success", success),
    SuccessHoverBackground => ("success_hover", success_hover),
    SuccessActiveBackground => ("success_active", success_active),
    SuccessForeground => ("success_foreground", success_foreground),
    WarningBackground => ("warning", warning),
    WarningHoverBackground => ("warning_hover", warning_hover),
    WarningActiveBackground => ("warning_active", warning_active),
    WarningForeground => ("warning_foreground", warning_foreground),
    InfoBackground => ("info", info),
    InfoHoverBackground => ("info_hover", info_hover),
    InfoActiveBackground => ("info_active", info_active),
    InfoForeground => ("info_foreground", info_foreground),
    Input => ("input", input),
    InputBackground => ("input_background", input_background),
    Caret => ("caret", caret),
    Selection => ("selection", selection),
    Ring => ("ring", ring),
    Muted => ("muted", muted),
    MutedForeground => ("muted_foreground", muted_foreground),
    Skeleton => ("skeleton", skeleton),
    ListBackground => ("list", list),
    ListActive => ("list_active", list_active),
    ListHover => ("list_hover", list_hover),
    ListActiveBorder => ("list_active_border", list_active_border),
    ListEven => ("list_even", list_even),
    ListHead => ("list_head", list_head),
    Tab => ("tab", tab),
    TabActive => ("tab_active", tab_active),
    TabActiveForeground => ("tab_active_foreground", tab_active_foreground),
    TabForeground => ("tab_foreground", tab_foreground),
    TabBar => ("tab_bar", tab_bar),
    TabBarSegmented => ("tab_bar_segmented", tab_bar_segmented),
    Sidebar => ("sidebar", sidebar),
    SidebarForeground => ("sidebar_foreground", sidebar_foreground),
    SidebarBorder => ("sidebar_border", sidebar_border),
    SidebarAccent => ("sidebar_accent", sidebar_accent),
    SidebarAccentForeground => ("sidebar_accent_foreground", sidebar_accent_foreground),
    SidebarPrimary => ("sidebar_primary", sidebar_primary),
    SidebarPrimaryForeground => ("sidebar_primary_foreground", sidebar_primary_foreground),
    TitleBar => ("title_bar", title_bar),
    TitleBarBorder => ("title_bar_border", title_bar_border),
    Link => ("link", link),
    LinkHover => ("link_hover", link_hover),
    LinkActive => ("link_active", link_active),
    Popover => ("popover", popover),
    PopoverForeground => ("popover_foreground", popover_foreground),
    Overlay => ("overlay", overlay),
    DragBorder => ("drag_border", drag_border),
    DropTarget => ("drop_target", drop_target),
    GroupBox => ("group_box", group_box),
    GroupBoxForeground => ("group_box_foreground", group_box_foreground),
    GroupBoxTitleForeground => ("group_box_title_foreground", group_box_title_foreground),
    Accordion => ("accordion", accordion),
    AccordionHover => ("accordion_hover", accordion_hover),
    ButtonPrimary => ("button_primary", button_primary),
    ButtonPrimaryHover => ("button_primary_hover", button_primary_hover),
    ButtonPrimaryActive => ("button_primary_active", button_primary_active),
    ButtonPrimaryForeground => ("button_primary_foreground", button_primary_foreground),
    ProgressBar => ("progress_bar", progress_bar),
    SliderBar => ("slider_bar", slider_bar),
    SliderThumb => ("slider_thumb", slider_thumb),
    Switch => ("switch", switch),
    SwitchThumb => ("switch_thumb", switch_thumb),
    Scrollbar => ("scrollbar", scrollbar),
    ScrollbarThumb => ("scrollbar_thumb", scrollbar_thumb),
    ScrollbarThumbHover => ("scrollbar_thumb_hover", scrollbar_thumb_hover),
    Table => ("table", table),
    TableActive => ("table_active", table_active),
    TableActiveBorder => ("table_active_border", table_active_border),
    TableEven => ("table_even", table_even),
    TableHead => ("table_head", table_head),
    TableHeadForeground => ("table_head_foreground", table_head_foreground),
    TableFoot => ("table_foot", table_foot),
    TableFootForeground => ("table_foot_foreground", table_foot_foreground),
    TableHover => ("table_hover", table_hover),
    TableRowBorder => ("table_row_border", table_row_border),
    Tiles => ("tiles", tiles),
    DescriptionListLabel => ("description_list_label", description_list_label),
    DescriptionListLabelForeground => ("description_list_label_foreground", description_list_label_foreground),
    WindowBorder => ("window_border", window_border),
    Chart1 => ("chart_1", chart_1),
    Chart2 => ("chart_2", chart_2),
    Chart3 => ("chart_3", chart_3),
    Chart4 => ("chart_4", chart_4),
    Chart5 => ("chart_5", chart_5),
    ChartBullish => ("chart_bullish", chart_bullish),
    ChartBearish => ("chart_bearish", chart_bearish),
    TypeAccentNeutral => ("type_accent_neutral", type_accent_neutral),
    TypeAccentSlate => ("type_accent_slate", type_accent_slate),
    TypeAccentLevel => ("type_accent_level", type_accent_level),
    TypeAccentPrefab => ("type_accent_prefab", type_accent_prefab),
    TypeAccentGold => ("type_accent_gold", type_accent_gold),
    TypeAccentLight => ("type_accent_light", type_accent_light),
    TypeAccentTeal => ("type_accent_teal", type_accent_teal),
    TypeAccentTerrain => ("type_accent_terrain", type_accent_terrain),
    TypeAccentAudio => ("type_accent_audio", type_accent_audio),
    TypeAccentAnimation => ("type_accent_animation", type_accent_animation),
}
