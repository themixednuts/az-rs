//! Runtime terrain product formats.
//!
//! This crate owns native Azoth terrain product bytes. It intentionally has no
//! dependency on authored document, schema, asset-builder, or editor crates.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

#[cfg(feature = "bevy")]
mod binding;
mod codec;
mod heightmap;
mod layer;
#[cfg(feature = "bevy")]
mod loader;
mod region;
mod types;
mod world;

pub use codec::{
    TerrainAssetCodecError, decode_terrain_heightmap_asset, decode_terrain_layer_set_asset,
    decode_terrain_region_asset, decode_terrain_world_asset, encode_terrain_heightmap_asset,
    encode_terrain_layer_set_asset, encode_terrain_region_asset, encode_terrain_world_asset,
    write_terrain_heightmap_asset, write_terrain_layer_set_asset, write_terrain_region_asset,
    write_terrain_world_asset,
};
pub use heightmap::{
    TerrainHeightmapAsset, TerrainHeightmapRegionView, TerrainWorldHeightSample,
    bilinear_world_height,
};
pub use layer::{SurfaceTag, TerrainLayer, TerrainLayerSetAsset};
#[cfg(feature = "bevy")]
pub use loader::{
    TerrainHeightmapAssetLoader, TerrainLayerSetAssetLoader, TerrainRegionAssetLoader,
    TerrainWorldAssetLoader,
};
pub use region::{
    TerrainConstantHeightSource, TerrainHeightGraphSource, TerrainHeightImageSource,
    TerrainHeightSource, TerrainHeightTilesSource, TerrainImageChannel, TerrainRegionAsset,
    TerrainSurfaceChannel, TerrainSurfaceGraphSource, TerrainSurfaceImageSource,
    TerrainSurfaceSource, TerrainSurfaceWeightsSource,
};
pub use types::{TerrainBounds, TerrainCoord, TerrainHeightRange, TerrainResolution};
pub use world::{TerrainRegionRef, TerrainWorldAsset};

pub const TERRAIN_WORLD_MAGIC: &[u8; 8] = b"AZTWDSC\0";
pub const TERRAIN_REGION_MAGIC: &[u8; 8] = b"AZTRDSC\0";
pub const TERRAIN_LAYER_SET_MAGIC: &[u8; 8] = b"AZTLDSC\0";
pub const TERRAIN_HEIGHTMAP_MAGIC: &[u8; 8] = b"AZTHDSC\0";

pub const TERRAIN_WORLD_PRODUCT_VERSION: u32 = 1;
pub const TERRAIN_REGION_PRODUCT_VERSION: u32 = 1;
pub const TERRAIN_LAYER_SET_PRODUCT_VERSION: u32 = 1;
pub const TERRAIN_HEIGHTMAP_PRODUCT_VERSION: u32 = 1;

pub const TERRAIN_WORLD_PRODUCT_EXTENSION: &str = "azterrain-world.bin";
pub const TERRAIN_REGION_PRODUCT_EXTENSION: &str = "azterrain-region.bin";
pub const TERRAIN_LAYER_SET_PRODUCT_EXTENSION: &str = "azterrain-layer-set.bin";
pub const TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION: &str = "azterrain-height.bin";

macro_rules! impl_terrain_asset_data {
    ($asset:ty, $name:literal, $type_id:literal, $stable_name:literal) => {
        impl az_core::AzTypeInfo for $asset {
            const NAME: &'static str = $name;
            const TYPE_ID: uuid::Uuid = uuid::uuid!($type_id);
        }

        impl az_core::AzRtti for $asset {}

        impl az_core::AssetData for $asset {
            const STABLE_NAME: &'static str = $stable_name;
        }
    };
}

impl_terrain_asset_data!(
    TerrainWorldAsset,
    "Azoth::TerrainWorldAsset",
    "8f11e3a0-c0d9-43bc-a21f-1f281bd64101",
    "azoth.terrain.world"
);
impl_terrain_asset_data!(
    TerrainRegionAsset,
    "Azoth::TerrainRegionAsset",
    "4f3ddc61-78d0-4f29-814b-f0f37aac23aa",
    "azoth.terrain.region"
);
impl_terrain_asset_data!(
    TerrainLayerSetAsset,
    "Azoth::TerrainLayerSetAsset",
    "9fb7f1d7-6edb-4f18-b6f4-0f9d9a57f8ea",
    "azoth.terrain.layer-set"
);
impl_terrain_asset_data!(
    TerrainHeightmapAsset,
    "Azoth::TerrainHeightmapAsset",
    "1f41b88d-633f-48d7-9b43-d24371f92123",
    "azoth.terrain.heightmap"
);

#[cfg(feature = "bevy")]
pub use binding::{
    TerrainHeightBinding, TerrainRegionBinding, TerrainRuntimeBindingError, TerrainRuntimePlugin,
    TerrainRuntimeSet, TerrainWorldBinding, TerrainWorldReference, TerrainWorldRegions,
};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The asset types this crate owns, for a composing host to register.
#[must_use]
pub const fn asset_types() -> [az_core::AssetTypeRegistration; 4] {
    [
        az_core::AssetTypeRegistration::for_asset::<TerrainWorldAsset>()
            .with_owner("az-terrain-runtime"),
        az_core::AssetTypeRegistration::for_asset::<TerrainRegionAsset>()
            .with_owner("az-terrain-runtime"),
        az_core::AssetTypeRegistration::for_asset::<TerrainLayerSetAsset>()
            .with_owner("az-terrain-runtime"),
        az_core::AssetTypeRegistration::for_asset::<TerrainHeightmapAsset>()
            .with_owner("az-terrain-runtime"),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::AssetTypeRegistration>()
        .register_many(asset_types());
}
