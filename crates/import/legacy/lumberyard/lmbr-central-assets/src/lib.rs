//! Legacy `LmbrCentral` source asset transforms.

pub mod builder;
pub mod material;
mod source_material;
pub mod source_transform;
pub mod tag_component;
pub mod vertex_shape;

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration, product_format_id, source_schema_type,
};
use az_core::{AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo};
use uuid::{Uuid, uuid};

pub use material::{
    MaterialTransformError, material_override_texture_source_paths, material_texture_source_paths,
    parse_material_asset, parse_material_asset_str, parse_material_override_asset,
    parse_material_override_asset_str,
};
pub use source_material::{
    MaterialColorSource, MaterialDefinitionSource, MaterialLinearColorSource,
    MaterialPublicParamSource, MaterialSource, MaterialTextureReferenceSource,
};
pub use source_transform::{
    MaterialSourceTransform, MaterialSourceTransformError, VolumeShapeMetadataSource,
    VolumeShapeReservedSource, VolumeShapeSource, VolumeShapeSourceTransform,
    VolumeShapeSourceTransformError, VolumeShapeVertexSource, is_legacy_volume_shape_source,
    material_schema, material_source_path, volume_shape_source_path,
};
pub use tag_component::{
    EntityTagComponent, TagComponentObjectStreamError, read_entity_tag_component,
    read_entity_tag_components, read_tag_component,
};
pub use vertex_shape::{VertexShapeTransformError, transform_vertex_shape_asset};

pub struct MaterialAssetData;

impl AzTypeInfo for MaterialAssetData {
    const NAME: &'static str = "LmbrCentral::MaterialAsset";
    const TYPE_ID: Uuid = uuid!("f46985b5-f7ff-4fcb-8e8c-dc240d701841");
}

impl AzRtti for MaterialAssetData {}

impl AssetData for MaterialAssetData {
    const STABLE_NAME: &'static str = "az.lmbr-central.material";
}

pub struct MaterialOverrideAssetData;

impl AzTypeInfo for MaterialOverrideAssetData {
    const NAME: &'static str = "Azoth::MaterialOverrideAsset";
    const TYPE_ID: Uuid = uuid!("bcd332a0-d4ab-45af-36ff-7a4c4535690b");
}

impl AzRtti for MaterialOverrideAssetData {}

impl AssetData for MaterialOverrideAssetData {
    const STABLE_NAME: &'static str = "azoth.compat.objectstream.material-override";
}

pub struct VolumeShapeAssetData;

impl AzTypeInfo for VolumeShapeAssetData {
    const NAME: &'static str = "LmbrCentral::VolumeShapeAsset";
    const TYPE_ID: Uuid = uuid!("6f5e9a14-0d2b-4c80-92a3-7f1c8e4d2b56");
}

impl AzRtti for VolumeShapeAssetData {}

impl AssetData for VolumeShapeAssetData {
    const STABLE_NAME: &'static str = "azoth.compat.lmbrcentral.volume-shape";
}

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.lmbrcentral.MaterialSource",
    ext = "material.ron"
)]
pub struct MaterialSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.lmbrcentral.VolumeShapeSource",
    ext = "shape.ron"
)]
pub struct VolumeShapeSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.lmbr-central.material",
    version = 1,
    asset = MaterialAssetData
)]
pub struct MaterialProductFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.lmbr-central.vertex-shape",
    version = 1,
    asset = VolumeShapeAssetData
)]
pub struct VertexShapeProductFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.objectstream.material-override",
    version = 1,
    asset = MaterialOverrideAssetData
)]
pub struct MaterialOverrideProductFormat;

pub mod ids {
    use super::{AssetData, MaterialAssetData, MaterialOverrideAssetData, VolumeShapeAssetData};
    use az_core::AssetType;

    /// `LmbrCentral::MaterialAsset` — Lumberyard reference:
    /// `dev/Gems/LmbrCentral/Code/include/LmbrCentral/Rendering/MaterialAsset.h`.
    pub const MATERIAL_ASSET: AssetType = MaterialAssetData::ASSET_TYPE;

    /// `az_objectstream::MaterialOverrideAsset` (az-rs minted).
    pub const MATERIAL_OVERRIDE: AssetType = MaterialOverrideAssetData::ASSET_TYPE;

    /// `lmbr_central_vshape::VolumeShapeAsset` (az-rs minted).
    pub const VOLUME_SHAPE: AssetType = VolumeShapeAssetData::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{MaterialSourceFormat, VolumeShapeSourceFormat, source_schema_type};
    use az_asset_builder::SourceSchemaType;

    pub const MATERIAL: SourceSchemaType = source_schema_type::<MaterialSourceFormat>();
    pub const VOLUME_SHAPE: SourceSchemaType = source_schema_type::<VolumeShapeSourceFormat>();
}

pub mod product_formats {
    use super::{
        MaterialOverrideProductFormat, MaterialProductFormat, VertexShapeProductFormat,
        product_format_id,
    };
    use az_asset_builder::ProductFormatId;

    /// `LmbrCentral` material product bytes after Azoth normalization.
    pub const LMBR_CENTRAL_MATERIAL: ProductFormatId = product_format_id::<MaterialProductFormat>();

    /// `LmbrCentral` vertex/volume shape product bytes after Azoth normalization.
    pub const LMBR_CENTRAL_VERTEX_SHAPE: ProductFormatId =
        product_format_id::<VertexShapeProductFormat>();

    /// Legacy `ObjectStream` material override product bytes.
    pub const OBJECTSTREAM_MATERIAL_OVERRIDE: ProductFormatId =
        product_format_id::<MaterialOverrideProductFormat>();
}

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 3] {
    [
        AssetTypeRegistration::for_asset::<MaterialAssetData>()
            .with_owner("LmbrCentral/Rendering/MaterialAsset.h"),
        AssetTypeRegistration::for_asset::<MaterialOverrideAssetData>()
            .with_owner("lmbr-central-assets::builder"),
        AssetTypeRegistration::for_asset::<VolumeShapeAssetData>()
            .with_owner("lmbr-central-assets::builder"),
    ]
}

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 3] {
    [
        ProductFormatRegistration::for_format::<MaterialProductFormat>(),
        ProductFormatRegistration::for_format::<VertexShapeProductFormat>(),
        ProductFormatRegistration::for_format::<MaterialOverrideProductFormat>(),
    ]
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 2] {
    [
        SourceSchemaRegistration::for_source::<MaterialSourceFormat>()
            .with_category("LmbrCentral Compatibility")
            .with_editable_file("materials", &["material.ron"]),
        SourceSchemaRegistration::for_source::<VolumeShapeSourceFormat>()
            .with_category("LmbrCentral Compatibility")
            .with_editable_file("shapes", &["shape.ron"]),
    ]
}

/// The build rules this crate owns, for a host contribution to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 2] {
    [
        BuildRuleRegistration::new(
            builder::MATERIAL_NAME,
            builder::MATERIAL_ID,
            builder::material_desc,
        ),
        BuildRuleRegistration::new(
            builder::VOLUME_SHAPE_NAME,
            builder::VOLUME_SHAPE_ID,
            builder::volume_shape_desc,
        ),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
