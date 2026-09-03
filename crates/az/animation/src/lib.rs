//! Native Azoth animation assets and renderer-independent runtime primitives.
//!
//! This crate accepts `.anim.glb` authoring sources and emits runtime motion
//! products. Legacy format importers can lower into this source format without
//! making project-specific animation assets part of the engine surface.

pub mod animation_set;
pub mod blend_space;
pub mod blend_space_asset;
pub mod builder;
pub mod character;
mod character_definition_builder;
pub mod controller_target;
pub mod events;
pub mod mannequin;
pub mod motion;
pub mod playback;
pub mod simple;

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatId, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration, SourceSchemaType, product_format_id, source_schema_type,
};
use az_core::{AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo};
use uuid::{Uuid, uuid};

pub const ANIMATION_SOURCE_SCHEMA_NAME: &str = "azoth.animation.AnimationSource";
pub const MOTION_GLTF_PRODUCT_FORMAT_ID: ProductFormatId =
    product_format_id::<AnimationRuntimeGltfProductFormat>();
pub const ANIMATION_SOURCE_SCHEMA: SourceSchemaType = source_schema_type::<AnimationSourceFormat>();
pub const ANIMATION_ASSET_STABLE_NAME: &str = "azoth.animation.motion";
pub const ANIMATION_ASSET_TYPE: AssetType = AnimationAssetData::ASSET_TYPE;
pub const ANIMATION_EVENT_DATABASE_PRODUCT_FORMAT_ID: ProductFormatId =
    product_format_id::<events::AnimationEventDatabaseProductFormat>();
pub const ANIMATION_EVENT_LIST_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<events::AnimationEventListSourceFormat>();
pub const CHARACTER_DEFINITION_PRODUCT_FORMAT_ID: ProductFormatId =
    product_format_id::<character::definition::CharacterDefinitionProductFormat>();
pub const CHARACTER_DEFINITION_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<character::definition::CharacterDefinitionSourceFormat>();

#[derive(bevy_asset::Asset, bevy_reflect::TypePath)]
pub struct AnimationAssetData;

impl AzTypeInfo for AnimationAssetData {
    const NAME: &'static str = "Azoth::AnimationAsset";
    const TYPE_ID: Uuid = uuid!("496d15eb-0ea7-4d78-86b1-848a1a5ff12b");
}

impl AzRtti for AnimationAssetData {}

impl AssetData for AnimationAssetData {
    const STABLE_NAME: &'static str = ANIMATION_ASSET_STABLE_NAME;
}

#[derive(SourceFormat)]
#[source(schema = "azoth.animation.AnimationSource", ext = "anim.glb")]
pub struct AnimationSourceFormat;

#[derive(ProductFormat)]
#[product_format(id = "azoth.animation.motion-gltf", version = 1, asset = AnimationAssetData)]
pub struct AnimationRuntimeGltfProductFormat;

pub mod ids {
    use az_core::AssetType;

    use super::{ANIMATION_ASSET_TYPE, AssetData, blend_space_asset, character, events};

    pub const ANIMATION: AssetType = ANIMATION_ASSET_TYPE;
    pub const ANIMATION_EVENTS: AssetType = events::AnimationEventDatabaseAsset::ASSET_TYPE;
    pub const BLEND_SPACE: AssetType = blend_space_asset::BlendSpaceAsset::ASSET_TYPE;
    pub const COMBINED_BLEND_SPACE: AssetType =
        blend_space_asset::CombinedBlendSpaceAsset::ASSET_TYPE;
    pub const CHARACTER_DEFINITION: AssetType =
        character::definition::CharacterDefinitionAsset::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{
        ANIMATION_EVENT_LIST_SOURCE_SCHEMA, ANIMATION_SOURCE_SCHEMA,
        CHARACTER_DEFINITION_SOURCE_SCHEMA, SourceSchemaType, blend_space_asset,
        source_schema_type,
    };

    pub const ANIMATION: SourceSchemaType = ANIMATION_SOURCE_SCHEMA;
    pub const ANIMATION_EVENTS: SourceSchemaType = ANIMATION_EVENT_LIST_SOURCE_SCHEMA;
    pub const BLEND_SPACE: SourceSchemaType =
        source_schema_type::<blend_space_asset::BlendSpaceSourceFormat>();
    pub const COMBINED_BLEND_SPACE: SourceSchemaType =
        source_schema_type::<blend_space_asset::CombinedBlendSpaceSourceFormat>();
    pub const CHARACTER_DEFINITION: SourceSchemaType = CHARACTER_DEFINITION_SOURCE_SCHEMA;
}

pub mod product_formats {
    use super::{
        ANIMATION_EVENT_DATABASE_PRODUCT_FORMAT_ID, CHARACTER_DEFINITION_PRODUCT_FORMAT_ID,
        MOTION_GLTF_PRODUCT_FORMAT_ID, ProductFormatId, blend_space_asset, product_format_id,
    };

    pub const RUNTIME_GLTF: ProductFormatId = MOTION_GLTF_PRODUCT_FORMAT_ID;
    pub const ANIMATION_EVENTS: ProductFormatId = ANIMATION_EVENT_DATABASE_PRODUCT_FORMAT_ID;
    pub const BLEND_SPACE: ProductFormatId =
        product_format_id::<blend_space_asset::BlendSpaceProductFormat>();
    pub const COMBINED_BLEND_SPACE: ProductFormatId =
        product_format_id::<blend_space_asset::CombinedBlendSpaceProductFormat>();
    pub const CHARACTER_DEFINITION: ProductFormatId = CHARACTER_DEFINITION_PRODUCT_FORMAT_ID;
}

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 5] {
    [
        AssetTypeRegistration::for_asset::<AnimationAssetData>().with_owner("az-animation"),
        AssetTypeRegistration::for_asset::<events::AnimationEventDatabaseAsset>()
            .with_owner("az-animation"),
        AssetTypeRegistration::for_asset::<character::definition::CharacterDefinitionAsset>()
            .with_owner("az-animation"),
        AssetTypeRegistration::for_asset::<blend_space_asset::BlendSpaceAsset>()
            .with_owner("az-animation"),
        AssetTypeRegistration::for_asset::<blend_space_asset::CombinedBlendSpaceAsset>()
            .with_owner("az-animation"),
    ]
}

/// The product byte contracts this crate owns.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 5] {
    [
        ProductFormatRegistration::for_format::<AnimationRuntimeGltfProductFormat>(),
        ProductFormatRegistration::for_format::<events::AnimationEventDatabaseProductFormat>(),
        ProductFormatRegistration::for_format::<
            character::definition::CharacterDefinitionProductFormat,
        >(),
        ProductFormatRegistration::for_format::<blend_space_asset::BlendSpaceProductFormat>(),
        ProductFormatRegistration::for_format::<blend_space_asset::CombinedBlendSpaceProductFormat>(
        ),
    ]
}

/// The animation authoring schemas this crate owns.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 5] {
    [
        SourceSchemaRegistration::for_source::<AnimationSourceFormat>()
            .with_category("Animation")
            .with_import_file("animations", &["anim.glb"]),
        SourceSchemaRegistration::for_source::<
            character::definition::CharacterDefinitionSourceFormat,
        >()
        .with_category("Animation")
        .with_editable_file("characters", &["character.ron"]),
        SourceSchemaRegistration::for_source::<blend_space_asset::BlendSpaceSourceFormat>()
            .with_label("Blend Space")
            .with_category("Animation")
            .with_editable_file("animations", &["bspace.ron"]),
        SourceSchemaRegistration::for_source::<blend_space_asset::CombinedBlendSpaceSourceFormat>()
            .with_label("Combined Blend Space")
            .with_category("Animation")
            .with_editable_file("animations", &["comb.ron"]),
        SourceSchemaRegistration::for_source::<events::AnimationEventListSourceFormat>()
            .with_label("Animation Events")
            .with_category("Animation")
            .with_editable_file("animations", &["animevents.ron"]),
    ]
}

/// Every animation build rule, gathered from the modules that define them.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 5] {
    [
        BuildRuleRegistration::new(builder::NAME, builder::ID, builder::desc),
        BuildRuleRegistration::new(
            character_definition_builder::NAME,
            character_definition_builder::ID,
            character_definition_builder::desc,
        ),
        BuildRuleRegistration::new(
            blend_space_asset::BLEND_SPACE_BUILDER_NAME,
            blend_space_asset::BLEND_SPACE_BUILDER_ID,
            blend_space_asset::blend_space_build_rule,
        ),
        BuildRuleRegistration::new(
            blend_space_asset::COMBINED_BLEND_SPACE_BUILDER_NAME,
            blend_space_asset::COMBINED_BLEND_SPACE_BUILDER_ID,
            blend_space_asset::combined_blend_space_build_rule,
        ),
        BuildRuleRegistration::new(
            events::ANIMATION_EVENT_LIST_BUILDER_NAME,
            events::ANIMATION_EVENT_LIST_BUILDER_ID,
            events::animation_event_list_build_rule,
        ),
    ]
}

/// The half a runtime resolves: what an animation product *is*.
///
/// Named for the bundle that calls it. This crate's four registration families
/// split across two role-reachabilities and therefore across two engine
/// bundles: a `client` must resolve a `.motion.gltf` product out of an
/// animation asset it never built, while nothing but the pipeline ever asks
/// which source file produced it. One `register` covering both would put the
/// runtime half behind the pipeline's three roles — which is exactly what the
/// deleted link-time anchor used to paper over, by force-linking the whole
/// crate into all six runtime binaries.
pub fn assets<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
}

/// The half only the pipeline reads: how a source becomes one of those
/// products.
///
/// The counterpart to [`assets`], and disjoint from it — each family is
/// registered by exactly one bundle, so composing both halves in a pipeline
/// role is a union, never a duplicate.
pub fn builders<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{
        BuildRuleRegistry, JobContext, composed_product_format, composed_source_schemas,
    };
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.animation-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    declare_caps!(AnimationCaps:);

    struct Animation;

    impl Contribution for Animation {
        type Caps = AnimationCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        /// Both halves, because the fixture stands in for an asset-worker —
        /// the one role that composes the `assets` and `builders` bundles
        /// together.
        fn register(&self, ctx: &mut GemContext<'_, AnimationCaps>) {
            assets(ctx);
            builders(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Animation, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn animation_registrations_use_engine_owned_typed_formats() {
        assert_eq!(
            source_schemas::ANIMATION.as_str(),
            ANIMATION_SOURCE_SCHEMA_NAME
        );
        assert_eq!(
            product_formats::RUNTIME_GLTF.as_str(),
            "azoth.animation.motion-gltf"
        );
        assert_eq!(AnimationAssetData::STABLE_NAME, ANIMATION_ASSET_STABLE_NAME);

        let composer = composed();
        let registries = composer.registries();

        let source_schema = composed_source_schemas(registries)
            .into_iter()
            .find(|schema| schema.entry.schema_type() == source_schemas::ANIMATION)
            .expect("animation source schema");
        assert_eq!(source_schema.entry.category(), "Animation");

        assert!(
            composed_product_format(registries, product_formats::RUNTIME_GLTF).is_some(),
            "animation product format is composed"
        );
    }

    #[test]
    fn every_animation_family_composes_whole() {
        let report = composed().finalize().expect("composition is valid");
        let counts = |registry: &str| {
            report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .count()
        };

        assert_eq!(counts("asset-type"), asset_types().len());
        assert_eq!(counts("product-format"), product_formats().len());
        assert_eq!(counts("source-schema"), source_schemas().len());
        assert_eq!(counts("build-rule"), build_rules().len());
        assert!(report.entries.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.animation-tests"
                && entry.instance.contribution.as_str() == "assets"
        }));
    }

    /// The registration's name and id are the compose key and the job key; the
    /// factory sets them again on the resolved rule. Nothing forces the two to
    /// agree, and a disagreement would file jobs under a rule that does not
    /// exist, so assert it here rather than trusting the literals.
    #[test]
    fn every_registration_names_the_rule_its_factory_builds() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }

    /// Every rule here shares the default priority, so dispatch order is the
    /// name order — deterministic, and independent of the order `build_rules`
    /// happens to list them in.
    #[test]
    fn every_animation_rule_dispatches_from_a_composed_host() {
        let composer = composed();
        let registry = BuildRuleRegistry::compose(&JobContext::new(composer.registries()));
        let names = registry.iter().map(|rule| rule.name).collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                builder::NAME,
                blend_space_asset::BLEND_SPACE_BUILDER_NAME,
                character_definition_builder::NAME,
                blend_space_asset::COMBINED_BLEND_SPACE_BUILDER_NAME,
                events::ANIMATION_EVENT_LIST_BUILDER_NAME,
            ]
        );
        assert_eq!(
            registry.by_id(builder::ID).expect("animation rule").name,
            "azoth.animation"
        );
    }
}
