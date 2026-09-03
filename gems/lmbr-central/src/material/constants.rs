//! Material asset identifiers and file filters.

use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::MaterialAsset` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:29`.
pub const MATERIAL_ASSET_TYPE_ID: Uuid = uuid!("F46985B5-F7FF-4FCB-8E8C-DC240D701841");

/// Lumberyard `LmbrCentral::DccMaterialAsset` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:42`.
pub const DCC_MATERIAL_ASSET_TYPE_ID: Uuid = uuid!("C88469CF-21E7-41EB-96FD-BF14FBB05EDC");

/// Lumberyard `LmbrCentral::TextureAsset` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:55`.
pub const TEXTURE_ASSET_TYPE_ID: Uuid = uuid!("59D5E20B-34DB-4D8E-B867-D33CC2556355");

/// `MB::MaterialOverrideAsset` type UUID.
pub const MATERIAL_OVERRIDE_ASSET_TYPE_ID: Uuid = uuid!("5A8C903D-69F3-4259-8E31-9CB04867BD6E");

/// `AzFramework::SimpleAssetReference<LmbrCentral::MaterialAsset>` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:66`.
pub const SIMPLE_MATERIAL_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("B7B8ECC7-FF89-4A76-A50E-4C6CA2B6E6B4");

/// `AzFramework::SimpleAssetReference<LmbrCentral::DccMaterialAsset>` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:67`.
pub const SIMPLE_DCC_MATERIAL_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("E865C742-A063-47A3-BCE1-E724A8D4B66D");

/// `AzFramework::SimpleAssetReference<LmbrCentral::TextureAsset>` type UUID.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/MaterialAsset.h:68`.
pub const SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("68E92460-5C0C-4031-9620-6F1A08763243");

/// `AzFramework::SimpleAssetReference<MB::MaterialOverrideAsset>` type UUID.
pub const SIMPLE_MATERIAL_OVERRIDE_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("19ED7B41-FB10-4DF5-A7FD-697182186D7F");

/// Lumberyard material component type UUID.
pub const MATERIAL_COMPONENT_TYPE_ID: Uuid = uuid!("BA3890BD-D2E7-4DB6-95CD-7E7D5525567A");

/// Source file filters accepted by Lumberyard `MaterialAsset`.
pub const MATERIAL_ASSET_FILE_FILTERS: &[&str] = &["mtl"];

/// Source file filters accepted by Lumberyard `DccMaterialAsset`.
pub const DCC_MATERIAL_ASSET_FILE_FILTERS: &[&str] = &["dccmtl"];

/// Source file filters accepted by Lumberyard `TextureAsset`.
pub const TEXTURE_ASSET_FILE_FILTERS: &[&str] = &[
    "dds", "tif", "bmp", "gif", "jpg", "jpeg", "jpe", "tga", "png", "swf", "gfx", "sfd", "sprite",
];

/// File extensions handled by the material asset loader.
///
/// The on-disk product preserves the source-path extension (`.mtl`)
/// because the extractor lays everything out at its vanilla pak
/// source path. The transformed binary payload still uses our
/// `MaterialAsset` format. Only the filename keeps the legacy extension so
/// catalog and filesystem paths remain identical.
pub const MATERIAL_ENGINE_ASSET_EXTENSIONS: &[&str] = &["mtl"];

/// File extensions handled by the material override asset loader.
///
/// Material overrides ship as XML in `libs/materialoverrides/…` —
/// we honour the vanilla pak extension. Disambiguation against
/// other XML asset kinds is the `AssetCatalog`'s job
/// (`AssetType` UUID lookup) once the catalog-driven runtime
/// pipeline lands.
pub const MATERIAL_OVERRIDE_ENGINE_ASSET_EXTENSIONS: &[&str] = &["xml"];
