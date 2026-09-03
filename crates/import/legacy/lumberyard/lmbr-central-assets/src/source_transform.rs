//! Legacy `LmbrCentral` source transforms.

use az_asset::{EngineTextureFormat, normalize_source_path};
use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
};
use az_gem_lmbr_central::{VertexShapeAsset, VertexShapeMetadata, VertexShapeReserved};
use bevy::math::Vec3;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    MaterialSource, MaterialTransformError, VertexShapeTransformError, parse_material_asset,
    source_schemas, transform_vertex_shape_asset,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialSourceTransform {
    texture_format: EngineTextureFormat,
}

impl MaterialSourceTransform {
    #[must_use]
    pub const fn new(texture_format: EngineTextureFormat) -> Self {
        Self { texture_format }
    }
}

impl LegacySourceTransform for MaterialSourceTransform {
    type Error = MaterialSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let asset = parse_material_asset(&input.source_path, input.bytes, self.texture_format)?;
        let source = MaterialSource::from_asset(&asset);
        Ok(LegacySourceOutput::authoring_source(
            material_source_path(&input.source_path),
            material_schema(),
            source.to_ron_bytes()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeShapeSource {
    pub source_path: String,
    pub version: u32,
    pub vertices: Vec<VolumeShapeVertexSource>,
    pub metadata: Vec<VolumeShapeMetadataSource>,
    pub height: f32,
    pub reserved: VolumeShapeReservedSource,
}

impl VolumeShapeSource {
    /// Parses a legacy `.vshapec` payload into the volume-shape source model.
    ///
    /// # Errors
    ///
    /// Returns any error [`transform_vertex_shape_asset`] returns.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, VertexShapeTransformError> {
        let asset = transform_vertex_shape_asset(bytes)?;
        Ok(Self::from_asset(source_path, &asset))
    }

    #[must_use]
    pub fn from_asset(source_path: &str, asset: &VertexShapeAsset) -> Self {
        Self {
            source_path: normalize_source_path(source_path),
            version: asset.version,
            vertices: asset
                .vertices
                .iter()
                .map(VolumeShapeVertexSource::from)
                .collect(),
            metadata: asset
                .metadata
                .iter()
                .map(VolumeShapeMetadataSource::from)
                .collect(),
            height: asset.height,
            reserved: VolumeShapeReservedSource::from(asset.reserved),
        }
    }

    /// Serializes the volume-shape source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Deserializes the volume-shape source model from RON bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`ron::error::SpannedError`] when `bytes` is not valid RON or
    /// does not match the [`VolumeShapeSource`] shape.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }

    #[must_use]
    pub fn to_asset(&self) -> VertexShapeAsset {
        VertexShapeAsset::new(
            self.version,
            self.vertices.iter().copied().map(Vec3::from).collect(),
            self.metadata
                .iter()
                .map(VertexShapeMetadata::from)
                .collect(),
            self.height,
            VertexShapeReserved::from(self.reserved),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VolumeShapeVertexSource {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<&Vec3> for VolumeShapeVertexSource {
    fn from(value: &Vec3) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
        }
    }
}

impl From<VolumeShapeVertexSource> for Vec3 {
    fn from(value: VolumeShapeVertexSource) -> Self {
        Self::new(value.x, value.y, value.z)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeShapeMetadataSource {
    pub key: String,
    pub value: String,
}

impl From<&VertexShapeMetadata> for VolumeShapeMetadataSource {
    fn from(value: &VertexShapeMetadata) -> Self {
        Self {
            key: value.key.clone(),
            value: value.value.clone(),
        }
    }
}

impl From<&VolumeShapeMetadataSource> for VertexShapeMetadata {
    fn from(value: &VolumeShapeMetadataSource) -> Self {
        Self {
            key: value.key.clone(),
            value: value.value.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeShapeReservedSource {
    pub first: u32,
    pub second: u32,
    pub third: u32,
}

impl From<VertexShapeReserved> for VolumeShapeReservedSource {
    fn from(value: VertexShapeReserved) -> Self {
        Self {
            first: value.first,
            second: value.second,
            third: value.third,
        }
    }
}

impl From<VolumeShapeReservedSource> for VertexShapeReserved {
    fn from(value: VolumeShapeReservedSource) -> Self {
        Self {
            first: value.first,
            second: value.second,
            third: value.third,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VolumeShapeSourceTransform;

impl LegacySourceTransform for VolumeShapeSourceTransform {
    type Error = VolumeShapeSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_volume_shape_source(&input.source_path) {
            return Err(VolumeShapeSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = VolumeShapeSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            volume_shape_source_path(&input.source_path),
            source_schemas::VOLUME_SHAPE,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub const fn material_schema() -> SourceSchemaType {
    source_schemas::MATERIAL
}

#[must_use]
pub fn material_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".mtl").unwrap_or(&normalized);
    format!("{stem}.material.ron")
}

#[must_use]
pub fn is_legacy_volume_shape_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".vshapec")
}

#[must_use]
pub fn volume_shape_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".vshapec").unwrap_or(&normalized);
    format!("{stem}.shape.ron")
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialSourceTransformError {
    #[error("transform material asset: {0}")]
    Transform(#[from] MaterialTransformError),
    #[error("serialize material source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum VolumeShapeSourceTransformError {
    #[error("unsupported volume shape path {path}")]
    UnsupportedPath { path: String },
    #[error("transform volume shape source: {0}")]
    Transform(#[from] VertexShapeTransformError),
    #[error("serialize volume shape source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use az_gem_lmbr_central::MaterialTextureMap;

    use super::*;

    #[test]
    fn material_source_path_replaces_legacy_extension() {
        assert_eq!(
            material_source_path("Materials/Foo/Example.mtl"),
            "materials/foo/example.material.ron"
        );
    }

    #[test]
    fn volume_shape_source_path_replaces_legacy_extension() {
        assert_eq!(
            volume_shape_source_path("Volumes/Region.vshapec"),
            "volumes/region.shape.ron"
        );
    }

    #[test]
    fn volume_shape_transform_emits_ron_authoring_source() {
        let output = VolumeShapeSourceTransform
            .transform(LegacySourceInput::new(
                "Volumes/Region.vshapec",
                &vertex_shape_bytes(),
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("volume shape transform should emit authoring source");
        };
        assert_eq!(artifact.path, "volumes/region.shape.ron");
        assert_eq!(artifact.schema, source_schemas::VOLUME_SHAPE);
        let source: VolumeShapeSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "volumes/region.vshapec");
        assert_eq!(
            source.vertices,
            vec![VolumeShapeVertexSource {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            }]
        );
        assert_eq!(source.metadata[0].key, "TerritoryId");
        assert_eq!(source.metadata[0].value, "14:@Example");
        assert_exact(source.height, 32.0);
        assert_eq!(source.reserved.third, 7);
    }

    #[test]
    fn volume_shape_paths_only_claim_legacy_vshapec() {
        assert!(is_legacy_volume_shape_source("Volumes/Region.vshapec"));
        assert!(!is_legacy_volume_shape_source("Volumes/Region.shape.ron"));
        assert!(!is_legacy_volume_shape_source("Shapes/Footstep.rnr"));
    }

    /// Compares a round-tripped `f32` bit-exactly.
    ///
    /// The fixture writes the same little-endian pattern the reader decodes,
    /// so any difference is a decode bug rather than accumulated error; an
    /// epsilon window would hide exactly the bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    fn vertex_shape_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        write_string(&mut bytes, "TerritoryId");
        write_string(&mut bytes, "14:@Example");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&32.0f32.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes
    }

    #[test]
    fn material_transform_emits_ron_authoring_source() {
        let xml = br#"
<Material MtlFlags="524544">
 <Textures>
  <Texture Map="Diffuse" File="Objects/Foo/Diff.tif"/>
 </Textures>
</Material>
"#;

        let output = MaterialSourceTransform::new(EngineTextureFormat::Dds)
            .transform(LegacySourceInput::new("Materials/Foo/Example.mtl", xml))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("material transform should emit authoring source");
        };
        assert_eq!(artifact.path, "materials/foo/example.material.ron");
        assert_eq!(artifact.schema, source_schemas::MATERIAL);
        let text = std::str::from_utf8(&artifact.bytes).unwrap();
        let source = ron::from_str::<MaterialSource>(text).unwrap();
        assert_eq!(source.source_path, "materials/foo/example.mtl");
        assert_eq!(source.root.material_flags, Some(524_544));
        assert_eq!(
            source.root.textures[0].image_asset_path.as_deref(),
            Some("objects/foo/diff.dds")
        );
        assert_eq!(source.root.textures[0].map, "Diffuse");

        let asset = source.to_asset();
        let diffuse = asset
            .root
            .textures
            .iter()
            .find(|texture| texture.map == MaterialTextureMap::Diffuse)
            .unwrap();
        assert_eq!(
            diffuse.image_asset_path.as_deref(),
            Some("objects/foo/diff.dds")
        );
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
