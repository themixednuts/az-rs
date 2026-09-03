//! Legacy `NvCloth` source import transforms.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use az_nv_cloth::{ClothFabricSource, ClothMaterialSource, source_schemas};
use ron::ser::PrettyConfig;
use serde::Serialize;

use crate::{
    ClothFabricImportError, ClothMaterialParseError, parse_cloth_fabric_source,
    parse_cloth_material,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClothMaterialSourceTransform;

impl LegacySourceTransform for ClothMaterialSourceTransform {
    type Error = ClothMaterialSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_cloth_material_source(&input.source_path) {
            return Err(ClothMaterialSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = ClothMaterialSource::new(parse_cloth_material(input.bytes)?);
        Ok(LegacySourceOutput::authoring_source(
            cloth_material_source_path(&input.source_path),
            source_schemas::CLOTH_MATERIAL,
            to_ron_bytes(&source)?,
        ))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClothFabricSourceTransform;

impl LegacySourceTransform for ClothFabricSourceTransform {
    type Error = ClothFabricSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let source_path = input.source_path.to_string();
        if !is_legacy_cloth_fabric_source(&source_path) {
            return Err(ClothFabricSourceTransformError::UnsupportedPath { path: source_path });
        }

        let source: ClothFabricSource = parse_cloth_fabric_source(input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            cloth_fabric_source_path(&source_path),
            source_schemas::CLOTH_FABRIC,
            to_ron_bytes(&source)?,
        ))
    }
}

fn to_ron_bytes(value: &impl Serialize) -> Result<Vec<u8>, ron::Error> {
    let ron = ron::ser::to_string_pretty(value, PrettyConfig::default())?;
    Ok(format!("{ron}\n").into_bytes())
}

#[must_use]
pub fn is_legacy_cloth_material_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".clothmaterial")
}

#[must_use]
pub fn is_legacy_cloth_fabric_source(source_path: &str) -> bool {
    std::path::Path::new(&normalize_source_path(source_path))
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cloth"))
}

#[must_use]
pub fn cloth_material_source_path(source_path: &str) -> String {
    replace_legacy_suffix(source_path, ".clothmaterial", ".clothmaterial.ron")
}

#[must_use]
pub fn cloth_fabric_source_path(source_path: &str) -> String {
    replace_legacy_suffix(source_path, ".cloth", ".cloth.ron")
}

fn replace_legacy_suffix(source_path: &str, suffix: &str, replacement: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(suffix).unwrap_or(&normalized);
    format!("{stem}{replacement}")
}

#[derive(Debug, thiserror::Error)]
pub enum ClothMaterialSourceTransformError {
    #[error("unsupported cloth material path {path}")]
    UnsupportedPath { path: String },
    #[error("parse cloth material source: {0}")]
    Parse(#[from] ClothMaterialParseError),
    #[error("serialize cloth material source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ClothFabricSourceTransformError {
    #[error("unsupported cloth fabric path {path}")]
    UnsupportedPath { path: String },
    #[error("parse cloth fabric source: {0}")]
    Parse(#[from] ClothFabricImportError),
    #[error("serialize cloth fabric source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_replace_legacy_extensions() {
        assert_eq!(
            cloth_material_source_path("Characters/Hero/Cape.clothmaterial"),
            "characters/hero/cape.clothmaterial.ron"
        );
        assert_eq!(
            cloth_fabric_source_path("Characters/Hero/Cape.cloth"),
            "characters/hero/cape.cloth.ron"
        );
    }

    #[test]
    fn classifies_only_uncooked_source_extensions() {
        assert!(is_legacy_cloth_material_source(
            "characters/hero/cape.clothmaterial"
        ));
        assert!(is_legacy_cloth_fabric_source("characters/hero/cape.CLOTH"));
        assert!(!is_legacy_cloth_material_source(
            "characters/hero/cape.clothmaterial.ron"
        ));
        assert!(!is_legacy_cloth_fabric_source(
            "characters/hero/cape.cloth.ron"
        ));
    }

    #[test]
    fn material_transform_emits_typed_ron_source() {
        let output = ClothMaterialSourceTransform
            .transform(LegacySourceInput::new(
                "Characters/Hero/Cape.clothmaterial",
                &cloth_material_bytes(),
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("cloth material should become authoring source");
        };
        assert_eq!(artifact.path, "characters/hero/cape.clothmaterial.ron");
        assert_eq!(artifact.schema, source_schemas::CLOTH_MATERIAL);

        let source: ClothMaterialSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.version, 1);
        assert_exact(source.material.stiffness_frequency, 50.0);
        assert_exact(source.material.motion_constraints.max_distance, 0.5);
    }

    /// Compares a round-tripped `f32` bit-exactly.
    ///
    /// The fixture writes the same little-endian pattern the parser reads
    /// back, so any difference is a decode bug rather than accumulated error;
    /// an epsilon window would hide exactly the bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    fn cloth_material_bytes() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(crate::CLOTH_MATERIAL_SIZE);
        for value in [
            1.0_f32, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0,
            50.0, 0.5, 0.5, 0.5, 1.0, 0.0, 0.0, 1.0, 1.0, 120.0, 30.0, 0.0, 0.1, 0.1, 0.1, 0.0,
            0.1, 0.1, 0.1, 0.0, 0.1, 0.1, 0.1, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0,
            1.0, 1.0, 0.0,
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }
}
