//! `AssetBuilder` adapter for Cry/Lumberyard DDS texture sources.

use std::sync::OnceLock;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse,
    JobContext, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
    ProductFormat, ProductFormatPolicy, TypedBuildProduct,
};
use az_filesystem::engine_path;
use uuid::uuid;

pub const NAME: &str = "cry.texture";
pub const ID: BuilderId = BuilderId::new(uuid!("7d815263-4f6e-40bb-c329-91be6f405d73"));
pub const VERSION: u32 = 1;

fn patterns() -> &'static [AssetBuilderPattern] {
    static PATTERNS: OnceLock<Vec<AssetBuilderPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| vec![AssetBuilderPattern::wildcard("*.dds")])
}

/// The DDS texture rule, bound to the product format `P` that carries the
/// transcoded bytes.
///
/// This crate parses DDS but owns no product format of its own, so it exposes
/// the rule as a template: the crate that owns the texture product format
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
    TypedBuildProduct::<P>::from_trusted_path(
        engine_path(source_path, "textures", "dds"),
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

    struct TestTextureAsset;

    impl AzTypeInfo for TestTextureAsset {
        const NAME: &'static str = "CryDdsTests::TestTextureAsset";
        const TYPE_ID: Uuid = uuid!("33333333-3333-4333-8333-333333333333");
    }

    impl AzRtti for TestTextureAsset {}

    impl AssetData for TestTextureAsset {
        const STABLE_NAME: &'static str = "az.test.cry.dds-texture";
    }

    #[derive(ProductFormat)]
    #[product_format(id = "az.test.cry.dds-texture", version = 1, asset = TestTextureAsset)]
    struct TestTextureProductFormat;

    #[test]
    fn descriptor_claims_dds_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc::<TestTextureProductFormat>(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("textures/grass.dds"));
        assert!(!desc.matches("textures/grass.png"));
        assert_eq!(
            desc.product_formats.declared(),
            Some(&[<TestTextureProductFormat as ProductFormat>::ID][..])
        );
    }

    #[test]
    fn instantiated_rule_is_a_build_rule_factory() {
        let factory: az_asset_builder::BuildRuleFactory = desc::<TestTextureProductFormat>;
        let registration = az_asset_builder::BuildRuleRegistration::new(NAME, ID, factory);

        assert_eq!(registration.name(), NAME);
        assert_eq!(registration.id(), ID);
    }
}
