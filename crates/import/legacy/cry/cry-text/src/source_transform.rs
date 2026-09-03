//! Source transforms for typed Cry/Lumberyard text-backed assets.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    GpuDeviceTable, LayerResourceList, ParseError, ResourceList, TextAssetKind, source_schemas,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelReferenceListSource {
    pub source_path: String,
    pub kind: LevelReferenceListSourceKind,
    pub entries: Vec<LevelReferenceListSourceEntry>,
}

impl LevelReferenceListSource {
    /// Builds the authoring source from a legacy resource-list `.txt`.
    ///
    /// # Errors
    ///
    /// Returns any error [`LayerResourceList::parse_str`] returns when
    /// `source_path` classifies as a per-layer list — [`ParseError::MissingField`]
    /// with `field: "path"` for a data line with no `;`. Plain resource lists
    /// never fail, and GPU-device and plain-text paths produce an empty entry
    /// list rather than an error.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, ParseError> {
        let kind = LevelReferenceListSourceKind::from_path(source_path);
        let text = crate::decode_text(bytes);
        let entries = match TextAssetKind::from_path(source_path) {
            TextAssetKind::ResourceList => ResourceList::parse_str(&text)?
                .entries()
                .iter()
                .map(|entry| LevelReferenceListSourceEntry::Resource {
                    path: entry.path().to_string(),
                })
                .collect(),
            TextAssetKind::LayerResourceList => LayerResourceList::parse_str(&text)?
                .entries()
                .iter()
                .map(|entry| LevelReferenceListSourceEntry::LayerResource {
                    layer: entry.layer().to_string(),
                    path: entry.path().to_string(),
                })
                .collect(),
            TextAssetKind::GpuDeviceTable | TextAssetKind::PlainText => Vec::new(),
        };

        Ok(Self {
            source_path: normalize_source_path(source_path),
            kind,
            entries,
        })
    }

    /// Serialises this source to pretty-printed RON bytes.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// plain strings and enums that RON always accepts, so this is reachable
    /// only through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelReferenceListSourceKind {
    ResourceList,
    AutoResourceList,
    BrushList,
    ShaderList,
    FullLodAssetList,
    Tags,
    PerLayerResourceList,
}

impl LevelReferenceListSourceKind {
    fn from_path(source_path: &str) -> Self {
        let normalized = normalize_source_path(source_path);
        match normalized.rsplit('/').next().unwrap_or(normalized.as_str()) {
            "auto_resourcelist.txt" => Self::AutoResourceList,
            "brushlist.txt" => Self::BrushList,
            "shaderslist.txt" => Self::ShaderList,
            "full_lod_asset_list.txt" => Self::FullLodAssetList,
            "tags.txt" => Self::Tags,
            "perlayerresourcelist.txt" => Self::PerLayerResourceList,
            _ => Self::ResourceList,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LevelReferenceListSourceEntry {
    Resource { path: String },
    LayerResource { layer: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuDeviceTableSource {
    pub source_path: String,
    pub vendor: String,
    pub devices: Vec<GpuDeviceSourceEntry>,
}

impl GpuDeviceTableSource {
    /// Builds the authoring source from a legacy GPU device table `.txt`.
    ///
    /// # Errors
    ///
    /// Returns any error [`GpuDeviceTable::parse_str`] returns —
    /// [`ParseError::MissingField`] naming `vendor_id`, `device_id` or
    /// `bucket` for a short row, or [`ParseError::InvalidHex`] when one of
    /// those columns does not parse.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, ParseError> {
        let text = crate::decode_text(bytes);
        let table = GpuDeviceTable::parse_str(&text)?;
        Ok(Self {
            source_path: normalize_source_path(source_path),
            vendor: gpu_vendor_name(source_path),
            devices: table
                .entries()
                .iter()
                .map(|device| GpuDeviceSourceEntry {
                    vendor_id: device.vendor_id(),
                    device_id: device.device_id(),
                    bucket: device.bucket(),
                    comment: device.comment().to_string(),
                })
                .collect(),
        })
    }

    /// Serialises this source to pretty-printed RON bytes.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// integers and plain strings that RON always accepts, so this is
    /// reachable only through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuDeviceSourceEntry {
    pub vendor_id: u32,
    pub device_id: u32,
    pub bucket: i32,
    pub comment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSourceKind {
    LevelReferenceList,
    GpuDeviceTable,
}

impl TextSourceKind {
    fn from_path(source_path: &str) -> Option<Self> {
        match TextAssetKind::from_path(source_path) {
            TextAssetKind::ResourceList | TextAssetKind::LayerResourceList => {
                Some(Self::LevelReferenceList)
            }
            TextAssetKind::GpuDeviceTable => Some(Self::GpuDeviceTable),
            TextAssetKind::PlainText => None,
        }
    }

    const fn source_schema(self) -> az_asset_builder::SourceSchemaType {
        match self {
            Self::LevelReferenceList => source_schemas::LEVEL_REFERENCE_LIST,
            Self::GpuDeviceTable => source_schemas::GPU_DEVICE_TABLE,
        }
    }

    const fn source_suffix(self) -> &'static str {
        match self {
            Self::LevelReferenceList => "levellist.ron",
            Self::GpuDeviceTable => "gpudevices.ron",
        }
    }

    fn source_path(self, source_path: &str) -> String {
        source_path_with_suffix(source_path, self.source_suffix())
    }

    fn to_ron_bytes(
        self,
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Vec<u8>, TextSourceTransformError> {
        match self {
            Self::LevelReferenceList => {
                Ok(LevelReferenceListSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::GpuDeviceTable => {
                Ok(GpuDeviceTableSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextSourceTransform;

impl LegacySourceTransform for TextSourceTransform {
    type Error = TextSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let kind = TextSourceKind::from_path(&input.source_path).ok_or_else(|| {
            TextSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            }
        })?;

        Ok(LegacySourceOutput::authoring_source(
            kind.source_path(&input.source_path),
            kind.source_schema(),
            kind.to_ron_bytes(&input.source_path, input.bytes)?,
        ))
    }
}

#[must_use]
pub fn is_legacy_text_source(source_path: &str) -> bool {
    TextSourceKind::from_path(source_path).is_some()
}

#[must_use]
pub fn text_source_path(source_path: &str) -> Option<String> {
    TextSourceKind::from_path(source_path).map(|kind| kind.source_path(source_path))
}

#[must_use]
pub fn level_reference_list_source_path(source_path: &str) -> Option<String> {
    matches!(
        TextSourceKind::from_path(source_path),
        Some(TextSourceKind::LevelReferenceList)
    )
    .then(|| source_path_with_suffix(source_path, "levellist.ron"))
}

#[must_use]
pub fn gpu_device_table_source_path(source_path: &str) -> Option<String> {
    matches!(
        TextSourceKind::from_path(source_path),
        Some(TextSourceKind::GpuDeviceTable)
    )
    .then(|| source_path_with_suffix(source_path, "gpudevices.ron"))
}

fn source_path_with_suffix(source_path: &str, suffix: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".txt").unwrap_or(&normalized);
    format!("{stem}.{suffix}")
}

fn gpu_vendor_name(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(normalized.as_str())
        .strip_suffix(".txt")
        .unwrap_or("")
        .to_string()
}

fn to_ron_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ron::Error> {
    let ron = ron::ser::to_string_pretty(value, PrettyConfig::default())?;
    Ok(format!("{ron}\n").into_bytes())
}

#[derive(Debug, thiserror::Error)]
pub enum TextSourceTransformError {
    #[error("unsupported typed text path {path}")]
    UnsupportedPath { path: String },
    #[error("parse typed text source: {0}")]
    Parse(#[from] ParseError),
    #[error("serialize typed text source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

    use super::*;

    #[test]
    fn transforms_resource_list_to_level_reference_source() {
        let output = TextSourceTransform
            .transform(LegacySourceInput::new(
                "Levels/foo/brushlist.txt",
                b"\xef\xbb\xbfobjects/tree.cgf\n\ntextures/wood.dds\n",
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(artifact.path, "levels/foo/brushlist.levellist.ron");
        assert_eq!(artifact.schema, source_schemas::LEVEL_REFERENCE_LIST);
        let source: LevelReferenceListSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "levels/foo/brushlist.txt");
        assert_eq!(source.kind, LevelReferenceListSourceKind::BrushList);
        assert_eq!(
            source.entries[0],
            LevelReferenceListSourceEntry::Resource {
                path: "objects/tree.cgf".to_string(),
            }
        );
        assert_eq!(
            source.entries[1],
            LevelReferenceListSourceEntry::Resource {
                path: "textures/wood.dds".to_string(),
            }
        );
    }

    #[test]
    fn transforms_layer_resource_list_to_level_reference_source() {
        let output = TextSourceTransform
            .transform(LegacySourceInput::new(
                "Levels/foo/perlayerresourcelist.txt",
                b"Main; objects/tree.cgf\nCaves; textures/stone.dds\n",
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(
            artifact.path,
            "levels/foo/perlayerresourcelist.levellist.ron"
        );
        assert_eq!(artifact.schema, source_schemas::LEVEL_REFERENCE_LIST);
        let source: LevelReferenceListSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(
            source.kind,
            LevelReferenceListSourceKind::PerLayerResourceList
        );
        assert_eq!(
            source.entries[0],
            LevelReferenceListSourceEntry::LayerResource {
                layer: "Main".to_string(),
                path: "objects/tree.cgf".to_string(),
            }
        );
    }

    #[test]
    fn transforms_gpu_device_table_to_authoring_source() {
        let output = TextSourceTransform
            .transform(LegacySourceInput::new(
                "Config/Gpu/amd.txt",
                b"0x1002, 0x6759, 1 // Radeon HD 6500 Series\n",
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(artifact.path, "config/gpu/amd.gpudevices.ron");
        assert_eq!(artifact.schema, source_schemas::GPU_DEVICE_TABLE);
        let source: GpuDeviceTableSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "config/gpu/amd.txt");
        assert_eq!(source.vendor, "amd");
        assert_eq!(source.devices[0].vendor_id, 0x1002);
        assert_eq!(source.devices[0].device_id, 0x6759);
        assert_eq!(source.devices[0].bucket, 1);
        assert_eq!(source.devices[0].comment, "Radeon HD 6500 Series");
    }

    #[test]
    fn text_source_paths_only_claim_known_typed_text() {
        assert_eq!(
            text_source_path("levels/foo/resourcelist.txt").as_deref(),
            Some("levels/foo/resourcelist.levellist.ron")
        );
        assert_eq!(
            text_source_path("config/gpu/nvidia.txt").as_deref(),
            Some("config/gpu/nvidia.gpudevices.ron")
        );
        assert!(is_legacy_text_source("levels/foo/perlayerresourcelist.txt"));
        assert!(!is_legacy_text_source("readme.txt"));
        assert!(text_source_path("readme.txt").is_none());
    }
}
