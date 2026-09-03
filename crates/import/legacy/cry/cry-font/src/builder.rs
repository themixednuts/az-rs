//! `AssetBuilder` adapter for OpenType font face sources.

use std::sync::OnceLock;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse,
    JobContext, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
    ProductFormat, ProductFormatPolicy, TypedBuildProduct,
};
use az_filesystem::{engine_path, source_extension_key};
use uuid::uuid;

pub const NAME: &str = "cry.font-face";
pub const ID: BuilderId = BuilderId::new(uuid!("d3e7b8c9-a5c5-4611-2982-f704c5a6b322"));
pub const VERSION: u32 = 1;

fn patterns() -> &'static [AssetBuilderPattern] {
    static PATTERNS: OnceLock<Vec<AssetBuilderPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            AssetBuilderPattern::wildcard("*.ttf"),
            AssetBuilderPattern::wildcard("*.otf"),
        ]
    })
}

/// The font-face rule, bound to the product format `P` that carries the face
/// bytes.
///
/// This crate parses font faces but owns no product format of its own, so it
/// exposes the rule as a template: the crate that owns the font product format
/// instantiates `desc::<ItsFormat>` and contributes the resulting
/// `fn(&JobContext<'_>) -> BuildRule` through its own build-rule registrar.
#[must_use]
pub fn desc<P: ProductFormat>(_: &JobContext<'_>) -> BuildRule {
    BuildRule {
        name: NAME,
        id: ID,
        primary_source: az_asset_builder::SourceMatcher::PathOnly(patterns()),
        source_dependencies: &[],
        version: VERSION,
        analysis_fingerprint: String::new(),
        product_formats: ProductFormatPolicy::Declared(&[<P as ProductFormat>::ID]),
        create_jobs,
        process_job: process_job::<P>,
    }
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

fn process_job<P: ProductFormat>(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    ProcessJobResponse {
        products: vec![transform_product::<P>(&req.source_path, req.source_bytes)],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

#[must_use]
pub fn transform_product<P: ProductFormat>(source_path: &str, source_bytes: &[u8]) -> BuildProduct {
    let ext = source_extension_key(source_path).unwrap_or_else(|| "ttf".to_string());
    TypedBuildProduct::<P>::from_trusted_path(
        engine_path(source_path, "fonts", &ext),
        0,
        source_bytes.to_vec(),
    )
    .erase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_core::{AssetData, AzRtti, AzTypeInfo};
    use uuid::Uuid;

    struct TestFontFaceAsset;

    impl AzTypeInfo for TestFontFaceAsset {
        const NAME: &'static str = "CryFontTests::TestFontFaceAsset";
        const TYPE_ID: Uuid = uuid!("44444444-4444-4444-8444-444444444444");
    }

    impl AzRtti for TestFontFaceAsset {}

    impl AssetData for TestFontFaceAsset {
        const STABLE_NAME: &'static str = "az.test.cry.font-face";
    }

    #[derive(ProductFormat)]
    #[product_format(id = "az.test.cry.font-face", version = 1, asset = TestFontFaceAsset)]
    struct TestFontFaceProductFormat;

    #[test]
    fn descriptor_claims_font_face_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc::<TestFontFaceProductFormat>(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("fonts/sans.ttf"));
        assert!(desc.matches("fonts/serif.otf"));
        assert!(!desc.matches("fonts/descriptor.font"));
        assert_eq!(
            desc.product_formats.declared(),
            Some(&[<TestFontFaceProductFormat as ProductFormat>::ID][..])
        );
    }

    #[test]
    fn instantiated_rule_is_a_build_rule_factory() {
        let factory: az_asset_builder::BuildRuleFactory = desc::<TestFontFaceProductFormat>;
        let registration = az_asset_builder::BuildRuleRegistration::new(NAME, ID, factory);

        assert_eq!(registration.name(), NAME);
        assert_eq!(registration.id(), ID);
    }

    #[test]
    fn create_jobs_emits_default_job_for_each_platform() {
        let registries = az_gem_contract::Registries::new();
        let context = JobContext::new(&registries);
        let platforms = [
            az_asset_builder::DEFAULT_PLATFORM_ID,
            az_asset_builder::PlatformId::new_static("server"),
        ];
        let req = CreateJobsRequest::new(
            ID,
            "fonts/sans.ttf",
            std::path::Path::new(""),
            uuid::Uuid::nil(),
            None,
            b"",
            &platforms,
            &context,
        );

        let response = create_jobs(&req);

        assert_eq!(response.jobs.len(), 2);
        assert_eq!(response.jobs[0].platform, "pc");
        assert_eq!(response.jobs[1].platform, "server");
        assert_eq!(response.jobs[0].job_key, "default");
        assert_eq!(response.jobs[1].job_key, "default");
    }
}
