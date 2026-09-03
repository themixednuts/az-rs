//! Typed source and product descriptors for asset-builder authoring.
//!
//! Registries, RPC, and the asset DB still expose string/UUID wire identities.
//! Builder authors define those identities once on Rust descriptor types, then
//! erase them into the existing build-rule and product records.

use std::borrow::Cow;
use std::marker::PhantomData;

use az_core::AssetData;

use crate::builder::{
    BuildRule, BuilderId, ProductFormatPolicy, SourceDependencyRule, SourceMatcher,
};
use crate::job::{CreateJobsFn, ProcessJobFn};
use crate::pattern::AssetBuilderPattern;
use crate::value::{ProductFormatId, SourceSchemaType};

/// Typed descriptor for one source-file or authored-document format.
pub trait SourceFormat {
    /// Source schema identity for schema-aware sources. Raw path-only formats
    /// leave this as `None`.
    const SCHEMA: Option<SourceSchemaType>;
    /// Slice form used by erased `SourceMatcher::Schemas`.
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[];
    /// Source path patterns claimed by this format.
    const PATTERNS: &'static [AssetBuilderPattern];

    #[must_use]
    fn matcher() -> SourceMatcher {
        if Self::SCHEMA_TYPES.is_empty() {
            SourceMatcher::PathOnly(Self::PATTERNS)
        } else {
            SourceMatcher::Schemas {
                patterns: Self::PATTERNS,
                schemas: Cow::Borrowed(Self::SCHEMA_TYPES),
            }
        }
    }
}

/// Typed descriptor for one build-product byte format.
pub trait ProductFormat {
    /// Registered product byte-format id.
    const ID: ProductFormatId;
    /// Current writer version for this byte format.
    const VERSION: u32;
    /// Runtime/editor asset type normally emitted in this product format.
    type Asset: AssetData;
}

/// Closed set of product formats emitted by one typed build rule.
///
/// Most rules use [`BuildRuleSourceBuilder::produces`] for one format. Rules
/// whose source is an authored aggregate use a tuple through
/// [`BuildRuleSourceBuilder::produces_product_set`] so every permitted product
/// family remains visible in the type declaration. This is intentionally
/// distinct from source-driven dynamic products.
pub trait ProductFormatSet {
    /// Product format identities admitted by this set, in declaration order.
    const IDS: &'static [ProductFormatId];
}

macro_rules! product_format_set {
    ($($format:ident),+ $(,)?) => {
        impl<$($format),+> ProductFormatSet for ($($format,)+)
        where
            $($format: ProductFormat),+
        {
            const IDS: &'static [ProductFormatId] = &[$($format::ID),+];
        }
    };
}

product_format_set!(P0);
product_format_set!(P0, P1);
product_format_set!(P0, P1, P2);
product_format_set!(P0, P1, P2, P3);

/// Returns the schema identity declared by a schema-aware source descriptor.
///
/// # Panics
///
/// Panics when `S::SCHEMA` is `None`, which is how raw path-only source formats
/// declare that they carry no source schema identity.
#[must_use]
pub const fn source_schema_type<S: SourceFormat>() -> SourceSchemaType {
    let Some(schema_type) = S::SCHEMA else {
        panic!("source_schema_type requires SourceFormat::SCHEMA");
    };
    schema_type
}

#[must_use]
pub const fn product_format_id<P: ProductFormat>() -> ProductFormatId {
    P::ID
}

#[must_use]
pub struct BuildRuleSourceBuilder<S>
where
    S: SourceFormat,
{
    name: &'static str,
    id: BuilderId,
    source_dependencies: &'static [SourceDependencyRule],
    version: u32,
    analysis_fingerprint: String,
    _marker: PhantomData<fn() -> S>,
}

#[must_use]
pub struct BuildRuleProductBuilder<S, P>
where
    S: SourceFormat,
    P: ProductFormat,
{
    name: &'static str,
    id: BuilderId,
    source_dependencies: &'static [SourceDependencyRule],
    version: u32,
    analysis_fingerprint: String,
    create_jobs: Option<CreateJobsFn>,
    _marker: PhantomData<fn() -> (S, P)>,
}

#[must_use]
pub struct BuildRuleProductSetBuilder<S, Products>
where
    S: SourceFormat,
    Products: ProductFormatSet,
{
    name: &'static str,
    id: BuilderId,
    source_dependencies: &'static [SourceDependencyRule],
    version: u32,
    analysis_fingerprint: String,
    create_jobs: Option<CreateJobsFn>,
    _marker: PhantomData<fn() -> (S, Products)>,
}

#[must_use]
pub struct BuildRuleDynamicProductBuilder<S>
where
    S: SourceFormat,
{
    name: &'static str,
    id: BuilderId,
    source_dependencies: &'static [SourceDependencyRule],
    version: u32,
    analysis_fingerprint: String,
    create_jobs: Option<CreateJobsFn>,
    _marker: PhantomData<fn() -> S>,
}

impl BuildRule {
    /// Start a typed build-rule declaration for a source descriptor.
    pub fn for_source<S>() -> BuildRuleSourceBuilder<S>
    where
        S: SourceFormat,
    {
        BuildRuleSourceBuilder {
            name: "",
            id: BuilderId::new(uuid::Uuid::nil()),
            source_dependencies: &[],
            version: 0,
            analysis_fingerprint: String::new(),
            _marker: PhantomData,
        }
    }
}

impl<S> BuildRuleSourceBuilder<S>
where
    S: SourceFormat,
{
    pub const fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    pub const fn id(mut self, id: BuilderId) -> Self {
        self.id = id;
        self
    }

    pub const fn source_dependencies(
        mut self,
        dependencies: &'static [SourceDependencyRule],
    ) -> Self {
        self.source_dependencies = dependencies;
        self
    }

    pub const fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// Set the builder-local analysis fingerprint.
    ///
    /// Change this value when analysis or job-creation logic changes without
    /// changing the source or product byte formats. This is deliberately not
    /// inferred from the asset-worker executable: one worker contains many
    /// independent builders. Rules whose products embed contributed types
    /// derive it from the host's composition instead of declaring a constant.
    pub fn analysis_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.analysis_fingerprint = fingerprint.into();
        self
    }

    pub fn produces<P>(self) -> BuildRuleProductBuilder<S, P>
    where
        P: ProductFormat,
    {
        BuildRuleProductBuilder {
            name: self.name,
            id: self.id,
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            create_jobs: None,
            _marker: PhantomData,
        }
    }

    /// Declare the closed product-format set emitted by an aggregate source.
    ///
    /// Use this when one source owns multiple stable product sub-IDs whose
    /// byte formats are known at compile time. Source-selected or
    /// registry-selected formats belong on [`Self::produces_dynamic_products`]
    /// instead.
    pub fn produces_product_set<Products>(self) -> BuildRuleProductSetBuilder<S, Products>
    where
        Products: ProductFormatSet,
    {
        BuildRuleProductSetBuilder {
            name: self.name,
            id: self.id,
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            create_jobs: None,
            _marker: PhantomData,
        }
    }

    /// Declare a typed source rule whose product format is selected from
    /// runtime registry or source payload data.
    ///
    /// Prefer [`Self::produces`] for normal one-source, one-product builders.
    /// This explicit escape hatch keeps multi-product rules searchable without
    /// reintroducing hand-authored product identity strings.
    pub fn produces_dynamic_products(self) -> BuildRuleDynamicProductBuilder<S> {
        BuildRuleDynamicProductBuilder {
            name: self.name,
            id: self.id,
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            create_jobs: None,
            _marker: PhantomData,
        }
    }
}

impl<S, P> BuildRuleProductBuilder<S, P>
where
    S: SourceFormat,
    P: ProductFormat,
{
    pub fn create_jobs(mut self, create_jobs: CreateJobsFn) -> Self {
        self.create_jobs = Some(create_jobs);
        self
    }

    /// Finish the typed rule with its job-processing function.
    ///
    /// # Panics
    ///
    /// Panics when `create_jobs` was never called, since the erased
    /// [`BuildRule`] has no optional job factory to fall back on.
    #[must_use]
    pub fn process(self, process_job: ProcessJobFn) -> BuildRule {
        BuildRule {
            name: self.name,
            id: self.id,
            primary_source: S::matcher(),
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            product_formats: ProductFormatPolicy::Declared(&[P::ID]),
            create_jobs: self
                .create_jobs
                .expect("typed BuildRule declarations must set create_jobs before process"),
            process_job,
        }
    }
}

impl<S, Products> BuildRuleProductSetBuilder<S, Products>
where
    S: SourceFormat,
    Products: ProductFormatSet,
{
    pub fn create_jobs(mut self, create_jobs: CreateJobsFn) -> Self {
        self.create_jobs = Some(create_jobs);
        self
    }

    /// Finish the typed aggregate rule with its job-processing function.
    ///
    /// # Panics
    ///
    /// Panics when `create_jobs` was never called.
    #[must_use]
    pub fn process(self, process_job: ProcessJobFn) -> BuildRule {
        debug_assert!(!Products::IDS.is_empty());
        BuildRule {
            name: self.name,
            id: self.id,
            primary_source: S::matcher(),
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            product_formats: ProductFormatPolicy::Declared(Products::IDS),
            create_jobs: self
                .create_jobs
                .expect("typed BuildRule declarations must set create_jobs before process"),
            process_job,
        }
    }
}

impl<S> BuildRuleDynamicProductBuilder<S>
where
    S: SourceFormat,
{
    pub fn create_jobs(mut self, create_jobs: CreateJobsFn) -> Self {
        self.create_jobs = Some(create_jobs);
        self
    }

    /// Finish the typed dynamic-product rule with its job-processing function.
    ///
    /// # Panics
    ///
    /// Panics when `create_jobs` was never called, since the erased
    /// [`BuildRule`] has no optional job factory to fall back on.
    #[must_use]
    pub fn process(self, process_job: ProcessJobFn) -> BuildRule {
        BuildRule {
            name: self.name,
            id: self.id,
            primary_source: S::matcher(),
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint,
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: self
                .create_jobs
                .expect("typed BuildRule declarations must set create_jobs before process"),
            process_job,
        }
    }
}

#[cfg(test)]
mod tests {
    use az_core::{AssetData, AssetType, AzRtti, AzTypeInfo};
    use uuid::{Uuid, uuid};

    use super::*;
    use crate::{
        BuildProduct, CreateJobsRequest, CreateJobsResponse, ProcessJobRequest, ProcessJobResponse,
        ProductPath, TypedBuildProduct,
    };

    #[derive(crate::SourceFormat)]
    #[source(schema = "az.test.TypedSource", ext = "typed.ron")]
    struct TypedSource;

    struct TypedAsset;

    impl AzTypeInfo for TypedAsset {
        const NAME: &'static str = "AzTest::TypedAsset";
        const TYPE_ID: Uuid = uuid!("11111111-1111-4111-8111-111111111111");
    }

    impl AzRtti for TypedAsset {}

    impl AssetData for TypedAsset {
        const STABLE_NAME: &'static str = "az.test.typed-asset";
    }

    #[derive(crate::ProductFormat)]
    #[product_format(id = "az.test.typed-product", version = 1, asset = TypedAsset)]
    struct TypedProduct;

    #[derive(crate::ProductFormat)]
    #[product_format(id = "az.test.aggregate-product", version = 2, asset = TypedAsset)]
    struct AggregateProduct;

    struct AggregateSource;

    impl SourceFormat for AggregateSource {
        const SCHEMA: Option<SourceSchemaType> = None;
        const SCHEMA_TYPES: &'static [SourceSchemaType] = &[
            SourceSchemaType::__from_static("az.test.AggregateA"),
            SourceSchemaType::__from_static("az.test.AggregateB"),
        ];
        const PATTERNS: &'static [AssetBuilderPattern] =
            &[AssetBuilderPattern::static_wildcard("*.aggregate.ron")];
    }

    fn no_jobs(_: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        CreateJobsResponse::default()
    }

    fn no_process(_: &ProcessJobRequest<'_>) -> ProcessJobResponse {
        ProcessJobResponse::default()
    }

    #[test]
    fn derive_constants_match_attributes() {
        assert_eq!(
            <TypedSource as SourceFormat>::SCHEMA.unwrap().as_str(),
            "az.test.TypedSource"
        );
        assert_eq!(
            <TypedSource as SourceFormat>::SCHEMA_TYPES,
            &[crate::SourceSchemaType::__from_static(
                "az.test.TypedSource"
            )]
        );
        assert_eq!(
            source_schema_type::<TypedSource>().as_str(),
            "az.test.TypedSource"
        );
        assert!(<TypedSource as SourceFormat>::PATTERNS[0].matches("foo.typed.ron"));
        assert_eq!(
            product_format_id::<TypedProduct>().as_str(),
            "az.test.typed-product"
        );
        assert_eq!(<TypedProduct as ProductFormat>::VERSION, 1);
        assert_eq!(
            <<TypedProduct as ProductFormat>::Asset as AssetData>::ASSET_TYPE,
            AssetType::new(uuid!("11111111-1111-4111-8111-111111111111"))
        );
    }

    #[test]
    fn typed_build_rule_produces_expected_erased_shape() {
        const ID: BuilderId = BuilderId::new(uuid!("22222222-2222-4222-8222-222222222222"));
        let typed = BuildRule::for_source::<TypedSource>()
            .named("az.test.typed")
            .id(ID)
            .version(1)
            .produces::<TypedProduct>()
            .create_jobs(no_jobs)
            .process(no_process);
        let erased = BuildRule {
            name: "az.test.typed",
            id: ID,
            primary_source: SourceMatcher::Schemas {
                patterns: <TypedSource as SourceFormat>::PATTERNS,
                schemas: <TypedSource as SourceFormat>::SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Declared(&[TypedProduct::ID]),
            create_jobs: no_jobs,
            process_job: no_process,
        };

        assert_eq!(typed.name, erased.name);
        assert_eq!(typed.id, erased.id);
        assert_eq!(typed.version, erased.version);
        assert_eq!(typed.product_formats, erased.product_formats);
        assert_eq!(
            typed.primary_source.schema_types(),
            erased.primary_source.schema_types()
        );
        assert!(typed.matches_source("foo.typed.ron", Some("az.test.TypedSource")));
    }

    #[test]
    fn typed_dynamic_product_rule_keeps_source_schema_filter() {
        const ID: BuilderId = BuilderId::new(uuid!("33333333-3333-4333-8333-333333333333"));
        let typed = BuildRule::for_source::<AggregateSource>()
            .named("az.test.aggregate")
            .id(ID)
            .version(11)
            .produces_dynamic_products()
            .create_jobs(no_jobs)
            .process(no_process);

        assert_eq!(typed.name, "az.test.aggregate");
        assert_eq!(
            typed.primary_source.schema_types(),
            <AggregateSource as SourceFormat>::SCHEMA_TYPES
        );
        assert!(typed.matches_source("foo.aggregate.ron", Some("az.test.AggregateB")));
        assert!(!typed.matches_source("foo.aggregate.ron", Some("az.test.Unknown")));
    }

    #[test]
    fn typed_product_set_rule_declares_its_closed_formats() {
        const ID: BuilderId = BuilderId::new(uuid!("44444444-4444-4444-8444-444444444444"));
        type Products = (TypedProduct, AggregateProduct);
        let typed = BuildRule::for_source::<AggregateSource>()
            .named("az.test.product-set")
            .id(ID)
            .version(12)
            .produces_product_set::<Products>()
            .create_jobs(no_jobs)
            .process(no_process);

        assert_eq!(
            <Products as ProductFormatSet>::IDS,
            &[TypedProduct::ID, AggregateProduct::ID]
        );
        assert_eq!(typed.id, ID);
        assert!(typed.product_formats.accepts(TypedProduct::ID));
        assert!(typed.product_formats.accepts(AggregateProduct::ID));
        assert!(
            !typed
                .product_formats
                .accepts(ProductFormatId::__from_static("az.test.undeclared-product"))
        );
        assert!(typed.matches_source("foo.aggregate.ron", Some("az.test.AggregateA")));
        assert!(!typed.matches_source("foo.aggregate.ron", Some("az.test.Unknown")));
    }

    #[test]
    fn typed_build_product_fills_identity() {
        let product = TypedBuildProduct::<TypedProduct>::new("out/product.bin", 7, vec![1, 2])
            .unwrap()
            .erase();

        assert_eq!(
            product.product_path,
            ProductPath::from_trusted("out/product.bin")
        );
        assert_eq!(product.asset_type, TypedAsset::asset_type());
        assert_eq!(product.format, <TypedProduct as ProductFormat>::ID);
        assert_eq!(
            product.format_version,
            <TypedProduct as ProductFormat>::VERSION
        );
        assert_eq!(product.sub_id, 7);
        assert_eq!(product.bytes, vec![1, 2]);
    }

    #[test]
    fn dynamic_build_product_escape_hatch_is_explicit() {
        let product = BuildProduct::from_dynamic_parts(
            "out/dynamic.bin",
            TypedAsset::asset_type(),
            <TypedProduct as ProductFormat>::ID,
            <TypedProduct as ProductFormat>::VERSION,
            9,
            Vec::new(),
        )
        .unwrap();

        assert_eq!(product.product_path.as_str(), "out/dynamic.bin");
        assert_eq!(product.sub_id, 9);
    }
}
