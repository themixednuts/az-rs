//! Native Azoth texture authoring-source product builder.
//!
//! This crate is the runtime-facing texture pipeline. It accepts true
//! authoring sources (`.png`, `.exr`) plus `.texture.ron` settings and emits
//! GPU-ready KTX2 products. Legacy DDS decoding intentionally lives in
//! project-specific offline extractor crates.

pub mod builder;
pub mod exr_codec;
mod ktx2_writer;
pub mod png_codec;
pub mod settings;

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFileEditCodecContext,
    SourceFileEditCodecRegistration, SourceFileEditDocument, SourceFileEditError,
    SourceFileEditLoadRequest, SourceFileEditLoadResult, SourceFileEditSaveRequest,
    SourceFileEditSaveResult, SourceFormat, SourceSchemaRegistration,
};
use az_core::{
    AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo, ReflectedValueEncoding,
    ReflectedValueEnvelope,
};
use uuid::{Uuid, uuid};

pub use az_texture_source::TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME;
pub use exr_codec::{
    DecodedFloatImage, DecodedImage, ExrCodecError, read_rgba8_exr, read_rgba32f_exr,
};
pub use png_codec::{DecodedImage16, DecodedPng, PngCodecError, read_png};
pub use settings::{
    TextureAuthoringFormat, TextureColorSpace, TextureCompressionFormat, TextureCompressionIntent,
    TextureDimension, TextureImageOrder, TextureMipSettings, TextureNormalConvention,
    TextureNormalSemantics, TextureOrmSemantics, TextureRole, TextureSettingsError,
    TextureSettingsWriteError, TextureSourceSettings, TextureSourceShape, read_texture_settings,
    texture_settings_source_path, write_texture_settings,
};

pub struct TextureAssetData;

impl AzTypeInfo for TextureAssetData {
    const NAME: &'static str = "Azoth::TextureAsset";
    const TYPE_ID: Uuid = uuid!("dcfe0778-e3df-4a01-9032-66a21497aa5b");
}

impl AzRtti for TextureAssetData {}

impl AssetData for TextureAssetData {
    const STABLE_NAME: &'static str = "azoth.texture.texture";
}

#[derive(SourceFormat)]
#[source(schema = "azoth.texture.TextureSource", ext = "png", ext = "exr")]
pub struct TextureSourceFormat;

#[derive(SourceFormat)]
#[source(schema = "azoth.texture.TextureSettings", ext = "texture.ron")]
pub struct TextureSettingsSourceFormat;

#[derive(ProductFormat)]
#[product_format(id = "azoth.texture.ktx2", version = 1, asset = TextureAssetData)]
pub struct TextureKtx2ProductFormat;

pub mod ids {
    use az_core::AssetType;

    use super::{AssetData, TextureAssetData};

    pub const TEXTURE: AssetType = TextureAssetData::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{SourceFormat, TextureSettingsSourceFormat, TextureSourceFormat};

    pub const TEXTURE: az_asset_builder::SourceSchemaType =
        match <TextureSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("TextureSourceFormat declares a schema"),
        };
    pub const TEXTURE_SETTINGS: az_asset_builder::SourceSchemaType =
        match <TextureSettingsSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("TextureSettingsSourceFormat declares a schema"),
        };
}

pub mod product_formats {
    use super::{ProductFormat, TextureKtx2ProductFormat};

    pub const KTX2: az_asset_builder::ProductFormatId =
        <TextureKtx2ProductFormat as ProductFormat>::ID;
}

fn load_texture_settings_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    let settings = TextureSourceSettings::from_ron_bytes(request.bytes)
        .map_err(|source| SourceFileEditError::failed(source.to_string()))?;
    Ok(SourceFileEditDocument::new(
        TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME,
        ReflectedValueEnvelope {
            type_path: TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME.to_string(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: settings
                .to_ron_bytes()
                .map_err(|source| SourceFileEditError::failed(source.to_string()))?,
        },
    ))
}

fn save_texture_settings_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    if request.root_schema != TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME {
        return Err(SourceFileEditError::unsupported(format!(
            "texture settings codec cannot save root schema `{}`",
            request.root_schema
        )));
    }
    if request.value.type_path != TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME {
        return Err(SourceFileEditError::failed(format!(
            "texture settings value has type path `{}`",
            request.value.type_path
        )));
    }
    if request.value.encoding != ReflectedValueEncoding::TypedRon {
        return Err(SourceFileEditError::failed(
            "texture settings value is not typed RON",
        ));
    }
    TextureSourceSettings::from_ron_bytes(&request.value.payload)
        .map_err(|source| SourceFileEditError::failed(source.to_string()))?
        .to_ron_bytes()
        .map_err(|source| SourceFileEditError::failed(source.to_string()))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The asset types this crate owns, for a composing host to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<TextureAssetData>().with_owner("az-texture-builder")]
}

/// The product formats this crate owns, for a composing host to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        TextureKtx2ProductFormat,
    >()]
}

/// The build rules this crate owns, for a composing host to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        builder::NAME,
        builder::ID,
        builder::desc,
    )]
}

/// The source schemas this crate owns, for a composing host to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 2] {
    [
        SourceSchemaRegistration::for_source::<TextureSourceFormat>()
            .with_category("Texture")
            .with_editable_file("textures", &["png", "exr"]),
        SourceSchemaRegistration::for_source::<TextureSettingsSourceFormat>()
            .with_label("Texture Settings")
            .with_category("Texture")
            .with_editable_file("textures", &["texture.ron"]),
    ]
}

/// The source-file edit codecs this crate owns, for a composing host to
/// register.
#[must_use]
pub fn source_file_edit_codecs() -> [SourceFileEditCodecRegistration; 1] {
    [SourceFileEditCodecRegistration::for_source::<
        TextureSettingsSourceFormat,
    >(
        load_texture_settings_source, save_texture_settings_source
    )]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<SourceFileEditCodecRegistration>()
        .register_many(source_file_edit_codecs());
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{
        BuildRuleRegistry, JobContext, PROJECT_SOURCE_ROOT, SourceFileEditLoadRequest,
        SourceFileEditSaveRequest, SourceSchemaAuthoring, SourceSchemaType,
        composed_product_format, composed_source_file_edit_codec, composed_source_schemas,
    };
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };

    declare_caps!(TextureCaps:);

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.texture-builder-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    /// This crate contributed the way a host's glue contributes it.
    struct Texture;

    impl Contribution for Texture {
        type Caps = TextureCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, TextureCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Texture, ProductActivation::default())
            .expect("texture registrations require no host capability");
        composer
    }

    fn schema(
        registries: &az_gem_contract::Registries,
        schema_type: SourceSchemaType,
    ) -> az_asset_builder::SourceSchemaRegistration {
        composed_source_schemas(registries)
            .into_iter()
            .find(|registration| registration.entry.schema_type() == schema_type)
            .map(|registration| registration.entry)
            .expect("schema is composed")
    }

    #[test]
    fn texture_registrations_use_typed_formats() {
        assert_eq!(
            source_schemas::TEXTURE.as_str(),
            "azoth.texture.TextureSource"
        );
        assert_eq!(
            source_schemas::TEXTURE_SETTINGS.as_str(),
            "azoth.texture.TextureSettings"
        );
        assert_eq!(product_formats::KTX2.as_str(), "azoth.texture.ktx2");

        let composer = composed();
        let registries = composer.registries();
        let schemas = composed_source_schemas(registries)
            .into_iter()
            .map(|registration| registration.entry.schema_type())
            .collect::<Vec<_>>();

        // `composed_source_schemas` orders by schema type, not contribution
        // order.
        assert_eq!(
            schemas,
            [source_schemas::TEXTURE_SETTINGS, source_schemas::TEXTURE]
        );
        assert_eq!(
            composed_product_format(registries, product_formats::KTX2)
                .expect("ktx2 product format is composed")
                .entry
                .id(),
            product_formats::KTX2
        );
    }

    #[test]
    fn texture_build_rule_is_composed() {
        let composer = composed();
        let registries = composer.registries();
        let registry = BuildRuleRegistry::compose(&JobContext::new(registries));
        let rule = registry
            .iter()
            .find(|rule| rule.name == builder::NAME)
            .expect("texture build rule is composed");

        assert_eq!(rule.id, builder::ID);
        assert!(rule.matches("textures/characters/player/body_d.png"));
    }

    #[test]
    fn texture_source_schemas_are_editable_file_workflows() {
        let composer = composed();
        let registries = composer.registries();
        let texture_registration = schema(registries, source_schemas::TEXTURE);
        assert_eq!(texture_registration.category(), "Texture");
        let SourceSchemaAuthoring::File { workflow } = texture_registration.authoring() else {
            panic!("texture source schema should be file-backed");
        };
        assert_eq!(workflow.source_root(), PROJECT_SOURCE_ROOT);
        assert_eq!(workflow.default_path_prefix(), "textures");
        assert_eq!(workflow.extensions(), &["png", "exr"]);
        assert!(!workflow.can_create());
        assert!(workflow.can_edit());

        let settings_registration = schema(registries, source_schemas::TEXTURE_SETTINGS);
        assert_eq!(settings_registration.label(), "Texture Settings");
        assert_eq!(settings_registration.category(), "Texture");
        let SourceSchemaAuthoring::File { workflow } = settings_registration.authoring() else {
            panic!("texture settings source schema should be file-backed");
        };
        assert_eq!(workflow.source_root(), PROJECT_SOURCE_ROOT);
        assert_eq!(workflow.default_path_prefix(), "textures");
        assert_eq!(workflow.extensions(), &["texture.ron"]);
        assert!(!workflow.can_create());
        assert!(workflow.can_edit());
    }

    /// The texture registrations trace back to the contribution that
    /// registered them. That provenance is what the hand-written owner string
    /// used to approximate, and the compose report is the copy that cannot
    /// drift from the composition it describes.
    #[test]
    fn texture_registrations_are_attributed_to_their_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let counts = |registry: &str| {
            report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .count()
        };

        assert_eq!(counts("product-format"), product_formats().len());
        assert_eq!(counts("source-schema"), source_schemas().len());
        assert_eq!(
            counts("source-file-edit-codec"),
            source_file_edit_codecs().len()
        );
        assert!(report.entries.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.texture-builder-tests"
                && entry.instance.contribution.as_str() == "assets"
        }));
    }

    #[test]
    fn texture_settings_source_edit_codec_round_trips_schema_value() {
        let settings = TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Srgb,
            role: TextureRole::Albedo,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: None,
            shape: None,
        };
        let bytes = settings.to_ron_bytes().unwrap();
        let composer = composed();
        let codec = composed_source_file_edit_codec(
            composer.registries(),
            source_schemas::TEXTURE_SETTINGS,
        )
        .expect("texture settings edit codec is composed")
        .entry;
        let context = SourceFileEditCodecContext::new(composer.registries());

        let document = codec
            .load(
                context,
                &SourceFileEditLoadRequest {
                    schema_type: source_schemas::TEXTURE_SETTINGS,
                    source_path: "textures/brick.texture.ron",
                    bytes: &bytes,
                },
            )
            .unwrap();
        assert_eq!(document.root_schema, TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME);
        let saved = codec
            .save(
                context,
                &SourceFileEditSaveRequest::new(
                    source_schemas::TEXTURE_SETTINGS,
                    "textures/brick.texture.ron",
                    &document.root_schema,
                    &document.value,
                    &document.objects,
                    &document.codec_state,
                ),
            )
            .unwrap();
        assert_eq!(
            TextureSourceSettings::from_ron_bytes(&saved).unwrap(),
            settings
        );
    }
}
