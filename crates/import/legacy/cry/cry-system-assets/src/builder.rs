//! `AssetBuilder` adapter for native `CrySystem` config TOML sources.

use std::str;

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
    normalize_source_path,
};
use uuid::uuid;

use crate::{ConfigProductFormat, ConfigSource, ConfigSourceFormat};

pub const NAME: &str = "cry-system.config";
pub const ID: BuilderId = BuilderId::new(uuid!("9fa37485-6182-42dd-e54e-b3d046728044"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<ConfigSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<ConfigProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(req: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let jobs = req
        .platforms
        .iter()
        .copied()
        .map(JobDescriptor::default_for_platform)
        .collect();
    CreateJobsResponse {
        jobs,
        ..CreateJobsResponse::default()
    }
}

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product = match transform_product(&req.source_path, req.source_bytes) {
        Ok(product) => product,
        Err(err) => {
            tracing::warn!(source = %req.source_path, error = %err, "config product failed");
            return ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            };
        }
    };

    ProcessJobResponse {
        products: vec![product],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

/// Validates a config authoring source and emits its build product.
///
/// The source bytes are passed through verbatim; deserializing is a validation
/// step, not a re-encode.
///
/// # Errors
///
/// Returns [`ConfigProductError::InvalidUtf8`] if `source_bytes` is not valid
/// UTF-8, or [`ConfigProductError::DeserializeToml`] if it is not a
/// well-formed `ConfigSource` TOML document — a syntax error, an unknown or
/// missing field, or a value whose type does not match its field.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, ConfigProductError> {
    let _source = parse_config_source(source_bytes)?;
    Ok(TypedBuildProduct::<ConfigProductFormat>::from_trusted_path(
        config_product_path(source_path),
        0,
        source_bytes.to_vec(),
    )
    .erase())
}

fn parse_config_source(source_bytes: &[u8]) -> Result<ConfigSource, ConfigProductError> {
    let source = str::from_utf8(source_bytes)?;
    toml::from_str(source).map_err(ConfigProductError::DeserializeToml)
}

fn config_product_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    normalized
        .strip_suffix(".config.toml")
        .map(|stem| format!("{stem}.config.bin"))
        .unwrap_or(normalized)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigProductError {
    #[error("config source is not UTF-8: {0}")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("deserialize config source TOML: {0}")]
    DeserializeToml(toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_native_config_toml_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("config/game.config.toml"));
        assert!(desc.matches("config/cvargroups/sys_spec.config.toml"));
        assert!(!desc.matches("config/game.cfg"));
        assert!(!desc.matches("config/cvargroups/sys_spec.ini"));
        assert!(!desc.matches("levels/foo/levelinfo.xml"));
        assert!(!desc.matches("ui/button.sprite"));
        assert!(!desc.matches("assetcatalog.catalog"));
        assert!(!desc.matches("libs/gameaudio/wwise/audio_tag_data.csv"));
        assert!(!desc.matches("sharedassets/example/world.json"));
        assert!(!desc.matches("levels/foo/resourcelist.txt"));
    }

    #[test]
    fn transform_product_validates_native_config_toml() {
        let source = br#"source_path = "config/game.cfg"

[[lines]]
kind = "assignment"
key = "sys_game_name"
value = "\"Example Game\""
"#;

        let product = transform_product("config/game.config.toml", source).unwrap();

        assert_eq!(product.product_path.as_str(), "config/game.config.bin");
        assert_eq!(product.asset_type, az_core::asset::ids::CONFIG);
        assert_eq!(product.format, crate::product_formats::CRY_SYSTEM_CONFIG);
        assert_eq!(product.bytes, source);
        assert!(transform_product("config/bad.config.toml", b"not = [").is_err());
    }
}
