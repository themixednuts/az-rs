//! `AssetBuilder` adapter for Cry/Lumberyard localization sources.

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::engine_path_with_extension_key;
use ron::error::SpannedError;
use uuid::uuid;

use crate::{LocalizationProductFormat, LocalizationSource, LocalizationSourceFormat};

pub const NAME: &str = "cry.localization";
pub const ID: BuilderId = BuilderId::new(uuid!("7d815263-4f6f-40bb-c32e-91be24f50d22"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<LocalizationSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<LocalizationProductFormat>()
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
            tracing::warn!(source = %req.source_path, error = %err, "localization product failed");
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

/// Validates a localization authoring source and emits its build product.
///
/// The source bytes are passed through verbatim; deserializing is a validation
/// step, not a re-encode.
///
/// # Errors
///
/// Returns [`LocalizationProductError::DeserializeRon`] if `source_bytes` is
/// not a well-formed `LocalizationSource` RON document — a syntax error, an
/// unknown or missing field, or a value whose type does not match its field.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, LocalizationProductError> {
    let _source = ron::de::from_bytes::<LocalizationSource>(source_bytes)?;
    Ok(
        TypedBuildProduct::<LocalizationProductFormat>::from_trusted_path(
            engine_path_with_extension_key(source_path, "localization", "loc.bin", Some("loc.ron")),
            0,
            source_bytes.to_vec(),
        )
        .erase(),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum LocalizationProductError {
    #[error("parse localization source RON: {0}")]
    DeserializeRon(#[from] SpannedError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_localization_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("localization/en-us/main.loc.ron"));
        assert!(!desc.matches("localization/en-us/main.loc"));
        assert!(!desc.matches("localization/en-us/main.loc.xml"));
    }

    #[test]
    fn transform_product_validates_localization_source_ron() {
        let source = br#"(
    source_path: "localization/en-us/main.loc.xml",
    locale: "en-us",
    namespace: "main",
    entries: [
        (
            state: "text",
            key: "Quest_1",
            text: "Hello",
            attributes: [],
        ),
    ],
)"#;

        let product = transform_product("localization/en-us/main.loc.ron", source).unwrap();

        assert_eq!(
            product.product_path.as_str(),
            "localization/localization/en-us/main.loc.bin"
        );
        assert_eq!(product.asset_type, crate::ids::LOCALIZATION);
        assert_eq!(product.format, crate::product_formats::CRY_LOCALIZATION);
        assert_eq!(product.bytes, source);
        assert!(transform_product("localization/en-us/bad.loc.ron", b"not ron").is_err());
    }
}
