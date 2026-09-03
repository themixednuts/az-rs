//! Legacy Cry renderer source/evidence classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
    normalize_source_path,
};
use thiserror::Error;

use crate::{DXORBIS_PATH, DxorbisAsset, ParseError, sky_light::SkyLightOpticalLut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxorbisSourceTransform {
    source_schema: SourceSchemaType,
}

impl DxorbisSourceTransform {
    #[inline]
    #[must_use]
    pub const fn new(source_schema: SourceSchemaType) -> Self {
        Self { source_schema }
    }

    #[inline]
    #[must_use]
    pub const fn source_schema(self) -> SourceSchemaType {
        self.source_schema
    }
}

impl LegacySourceTransform for DxorbisSourceTransform {
    type Error = DxorbisSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let path = input.source_path.to_string();
        if !is_legacy_dxorbis_source(&path) {
            return Err(DxorbisSourceTransformError::UnsupportedPath { path });
        }

        DxorbisAsset::parse_path(&path, input.bytes).map_err(|source| {
            DxorbisSourceTransformError::Parse {
                path: path.clone(),
                source,
            }
        })?;

        Ok(LegacySourceOutput::compatibility_evidence(
            path,
            self.source_schema,
            input.bytes.to_vec(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyLightLutSourceTransform {
    source_schema: SourceSchemaType,
}

impl SkyLightLutSourceTransform {
    #[inline]
    #[must_use]
    pub const fn new(source_schema: SourceSchemaType) -> Self {
        Self { source_schema }
    }

    #[inline]
    #[must_use]
    pub const fn source_schema(self) -> SourceSchemaType {
        self.source_schema
    }
}

impl LegacySourceTransform for SkyLightLutSourceTransform {
    type Error = SkyLightLutSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let path = input.source_path.to_string();
        if !is_legacy_sky_light_lut_source(&path) {
            return Err(SkyLightLutSourceTransformError::UnsupportedPath { path });
        }

        SkyLightOpticalLut::parse(input.bytes).map_err(|source| {
            SkyLightLutSourceTransformError::Parse {
                path: path.clone(),
                source,
            }
        })?;

        Ok(LegacySourceOutput::compatibility_evidence(
            path,
            self.source_schema,
            input.bytes.to_vec(),
        ))
    }
}

#[must_use]
pub fn is_legacy_dxorbis_source(source_path: &str) -> bool {
    let path = normalize_source_path(source_path);
    path.contains(DXORBIS_PATH)
        && std::path::Path::new(&path)
            .extension()
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("pssl") || extension.eq_ignore_ascii_case("bin")
            })
}

#[must_use]
pub fn is_legacy_sky_light_lut_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with("engineassets/sky/optical.lut")
}

#[derive(Debug, Error)]
pub enum DxorbisSourceTransformError {
    #[error("unsupported Cry renderer dxorbis path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Cry renderer dxorbis asset {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, Error)]
pub enum SkyLightLutSourceTransformError {
    #[error("unsupported Cry renderer sky-light LUT path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Cry renderer sky-light LUT {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{
        LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat,
        SourceSchemaType, source_schema_type,
    };

    use super::*;
    use crate::sky_light::{
        SKY_LIGHT_LUT_MAGIC, SKY_LIGHT_LUT_SIZE, SKY_LIGHT_LUT_TABLE_SET, SKY_LIGHT_LUT_VERSION,
    };

    const TEST_SCHEMA: SourceSchemaType = source_schema_type::<TestCryRendererEvidenceSource>();

    #[derive(SourceFormat)]
    #[source(schema = "azoth.test.CryRendererEvidence", ext = "cry-renderer-test")]
    struct TestCryRendererEvidenceSource;

    #[test]
    fn routes_only_known_cry_renderer_internals() {
        assert!(is_legacy_dxorbis_source(
            "EngineAssets/Dxorbis/debug_shader.pssl"
        ));
        assert!(is_legacy_dxorbis_source(
            "EngineAssets/Dxorbis/hw_cursor_image.bin"
        ));
        assert!(!is_legacy_dxorbis_source("Shaders/Cache/lookupdata.bin"));

        assert!(is_legacy_sky_light_lut_source(
            "EngineAssets/Sky/Optical.lut"
        ));
        assert!(!is_legacy_sky_light_lut_source("Textures/colorgrade.lut"));
    }

    #[test]
    fn records_pssl_shader_source_as_compatibility_evidence() {
        let output = DxorbisSourceTransform::new(TEST_SCHEMA)
            .transform(LegacySourceInput::new(
                "EngineAssets/Dxorbis/debug_shader.pssl",
                b"float4 main() : SV_Target { return 1; }\n",
            ))
            .unwrap();

        match output {
            LegacySourceOutput::CompatibilityEvidence(artifact) => {
                assert_eq!(artifact.path, "engineassets/dxorbis/debug_shader.pssl");
                assert_eq!(artifact.schema, TEST_SCHEMA);
                assert_eq!(artifact.bytes, b"float4 main() : SV_Target { return 1; }\n");
            }
            other => panic!("expected compatibility evidence, got {other:?}"),
        }
    }

    #[test]
    fn records_hardware_cursor_bin_as_compatibility_evidence() {
        let bytes = vec![0; crate::HARDWARE_CURSOR_IMAGE_SIZE];
        let output = DxorbisSourceTransform::new(TEST_SCHEMA)
            .transform(LegacySourceInput::new(
                "EngineAssets/Dxorbis/hw_cursor_image.bin",
                &bytes,
            ))
            .unwrap();

        assert!(matches!(
            output,
            LegacySourceOutput::CompatibilityEvidence(_)
        ));
    }

    #[test]
    fn records_sky_light_lut_as_compatibility_evidence() {
        let bytes = sky_light_lut_bytes();
        let output = SkyLightLutSourceTransform::new(TEST_SCHEMA)
            .transform(LegacySourceInput::new(
                "EngineAssets/Sky/Optical.lut",
                &bytes,
            ))
            .unwrap();

        match output {
            LegacySourceOutput::CompatibilityEvidence(artifact) => {
                assert_eq!(artifact.path, "engineassets/sky/optical.lut");
                assert_eq!(artifact.schema, TEST_SCHEMA);
                assert_eq!(artifact.bytes, bytes);
            }
            other => panic!("expected compatibility evidence, got {other:?}"),
        }
    }

    fn sky_light_lut_bytes() -> Vec<u8> {
        let mut bytes = vec![0; SKY_LIGHT_LUT_SIZE];
        bytes[..4].copy_from_slice(SKY_LIGHT_LUT_MAGIC);
        bytes[4..6].copy_from_slice(&SKY_LIGHT_LUT_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&SKY_LIGHT_LUT_TABLE_SET.to_le_bytes());
        bytes
    }
}
