//! `AssetBuilder` adapter for Cry renderer shader cache sources.

use std::sync::OnceLock;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse,
    JobContext, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
    ProductFormat, ProductFormatPolicy, TypedBuildProduct, normalize_source_path,
};
use uuid::uuid;

pub const NAME: &str = "cry.shader-cache";
pub const ID: BuilderId = BuilderId::new(uuid!("84f9c9a1-786a-475d-a7c2-b0d2ef2b0f24"));
pub const VERSION: u32 = 1;

fn patterns() -> &'static [AssetBuilderPattern] {
    static PATTERNS: OnceLock<Vec<AssetBuilderPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            AssetBuilderPattern::regex(r"^shaders/[^/]+\.ext$")
                .expect("valid shader extension regex"),
            AssetBuilderPattern::regex(r"^shaders/cache/.+\.(cfib|cfxb|fxcb)$")
                .expect("valid shader cache regex"),
            AssetBuilderPattern::regex(r"^shaders/cache/.+/lookupdata\.bin$")
                .expect("valid shader lookup regex"),
        ]
    })
}

/// The shader-cache rule, bound to the product format `P` that carries the
/// cache blob bytes.
///
/// This crate parses shader caches but owns no product format of its own, so
/// it exposes the rule as a template: the crate that owns the shader-cache
/// product format instantiates `desc::<ItsFormat>` and contributes the
/// resulting `fn(&JobContext<'_>) -> BuildRule` through its own build-rule
/// registrar.
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
    TypedBuildProduct::<P>::from_trusted_path(
        normalize_source_path(source_path),
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

    struct TestShaderCacheAsset;

    impl AzTypeInfo for TestShaderCacheAsset {
        const NAME: &'static str = "CryShaderCacheTests::TestShaderCacheAsset";
        const TYPE_ID: Uuid = uuid!("55555555-5555-4555-8555-555555555555");
    }

    impl AzRtti for TestShaderCacheAsset {}

    impl AssetData for TestShaderCacheAsset {
        const STABLE_NAME: &'static str = "az.test.cry.shader-cache";
    }

    #[derive(ProductFormat)]
    #[product_format(id = "az.test.cry.shader-cache", version = 1, asset = TestShaderCacheAsset)]
    struct TestShaderCacheProductFormat;

    #[test]
    fn patterns_match_shader_cache_blobs_only() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc::<TestShaderCacheProductFormat>(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert_eq!(
            desc.product_formats.declared(),
            Some(&[<TestShaderCacheProductFormat as ProductFormat>::ID][..])
        );
        assert!(desc.matches("shaders/alphacutout.ext"));
        assert!(!desc.matches("materials/alphacutout.ext"));
        assert!(desc.matches("shaders/cache/d3d11/common.cfib"));
        assert!(desc.matches("shaders/cache/d3d12/fallback.cfxb"));
        assert!(desc.matches("shaders/cache/d3d11/cgpshaders/fixedpipelineemu@dummyps.fxcb"));
        assert!(desc.matches("shaders/cache/d3d12/lookupdata.bin"));
        assert!(!desc.matches("sounds/wwise/lookupdata.bin"));
    }

    #[test]
    fn instantiated_rule_is_a_build_rule_factory() {
        let factory: az_asset_builder::BuildRuleFactory = desc::<TestShaderCacheProductFormat>;
        let registration = az_asset_builder::BuildRuleRegistration::new(NAME, ID, factory);

        assert_eq!(registration.name(), NAME);
        assert_eq!(registration.id(), ID);
    }
}
