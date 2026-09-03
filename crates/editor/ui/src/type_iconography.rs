//! Shared semantic type iconography for editor trees and asset surfaces.
//!
//! Callers resolve authored/catalog data to an [`EditorTypeKind`]; this module
//! is the single place that chooses both the SVG and the semantic theme token.

use gpui::Hsla;
use gpui_component::{IconName, theme::Theme};

/// Semantic kinds presented by the editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EditorTypeKind {
    Level,
    Group,
    Mesh,
    Prefab,
    Slice,
    Light,
    Sun,
    Camera,
    Volume,
    Terrain,
    Empty,
    Ui,
    Fog,
    Probe,
    Material,
    Texture,
    Script,
    Audio,
    Animation,
    #[default]
    Source,
}

/// Asset kinds in the design's stable Asset Browser presentation order.
/// `Source` and non-asset scene/entity kinds are intentionally excluded.
pub const ASSET_CATEGORY_KINDS: &[EditorTypeKind] = &[
    EditorTypeKind::Mesh,
    EditorTypeKind::Material,
    EditorTypeKind::Texture,
    EditorTypeKind::Prefab,
    EditorTypeKind::Script,
    EditorTypeKind::Audio,
    EditorTypeKind::Level,
    EditorTypeKind::Animation,
];

impl EditorTypeKind {
    /// Stable category label used by asset-oriented editor surfaces.
    #[must_use]
    pub const fn asset_category_label(self) -> Option<&'static str> {
        match self {
            Self::Mesh => Some("Meshes"),
            Self::Material => Some("Materials"),
            Self::Texture => Some("Textures"),
            Self::Prefab => Some("Prefabs"),
            Self::Script => Some("Scripts"),
            Self::Audio => Some("Audio"),
            Self::Level => Some("Levels"),
            Self::Animation => Some("Animations"),
            _ => None,
        }
    }

    /// Conventional leading authoring folders that are redundant once the
    /// asset is already displayed beneath its resolved type category.
    #[must_use]
    pub const fn asset_route_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Mesh => &["meshes"],
            Self::Material => &["materials"],
            Self::Texture => &["textures"],
            Self::Prefab => &["prefabs"],
            Self::Script => &["scripts"],
            Self::Audio => &["audio"],
            Self::Level => &["levels", "scenes"],
            Self::Animation => &["animations"],
            _ => &[],
        }
    }

    /// Closest checked-in SVG to the design's Material Symbol glyph.
    #[must_use]
    pub const fn icon(self) -> IconName {
        match self {
            Self::Level => IconName::Globe,
            Self::Group => IconName::FolderClosed,
            Self::Mesh => IconName::Box,
            Self::Prefab => IconName::Boxes,
            Self::Slice => IconName::Database,
            Self::Light => IconName::Lightbulb,
            Self::Sun => IconName::Sun,
            Self::Camera => IconName::Video,
            Self::Volume => IconName::Scan,
            Self::Terrain => IconName::Mountain,
            Self::Empty => IconName::Flag,
            Self::Ui => IconName::LayoutDashboard,
            Self::Fog => IconName::Cloud,
            Self::Probe => IconName::Circle,
            Self::Material => IconName::Blend,
            Self::Texture => IconName::Image,
            Self::Script => IconName::Code,
            Self::Audio => IconName::AudioWaveform,
            Self::Animation => IconName::Film,
            Self::Source => IconName::File,
        }
    }

    /// Semantic, theme-authored tint shared by every surface.
    #[must_use]
    pub fn tint(self, theme: &Theme) -> Hsla {
        match self {
            Self::Level => theme.type_accent_level,
            Self::Group | Self::Empty | Self::Source => theme.type_accent_neutral,
            Self::Mesh | Self::Camera | Self::Fog => theme.type_accent_slate,
            Self::Prefab | Self::Ui | Self::Probe => theme.type_accent_prefab,
            Self::Slice | Self::Sun | Self::Material => theme.type_accent_gold,
            Self::Light => theme.type_accent_light,
            Self::Volume | Self::Script => theme.type_accent_teal,
            Self::Terrain | Self::Texture => theme.type_accent_terrain,
            Self::Audio => theme.type_accent_audio,
            Self::Animation => theme.type_accent_animation,
        }
    }

    #[must_use]
    pub const fn tag(self) -> Option<&'static str> {
        match self {
            Self::Level => Some("Level"),
            Self::Group => Some("Group"),
            Self::Prefab => Some("Prefab"),
            _ => None,
        }
    }
}

/// Exact component schema-to-kind registrations, ordered by presentation
/// priority.
///
/// New component families join the outliner by adding their stable schema id
/// here; entity names and source file extensions never participate.
pub const COMPONENT_TYPE_KIND_MAP: &[(&str, EditorTypeKind)] = &[
    ("azoth.camera.Camera", EditorTypeKind::Camera),
    ("azoth.render.Camera", EditorTypeKind::Camera),
    ("azoth.light.DirectionalLight", EditorTypeKind::Sun),
    ("azoth.render.DirectionalLight", EditorTypeKind::Sun),
    ("azoth.render.PointLight", EditorTypeKind::Light),
    ("azoth.render.SpotLight", EditorTypeKind::Light),
    ("azoth.light.Light", EditorTypeKind::Light),
    ("azoth.render.Light", EditorTypeKind::Light),
    ("azoth.terrain.Terrain", EditorTypeKind::Terrain),
    ("azoth.render.Fog", EditorTypeKind::Fog),
    ("azoth.render.ReflectionProbe", EditorTypeKind::Probe),
    ("azoth.render.Volume", EditorTypeKind::Volume),
    ("azoth.ui.Canvas", EditorTypeKind::Ui),
    ("azoth.render.Mesh", EditorTypeKind::Mesh),
];

/// Resolve an entity from authored structure and its component schema ids.
#[must_use]
pub fn entity_kind<'a>(
    schema_type: &str,
    component_schema_types: impl IntoIterator<Item = &'a str>,
    has_children: bool,
) -> EditorTypeKind {
    if schema_type == "azoth.prefab.Instance" {
        return EditorTypeKind::Prefab;
    }
    if schema_type == "azoth.prefab.Prefab" {
        return EditorTypeKind::Level;
    }

    let mut resolved = None;
    for component_type in component_schema_types {
        let Some((priority, kind)) = COMPONENT_TYPE_KIND_MAP.iter().enumerate().find_map(
            |(priority, (registered_type, kind))| {
                (component_type == *registered_type).then_some((priority, *kind))
            },
        ) else {
            continue;
        };
        if resolved.is_none_or(|(best_priority, _)| priority < best_priority) {
            resolved = Some((priority, kind));
        }
    }
    if let Some((_, kind)) = resolved {
        return kind;
    }

    if has_children {
        EditorTypeKind::Group
    } else {
        EditorTypeKind::Empty
    }
}

/// Resolve catalog-provided source/product type metadata to presentation.
/// This deliberately accepts type metadata only: callers must not pass a file
/// extension synthesized from the source path.
#[must_use]
pub fn asset_kind(raw_type: &str, label: &str) -> EditorTypeKind {
    let matches = |needle| {
        contains_ascii_case_insensitive(raw_type, needle)
            || contains_ascii_case_insensitive(label, needle)
    };

    if matches("material") || matches("azmat") {
        EditorTypeKind::Material
    } else if matches("texture") || matches("image") {
        EditorTypeKind::Texture
    } else if matches("prefab") || matches("slice") {
        EditorTypeKind::Prefab
    } else if matches("mesh") || matches("model") || matches("gltf") {
        EditorTypeKind::Mesh
    } else if matches("scene") || matches("level") {
        EditorTypeKind::Level
    } else if matches("script") || matches("code") {
        EditorTypeKind::Script
    } else if matches("audio") || matches("sound") {
        EditorTypeKind::Audio
    } else if matches("anim") || matches("motion") {
        EditorTypeKind::Animation
    } else {
        EditorTypeKind::Source
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    !needle.is_empty()
        && haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_types_resolve_without_source_extensions() {
        assert_eq!(
            asset_kind("azoth.material.Material", "Material"),
            EditorTypeKind::Material
        );
        assert_eq!(
            asset_kind("azoth.prefab.Prefab", "Prefab"),
            EditorTypeKind::Prefab
        );
        assert_eq!(
            asset_kind("azoth.mesh.SourceModel", "Mesh Source"),
            EditorTypeKind::Mesh
        );
    }

    #[test]
    fn asset_categories_keep_design_order_and_labels() {
        assert_eq!(
            ASSET_CATEGORY_KINDS
                .iter()
                .filter_map(|kind| kind.asset_category_label())
                .collect::<Vec<_>>(),
            vec![
                "Meshes",
                "Materials",
                "Textures",
                "Prefabs",
                "Scripts",
                "Audio",
                "Levels",
                "Animations",
            ]
        );
    }

    #[test]
    fn entity_kind_is_driven_by_component_schema_ids() {
        assert_eq!(
            entity_kind("azoth.prefab.Entity", ["azoth.render.Mesh"], false),
            EditorTypeKind::Mesh
        );
        assert_eq!(
            entity_kind("azoth.prefab.Entity", ["azoth.transform.Transform"], true),
            EditorTypeKind::Group
        );
        assert_eq!(
            entity_kind("azoth.prefab.Entity", ["azoth.transform.Transform"], false),
            EditorTypeKind::Empty
        );
        assert_eq!(
            entity_kind("azoth.prefab.Entity", ["azoth.render.Camera"], false),
            EditorTypeKind::Camera
        );
        assert_eq!(
            entity_kind(
                "azoth.prefab.Entity",
                ["azoth.render.DirectionalLight"],
                false
            ),
            EditorTypeKind::Sun
        );
        for schema in ["azoth.render.PointLight", "azoth.render.SpotLight"] {
            assert_eq!(
                entity_kind("azoth.prefab.Entity", [schema], false),
                EditorTypeKind::Light
            );
        }
    }
}
