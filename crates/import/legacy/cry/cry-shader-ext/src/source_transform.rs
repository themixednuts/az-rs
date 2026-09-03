//! Source transform for Cry/Lumberyard shader generation extension files.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    normalize_source_path, source_schema_type,
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    ParseError, PreprocessorDirective, PreprocessorDirectiveKind, ShaderDependencyFlags,
    ShaderExtension, ShaderItem, ShaderProperty, ShaderPropertyFlags,
};

pub const SHADER_PRESET_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<ShaderPresetSourceFormat>();

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.ShaderPresetSource",
    ext = "shaderpreset.ron"
)]
pub struct ShaderPresetSourceFormat;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderPresetSource {
    pub source_path: String,
    pub items: Vec<ShaderPresetItemSource>,
}

impl ShaderPresetSource {
    /// Parses a legacy `Shaders/*.ext` script into the authoring source model.
    ///
    /// # Errors
    ///
    /// Returns any error [`ShaderExtension::parse`] returns — [`ParseError::Utf8`]
    /// for a non-UTF-8 payload, [`ParseError::UnknownToken`] for a token the
    /// grammar does not accept, [`ParseError::MissingPropertyName`] for a
    /// `Property` block without a `Name`, and the remaining grammar variants.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, ParseError> {
        let extension = ShaderExtension::parse(bytes)?;
        Ok(Self::from_extension(source_path, &extension))
    }

    #[must_use]
    pub fn from_extension(source_path: &str, extension: &ShaderExtension<'_>) -> Self {
        Self {
            source_path: normalize_source_path(source_path),
            items: extension
                .items()
                .iter()
                .map(ShaderPresetItemSource::from_item)
                .collect(),
        }
    }

    /// Serializes the authoring source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Deserializes the authoring source model from RON bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`ron::error::SpannedError`] when `bytes` is not valid RON or
    /// does not match the [`ShaderPresetSource`] shape.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderPresetItemSource {
    Version(String),
    UsesCommonGlobalFlags,
    Directive(ShaderDirectiveSource),
    Property(ShaderPropertySource),
}

impl ShaderPresetItemSource {
    #[must_use]
    pub fn from_item(item: &ShaderItem<'_>) -> Self {
        match item {
            ShaderItem::Version(version) => Self::Version((*version).to_owned()),
            ShaderItem::UsesCommonGlobalFlags => Self::UsesCommonGlobalFlags,
            ShaderItem::Directive(directive) => {
                Self::Directive(ShaderDirectiveSource::from_directive(*directive))
            }
            ShaderItem::Property(property) => {
                Self::Property(ShaderPropertySource::from_property(property))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderDirectiveSource {
    pub kind: ShaderDirectiveKindSource,
    pub condition: Option<String>,
}

impl ShaderDirectiveSource {
    #[must_use]
    pub fn from_directive(directive: PreprocessorDirective<'_>) -> Self {
        Self {
            kind: directive.kind.into(),
            condition: directive.condition.map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderDirectiveKindSource {
    If,
    Ifdef,
    Ifndef,
    Elif,
    Else,
    Endif,
    Other,
}

impl From<PreprocessorDirectiveKind> for ShaderDirectiveKindSource {
    fn from(value: PreprocessorDirectiveKind) -> Self {
        match value {
            PreprocessorDirectiveKind::If => Self::If,
            PreprocessorDirectiveKind::Ifdef => Self::Ifdef,
            PreprocessorDirectiveKind::Ifndef => Self::Ifndef,
            PreprocessorDirectiveKind::Elif => Self::Elif,
            PreprocessorDirectiveKind::Else => Self::Else,
            PreprocessorDirectiveKind::Endif => Self::Endif,
            PreprocessorDirectiveKind::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderPropertySource {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub mask: u64,
    pub flags: Vec<ShaderPropertyFlagSource>,
    pub precache_names: Vec<String>,
    pub depend_flag_set: Vec<String>,
    pub depend_flag_reset: Vec<String>,
    pub dependency_set: Vec<String>,
    pub dependency_reset: Vec<String>,
}

impl ShaderPropertySource {
    #[must_use]
    pub fn from_property(property: &ShaderProperty<'_>) -> Self {
        Self {
            name: property.name.to_owned(),
            display_name: property.display_name.map(str::to_owned),
            description: property.description.map(str::to_owned),
            mask: property.mask,
            flags: property_flags(property.flags),
            precache_names: strings(&property.precache_names),
            depend_flag_set: strings(&property.depend_flag_set),
            depend_flag_reset: strings(&property.depend_flag_reset),
            dependency_set: dependency_flags(property.dependency_set),
            dependency_reset: dependency_flags(property.dependency_reset),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderPropertyFlagSource {
    Hidden,
    Precache,
    AutoPrecache,
    LowSpecAutoPrecache,
    Runtime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShaderPresetSourceTransform;

impl LegacySourceTransform for ShaderPresetSourceTransform {
    type Error = ShaderPresetSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_shader_preset_source(&input.source_path) {
            return Err(ShaderPresetSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = ShaderPresetSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            shader_preset_source_path(&input.source_path),
            SHADER_PRESET_SOURCE_SCHEMA,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_shader_preset_source(source_path: &str) -> bool {
    let normalized = normalize_source_path(source_path);
    let Some(shader_name) = normalized.strip_prefix("shaders/") else {
        return false;
    };

    let is_ext = std::path::Path::new(shader_name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ext"));
    is_ext && !shader_name.contains('/')
}

#[must_use]
pub fn shader_preset_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".ext").unwrap_or(&normalized);
    format!("{stem}.shaderpreset.ron")
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn property_flags(flags: ShaderPropertyFlags) -> Vec<ShaderPropertyFlagSource> {
    let mut source = Vec::new();
    if flags.contains(ShaderPropertyFlags::HIDDEN) {
        source.push(ShaderPropertyFlagSource::Hidden);
    }
    if flags.contains(ShaderPropertyFlags::PRECACHE) {
        source.push(ShaderPropertyFlagSource::Precache);
    }
    if flags.contains(ShaderPropertyFlags::AUTO_PRECACHE) {
        source.push(ShaderPropertyFlagSource::AutoPrecache);
    }
    if flags.contains(ShaderPropertyFlags::LOW_SPEC_AUTO_PRECACHE) {
        source.push(ShaderPropertyFlagSource::LowSpecAutoPrecache);
    }
    if flags.contains(ShaderPropertyFlags::RUNTIME) {
        source.push(ShaderPropertyFlagSource::Runtime);
    }
    source
}

fn dependency_flags(flags: ShaderDependencyFlags) -> Vec<String> {
    SHADER_DEPENDENCY_FLAG_NAMES
        .iter()
        .filter_map(|(flag, name)| flags.contains(*flag).then_some((*name).to_owned()))
        .collect()
}

const SHADER_DEPENDENCY_FLAG_NAMES: &[(ShaderDependencyFlags, &str)] = &[
    (ShaderDependencyFlags::LM_DIFFUSE, "lm_diffuse"),
    (ShaderDependencyFlags::TEX_DETAIL, "tex_detail"),
    (ShaderDependencyFlags::TEX_NORMALS, "tex_normals"),
    (ShaderDependencyFlags::TEX_ENVCM, "tex_envcm"),
    (ShaderDependencyFlags::TEX_SPECULAR, "tex_specular"),
    (
        ShaderDependencyFlags::TEX_SECOND_SMOOTHNESS,
        "tex_second_smoothness",
    ),
    (ShaderDependencyFlags::TEX_HEIGHT, "tex_height"),
    (ShaderDependencyFlags::TEX_SUBSURFACE, "tex_subsurface"),
    (ShaderDependencyFlags::HW_BILINEAR_FP16, "hw_bilinear_fp16"),
    (ShaderDependencyFlags::HW_SEPARATE_FP16, "hw_separate_fp16"),
    (ShaderDependencyFlags::HW_DURANGO, "hw_durango"),
    (ShaderDependencyFlags::HW_ORBIS, "hw_orbis"),
    (ShaderDependencyFlags::TEX_CUSTOM, "tex_custom"),
    (
        ShaderDependencyFlags::TEX_CUSTOM_SECONDARY,
        "tex_custom_secondary",
    ),
    (ShaderDependencyFlags::TEX_DECAL, "tex_decal"),
    (ShaderDependencyFlags::TEX_OCC, "tex_occ"),
    (ShaderDependencyFlags::TEX_SPECULAR_2, "tex_specular_2"),
    (ShaderDependencyFlags::HW_GLES3, "hw_gles3"),
    (ShaderDependencyFlags::USER_ENABLED, "user_enabled"),
    (ShaderDependencyFlags::HW_SAA, "hw_saa"),
    (ShaderDependencyFlags::TEX_EMITTANCE, "tex_emittance"),
    (ShaderDependencyFlags::HW_DX12, "hw_dx12"),
    (ShaderDependencyFlags::HW_DX11, "hw_dx11"),
    (ShaderDependencyFlags::HW_GL4, "hw_gl4"),
    (
        ShaderDependencyFlags::HW_WATER_TESSELLATION,
        "hw_water_tessellation",
    ),
    (
        ShaderDependencyFlags::HW_SILHOUETTE_POM,
        "hw_silhouette_pom",
    ),
    (ShaderDependencyFlags::HW_PROSPERO, "hw_prospero"),
    (ShaderDependencyFlags::HW_SCARLETT, "hw_scarlett"),
];

#[derive(Debug, thiserror::Error)]
pub enum ShaderPresetSourceTransformError {
    #[error("unsupported shader preset source path {path}")]
    UnsupportedPath { path: String },
    #[error("parse shader preset source: {0}")]
    Parse(#[from] ParseError),
    #[error("serialize shader preset source RON: {0}")]
    Serialize(#[from] ron::Error),
}
