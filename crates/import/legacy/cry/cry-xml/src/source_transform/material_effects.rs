use std::{num::ParseFloatError, num::ParseIntError, str};

use az_asset_builder::normalize_source_path;
use quick_xml::events::attributes::AttrError;
use serde::{Deserialize, Serialize};

use super::to_ron_bytes;

mod fx_library;
mod spreadsheet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "document", rename_all = "snake_case")]
pub enum MaterialEffectsSource {
    Library(MaterialEffectsLibrarySource),
    InteractionIndex(MaterialEffectsInteractionIndexSource),
}

impl MaterialEffectsSource {
    /// Parse a legacy `materialeffects.xml` payload.
    ///
    /// # Errors
    ///
    /// Returns [`MaterialEffectsParseError::UnsupportedPath`] when
    /// `source_path` is not a material-effects asset,
    /// [`MaterialEffectsParseError::InvalidUtf8`] when `bytes` are not UTF-8,
    /// and any parser error the XML reader reports for malformed markup or an
    /// unparsable attribute value.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, MaterialEffectsParseError> {
        let normalized_source_path = normalize_source_path(source_path);
        if !is_material_effects_xml_path(&normalized_source_path) {
            return Err(MaterialEffectsParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes).map_err(MaterialEffectsParseError::InvalidUtf8)?;
        if normalized_source_path == "libs/materialeffects/materialeffects.xml" {
            spreadsheet::parse_interaction_index(normalized_source_path, xml)
        } else {
            fx_library::parse_fx_library(normalized_source_path, xml)
        }
    }

    /// Serialize this source projection to pretty RON bytes.
    ///
    /// # Errors
    ///
    /// Returns any [`ron::Error`] the RON serializer reports for this value.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectsLibrarySource {
    pub source_path: String,
    pub kind: String,
    pub effects: Vec<MaterialEffectSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsInteractionIndexSource {
    pub source_path: String,
    pub worksheet: String,
    pub columns: Vec<MaterialEffectsInteractionAxisEntrySource>,
    pub rows: Vec<MaterialEffectsInteractionRowSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsInteractionAxisEntrySource {
    pub index: u32,
    pub name: String,
    pub metadata: MaterialEffectsSpreadsheetCellMetadataSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsInteractionRowSource {
    pub index: u32,
    pub kind: MaterialEffectsInteractionRowKindSource,
    pub name: String,
    pub metadata: MaterialEffectsSpreadsheetCellMetadataSource,
    pub entries: Vec<MaterialEffectsInteractionCellSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialEffectsInteractionRowKindSource {
    Surface,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsInteractionCellSource {
    pub column_index: u32,
    pub column: String,
    pub reference: MaterialEffectReferenceSource,
    pub metadata: MaterialEffectsSpreadsheetCellMetadataSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectReferenceSource {
    pub raw: String,
    pub library: String,
    pub effect: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsSpreadsheetCellMetadataSource {
    pub formula: Option<String>,
    pub value_type: Option<String>,
    pub rel_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectSource {
    pub name: String,
    pub delay: Option<f32>,
    pub filter: MaterialEffectFilterSource,
    pub resources: Vec<MaterialEffectResourceSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectFilterSource {
    pub game: Option<String>,
    pub devmode: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterialEffectResourceSource {
    Audio(MaterialEffectAudioSource),
    Particle(MaterialEffectParticleSource),
    Decal(MaterialEffectDecalSource),
    ForceFeedback(MaterialEffectForceFeedbackSource),
    Random(MaterialEffectRandomSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectAudioSource {
    pub trigger: String,
    pub filter: MaterialEffectFilterSource,
    pub switches: Vec<MaterialEffectAudioSwitchSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectAudioSwitchSource {
    pub name: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectParticleSource {
    pub filter: MaterialEffectFilterSource,
    pub names: Vec<MaterialEffectParticleNameSource>,
    pub direction: Option<MaterialEffectParticleDirectionSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectParticleNameSource {
    pub path: String,
    pub direction: Option<String>,
    pub user_data: Option<String>,
    pub scale: Option<f32>,
    pub max_distance: Option<f32>,
    pub min_scale: Option<f32>,
    pub max_scale: Option<f32>,
    pub max_scale_distance: Option<f32>,
    pub attach: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialEffectParticleDirectionSource {
    Normal,
    Ricochet,
    Unknown(MaterialEffectParticleUnknownDirectionSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectParticleUnknownDirectionSource {
    pub raw: String,
}

impl MaterialEffectParticleDirectionSource {
    fn from_legacy(raw: &str) -> Self {
        match raw.trim() {
            "Normal" => Self::Normal,
            "Ricochet" => Self::Ricochet,
            _ => Self::Unknown(MaterialEffectParticleUnknownDirectionSource {
                raw: raw.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectDecalSource {
    pub filter: MaterialEffectFilterSource,
    pub material: Option<String>,
    pub min_scale: Option<f32>,
    pub max_scale: Option<f32>,
    pub rotation_degrees: Option<f32>,
    pub grow_time: Option<f32>,
    pub assemble_decals: Option<bool>,
    pub force_edge: Option<bool>,
    pub lifetime: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectForceFeedbackSource {
    pub name: String,
    pub filter: MaterialEffectFilterSource,
    pub min_falloff_distance: Option<f32>,
    pub max_falloff_distance: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialEffectRandomSource {
    pub filter: MaterialEffectFilterSource,
    pub resources: Vec<MaterialEffectResourceSource>,
}

#[must_use]
pub fn is_material_effects_xml_path(normalized_source_path: &str) -> bool {
    normalized_source_path == "libs/materialeffects/materialeffects.xml"
        || (normalized_source_path.starts_with("libs/materialeffects/fxlibs/")
            && crate::has_extension(normalized_source_path, "xml"))
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialEffectsParseError {
    #[error("unsupported material-effects XML path {path}")]
    UnsupportedPath { path: String },
    #[error("material-effects XML is not UTF-8")]
    InvalidUtf8(str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("missing {name:?} worksheet")]
    MissingWorksheet { name: &'static str },
    #[error("material-effects spreadsheet has no header row")]
    MissingHeaderRow,
    #[error("material-effects spreadsheet row {row_index} has entries but no row label")]
    MissingRowLabel { row_index: u32 },
    #[error(
        "material-effects spreadsheet row {row_index} entry at column {column_index} has no matching header column"
    )]
    EntryWithoutColumn { row_index: u32, column_index: u32 },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("element appears after closing material-effects root")]
    ElementAfterRoot,
    #[error("unexpected <{element}> element")]
    UnknownElement { element: String },
    #[error("unexpected {attribute:?} attribute on <{element}>")]
    UnknownAttribute {
        element: &'static str,
        attribute: String,
    },
    #[error("missing {attribute:?} attribute on <{element}>")]
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("<{element}> cannot be nested")]
    NestedElement { element: &'static str },
    #[error("<{element}> cannot appear outside <{parent}>")]
    ElementInWrongParent {
        element: &'static str,
        parent: &'static str,
    },
    #[error("<{element}> text capture cannot be nested")]
    NestedTextElement { element: &'static str },
    #[error("unexpected </{element}>")]
    UnexpectedEndElement { element: String },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("unexpected text in material-effects XML: {text:?}")]
    UnexpectedText { text: String },
    #[error("invalid integer {value:?} in <{element}> {attribute}: {source}")]
    InvalidInteger {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseIntError,
    },
    #[error("invalid bool {value:?} in <{element}> {attribute}")]
    InvalidBool {
        element: &'static str,
        attribute: &'static str,
        value: String,
    },
    #[error("invalid float {value:?} in <{element}> {attribute}: {source}")]
    InvalidFloat {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseFloatError,
    },
}
