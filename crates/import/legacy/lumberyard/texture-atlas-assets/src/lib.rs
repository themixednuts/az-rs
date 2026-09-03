//! `TextureAtlas` index transformation.

pub mod builder;
pub mod source_transform;

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration, normalize_source_path,
};
use az_core::{AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo};

pub use source_transform::{
    TextureAtlasSource, TextureAtlasSourceRegion, TextureAtlasSourceTransform,
    TextureAtlasSourceTransformError, texture_atlas_schema, texture_atlas_source_path,
};

use az_asset::{EngineTextureFormat, engine_texture_asset_path, same_stem_texture_asset_path};
use az_gem_texture_atlas::{
    NameRange, TextureAtlasAsset, TextureAtlasAssetFormatError, TextureAtlasEntry,
};
use bevy::image::TextureAtlasLayout;
use texture_atlas::{TextureAtlasParseError, visit_texture_atlas_index};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub struct TextureAtlasAssetData;

impl AzTypeInfo for TextureAtlasAssetData {
    const NAME: &'static str = "TextureAtlas::TextureAtlasAsset";
    const TYPE_ID: Uuid = uuid!("567dd24a-7e4b-4f5f-d09f-14e6fcdf0805");
}

impl AzRtti for TextureAtlasAssetData {}

impl AssetData for TextureAtlasAssetData {
    const STABLE_NAME: &'static str = "azoth.compat.texture-atlas.texture-atlas";
}

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.texture-atlas.TextureAtlasSource",
    ext = "atlas.ron"
)]
pub struct TextureAtlasSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.texture-atlas.texture-atlas",
    version = 1,
    asset = TextureAtlasAssetData
)]
pub struct TextureAtlasProductFormat;

pub mod ids {
    use super::{AssetData, TextureAtlasAssetData};
    use az_core::AssetType;

    /// `TextureAtlasAsset` (`.texatlasidx`, az-rs minted).
    pub const TEXTURE_ATLAS: AssetType = TextureAtlasAssetData::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{SourceFormat, TextureAtlasSourceFormat};

    pub const TEXTURE_ATLAS: az_asset_builder::SourceSchemaType =
        match <TextureAtlasSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("TextureAtlasSourceFormat declares a schema"),
        };
}

pub mod product_formats {
    use super::{ProductFormat, TextureAtlasProductFormat};

    /// Texture atlas product bytes.
    pub const TEXTURE_ATLAS_PRODUCT: az_asset_builder::ProductFormatId =
        <TextureAtlasProductFormat as ProductFormat>::ID;
}

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<TextureAtlasAssetData>()
        .with_owner("texture-atlas-assets::builder")]
}

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        TextureAtlasProductFormat,
    >()]
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [
        SourceSchemaRegistration::for_source::<TextureAtlasSourceFormat>()
            .with_category("TextureAtlas Compatibility")
            .with_import_file("textures", &["atlas.ron"]),
    ]
}

/// The build rules this crate owns, for a host contribution to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        builder::NAME,
        builder::ID,
        builder::desc,
    )]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

/// Converts a legacy `.texatlasidx` index straight into the runtime atlas
/// asset, skipping the authoring-source round trip.
///
/// # Errors
///
/// Returns [`TextureAtlasTransformError::Parse`] for a malformed index —
/// non-UTF-8 bytes, malformed XML, an unsupported version, a disallowed
/// element or attribute, or a bad region coordinate;
/// [`TextureAtlasTransformError::TooLarge`] if a region name offset or length
/// exceeds `u32`; [`TextureAtlasTransformError::CoordinateOverflow`] if a
/// region's base plus size exceeds `u32`; and
/// [`TextureAtlasTransformError::Format`] if the assembled layout is rejected.
pub fn transform_texture_atlas_index(
    bytes: &[u8],
    source_path: &str,
    texture_format: EngineTextureFormat,
) -> Result<TextureAtlasAsset, TextureAtlasTransformError> {
    let mut names = String::new();
    let mut ranges = Vec::new();
    let mut textures = Vec::new();
    let image_path = texture_atlas_image_engine_path(source_path, texture_format);

    let stats = visit_texture_atlas_index(bytes, |region| {
        let start = names.len();
        names.push_str(&texture_region_engine_key(region.name, texture_format));
        ranges.push((start, names.len() - start));
        textures.push(region.rect);
    })?;

    let entries = ranges
        .into_iter()
        .map(|(start, len)| {
            Ok(TextureAtlasEntry::new(NameRange::new(
                checked_u32(start, "texture-atlas name offset")?,
                checked_u32(len, "texture-atlas name length")?,
            )))
        })
        .collect::<Result<Vec<_>, TextureAtlasTransformError>>()?
        .into_boxed_slice();

    TextureAtlasAsset::new(
        image_path.into_boxed_str(),
        TextureAtlasLayout {
            size: stats.size,
            textures,
        },
        names.into_boxed_str(),
        entries,
    )
    .map_err(TextureAtlasTransformError::Format)
}

pub(crate) fn texture_atlas_image_engine_path(
    source_path: &str,
    texture_format: EngineTextureFormat,
) -> String {
    // Identity-on-pak-source — atlases reference their backing
    // texture by pak source path. `.texatlasidx` companions sit
    // beside a same-stem `.dds`.
    routed_texture_source_path(&same_stem_texture_asset_path(
        source_path,
        ".texatlasidx",
        texture_format,
    ))
}

pub(crate) fn texture_region_engine_key(name: &str, texture_format: EngineTextureFormat) -> String {
    // Atlas region keys are looked up by stem (no extension); we
    // normalise the name and strip any source extension so the
    // catalog lookup matches whether the authored cell name has
    // `.dds` or not.
    let source_path = routed_texture_source_path(&engine_texture_asset_path(name, texture_format));
    strip_extension(&source_path).to_owned()
}

fn routed_texture_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    if let Some(rest) = normalized.strip_prefix("lyshineui/") {
        return format!("ui/lyshineui/{rest}");
    }
    if normalized.starts_with("textures/") {
        return normalized;
    }
    format!("textures/{normalized}")
}

fn strip_extension(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(base, _)| base)
}

fn checked_u32(value: usize, what: &'static str) -> Result<u32, TextureAtlasTransformError> {
    u32::try_from(value).map_err(|_| TextureAtlasTransformError::TooLarge { what, value })
}

#[derive(Debug, Error)]
pub enum TextureAtlasTransformError {
    #[error("parse TextureAtlas index: {0}")]
    Parse(#[from] TextureAtlasParseError),
    #[error("{what} {value} exceeds u32")]
    TooLarge { what: &'static str, value: usize },
    #[error("atlas coordinate `{field}` overflowed: {base} + {size}")]
    CoordinateOverflow {
        field: &'static str,
        base: u32,
        size: u32,
    },
    #[error("texture-atlas asset format: {0}")]
    Format(#[from] TextureAtlasAssetFormatError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    /// The registration is keyed on the builder id and ordered by the name, so
    /// a registration that disagrees with the rule it resolves would file job
    /// attempts under an identity the dispatcher never reports.
    #[test]
    fn every_registration_matches_the_rule_it_resolves() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }

    #[test]
    fn atlas_image_path_uses_same_stem_texture_product() {
        assert_eq!(
            texture_atlas_image_engine_path(
                "LyShineUI/Images/TextureAtlas/Chat.texatlasidx",
                EngineTextureFormat::Dds,
            ),
            "ui/lyshineui/images/textureatlas/chat.dds"
        );
        assert_eq!(
            texture_atlas_image_engine_path(
                "LyShineUI/Images/TextureAtlas/Chat.texatlasidx",
                EngineTextureFormat::Ktx2,
            ),
            "ui/lyshineui/images/textureatlas/chat.ktx2"
        );
    }

    #[test]
    fn atlas_region_names_use_texture_product_keys() {
        assert_eq!(
            texture_region_engine_key("LyShineUI/Images/Icon", EngineTextureFormat::Dds),
            "ui/lyshineui/images/icon"
        );
        assert_eq!(
            texture_region_engine_key(
                "textures/LyShineUI/Images/Icon.dds",
                EngineTextureFormat::Ktx2
            ),
            "textures/lyshineui/images/icon"
        );
    }

    #[test]
    fn source_transform_emits_atlas_ron_authoring_source() {
        let output = TextureAtlasSourceTransform::new(EngineTextureFormat::Ktx2)
            .transform(LegacySourceInput::new(
                "LyShineUI/Images/TextureAtlas/Chat.texatlasidx",
                atlas_xml(),
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("texture-atlas transform should emit native source");
        };
        assert_eq!(
            artifact.path,
            "ui/lyshineui/images/textureatlas/chat.atlas.ron"
        );
        assert_eq!(artifact.schema, source_schemas::TEXTURE_ATLAS);

        let source: TextureAtlasSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(
            source.source_path,
            "lyshineui/images/textureatlas/chat.texatlasidx"
        );
        assert_eq!(source.image, "ui/lyshineui/images/textureatlas/chat.ktx2");
        assert_eq!(source.size, [24, 12]);
        assert_eq!(
            source.regions,
            vec![TextureAtlasSourceRegion {
                name: "ui/lyshineui/images/icon".to_string(),
                left: 17,
                top: 0,
                width: 4,
                height: 4,
            }]
        );
    }

    fn atlas_xml() -> &'static [u8] {
        br#"<ObjectStream version="3">
            <Class name="TextureAtlasImpl" version="1" type="{2CA51C61-1B5F-4480-A257-F28D8944AA35}">
                <Class name="AZStd::unordered_map" field="Coordinate Pairs" type="{23D80BE7-76FE-5C35-B4EA-A4492B2B058C}">
                    <Class name="AZStd::pair" field="element" type="{657F8527-0117-54A0-91E7-6E67A7464BC5}">
                        <Class name="AZStd::string" field="value1" value="LyShineUI/Images/Icon" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
                        <Class name="AtlasCoordinates" field="value2" version="1" type="{FC5D6A60-1056-4F6C-96F7-6A47912F8A35}">
                            <Class name="int" field="Left" value="17" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Top" value="0" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Width" value="4" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Height" value="4" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                        </Class>
                    </Class>
                </Class>
                <Class name="int" field="Width" value="24" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                <Class name="int" field="Height" value="12" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
            </Class>
        </ObjectStream>"#
    }
}
