//! Legacy shader-cache source classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};

use crate::{ParseError, ResourceFile, ShaderBin, ShaderLookupData};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShaderCacheSourceTransform;

impl LegacySourceTransform for ShaderCacheSourceTransform {
    type Error = ShaderCacheSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let source_path = input.source_path.to_string();
        let kind = ShaderCacheSourceKind::from_source_path(&source_path).ok_or_else(|| {
            ShaderCacheSourceTransformError::UnsupportedPath {
                path: source_path.clone(),
            }
        })?;

        match kind {
            ShaderCacheSourceKind::LookupData => {
                ShaderLookupData::parse(input.bytes)?;
            }
            ShaderCacheSourceKind::ShaderBin => {
                ShaderBin::parse(input.bytes)?;
            }
            ShaderCacheSourceKind::ResourceCache => {
                ResourceFile::parse(input.bytes)?;
            }
        }

        Ok(LegacySourceOutput::Excluded {
            reason: format!(
                "legacy Cry renderer shader cache {source_path} is a generated product; Azoth regenerates shader products from native shader/material source"
            ),
        })
    }
}

#[must_use]
pub fn is_legacy_shader_cache_source(source_path: &str) -> bool {
    ShaderCacheSourceKind::from_source_path(source_path).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShaderCacheSourceKind {
    LookupData,
    ShaderBin,
    ResourceCache,
}

impl ShaderCacheSourceKind {
    fn from_source_path(source_path: &str) -> Option<Self> {
        let source_path = normalize_source_path(source_path);
        if !source_path.starts_with("shaders/cache/") {
            return None;
        }

        if source_path.ends_with("/lookupdata.bin") {
            return Some(Self::LookupData);
        }

        let extension = std::path::Path::new(&source_path)
            .extension()
            .and_then(|extension| extension.to_str())?;
        if extension.eq_ignore_ascii_case("cfib") || extension.eq_ignore_ascii_case("cfxb") {
            return Some(Self::ShaderBin);
        }
        if extension.eq_ignore_ascii_case("fxcb") {
            return Some(Self::ResourceCache);
        }

        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShaderCacheSourceTransformError {
    #[error("unsupported shader-cache source path {path}")]
    UnsupportedPath { path: String },
    #[error("parse shader-cache source: {0}")]
    Parse(#[from] ParseError),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

    use super::*;
    use crate::{LOOKUP_DATA_CACHE_VERSION_SIZE, LOOKUP_DATA_MAGIC, ResourceVersion};

    #[test]
    fn routes_shader_cache_sources_without_claiming_generic_bin() {
        assert!(is_legacy_shader_cache_source(
            "Shaders/Cache/D3D11/common.cfib"
        ));
        assert!(is_legacy_shader_cache_source(
            "Shaders/Cache/D3D12/fallback.cfxb"
        ));
        assert!(is_legacy_shader_cache_source(
            "Shaders/Cache/D3D11/CGPShaders/fixedpipelineemu@dummyps.fxcb"
        ));
        assert!(is_legacy_shader_cache_source(
            "Shaders/Cache/D3D12/lookupdata.bin"
        ));
        assert!(!is_legacy_shader_cache_source(
            "Sounds/Wwise/lookupdata.bin"
        ));
        assert!(!is_legacy_shader_cache_source("Shaders/alphacutout.ext"));
    }

    #[test]
    fn excludes_valid_shader_token_binaries() {
        for path in [
            "Shaders/Cache/D3D11/common.cfib",
            "Shaders/Cache/D3D12/fallback.cfxb",
        ] {
            let output = ShaderCacheSourceTransform
                .transform(LegacySourceInput::new(path, &shader_bin_bytes()))
                .unwrap();

            assert_eq!(output.artifact(), None);
            match output {
                LegacySourceOutput::Excluded { reason } => {
                    assert!(reason.contains(&normalize_source_path(path)));
                    assert!(reason.contains("generated product"));
                }
                other => panic!("expected excluded shader binary, got {other:?}"),
            }
        }
    }

    #[test]
    fn excludes_valid_shader_resource_cache() {
        let output = ShaderCacheSourceTransform
            .transform(LegacySourceInput::new(
                "Shaders/Cache/D3D11/CGPShaders/fixedpipelineemu@dummyps.fxcb",
                &resource_cache_bytes(),
            ))
            .unwrap();

        assert_eq!(output.artifact(), None);
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(
                    reason.contains("shaders/cache/d3d11/cgpshaders/fixedpipelineemu@dummyps.fxcb")
                );
                assert!(reason.contains("generated product"));
            }
            other => panic!("expected excluded shader resource cache, got {other:?}"),
        }
    }

    #[test]
    fn extension_family_enforces_native_wire_identity() {
        assert!(
            ShaderCacheSourceTransform
                .transform(LegacySourceInput::new(
                    "Shaders/Cache/D3D11/common.cfib",
                    &resource_cache_bytes(),
                ))
                .is_err()
        );
        assert!(
            ShaderCacheSourceTransform
                .transform(LegacySourceInput::new(
                    "Shaders/Cache/D3D11/CGPShaders/fixedpipelineemu@dummyps.fxcb",
                    &shader_bin_bytes(),
                ))
                .is_err()
        );
    }

    #[test]
    fn excludes_valid_shader_lookup_data() {
        let output = ShaderCacheSourceTransform
            .transform(LegacySourceInput::new(
                "Shaders/Cache/D3D12/lookupdata.bin",
                &lookup_data_bytes(),
            ))
            .unwrap();

        assert_eq!(output.artifact(), None);
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(reason.contains("shaders/cache/d3d12/lookupdata.bin"));
                assert!(reason.contains("generated product"));
            }
            other => panic!("expected excluded shader lookup data, got {other:?}"),
        }
    }

    fn shader_bin_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FXB0");
        bytes.extend_from_slice(&0x9ab9_0fdfu32.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&11u16.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(crate::SHADER_BIN_HEADER_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u32::try_from(crate::SHADER_BIN_HEADER_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0x8ee0_2d75u32.to_le_bytes());
        bytes
    }

    fn resource_cache_bytes() -> Vec<u8> {
        let payload = [0xaa, 0xbb, 0xcc, 0xdd];
        let directory_offset = crate::RESOURCE_HEADER_SIZE + payload.len();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CPCK");
        bytes.extend_from_slice(&ResourceVersion::LZSS_VALUE.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(directory_offset).unwrap().to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&0x80cc_dc7eu32.to_le_bytes());
        bytes.extend_from_slice(
            &(u32::try_from(payload.len()).unwrap() | 0x2800_0000).to_le_bytes(),
        );
        bytes.extend_from_slice(
            &i32::try_from(crate::RESOURCE_HEADER_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        bytes
    }

    fn lookup_data_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(*LOOKUP_DATA_MAGIC);
        bytes.extend(ResourceVersion::LZSS_VALUE.to_le_bytes());
        bytes.extend(cache_version_bytes("Ver: 11.0"));
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(0x1122_3344u32.to_le_bytes());
        bytes.extend(2i32.to_le_bytes());
        bytes.extend(3i32.to_le_bytes());
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(11u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(0xaabb_ccddu32.to_le_bytes());
        bytes.extend(0x5566_7788u32.to_le_bytes());
        bytes
    }

    fn cache_version_bytes(value: &str) -> [u8; LOOKUP_DATA_CACHE_VERSION_SIZE] {
        let mut bytes = [0; LOOKUP_DATA_CACHE_VERSION_SIZE];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }
}
