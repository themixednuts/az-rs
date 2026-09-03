//! Mesh product identities shared by offline builders and runtime systems.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

use az_core::{AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo};
use uuid::{Uuid, uuid};

pub const MESH_ASSET_TYPE_NAME: &str = "azoth.mesh.model";
pub const SKINNED_MESH_ASSET_TYPE_NAME: &str = "azoth.mesh.skinned-model";
pub const SKELETON_ASSET_TYPE_NAME: &str = "azoth.mesh.skeleton";

pub struct MeshAssetData;

impl AzTypeInfo for MeshAssetData {
    const NAME: &'static str = "Azoth::MeshAsset";
    const TYPE_ID: Uuid = uuid!("84bc2032-9c3a-4a56-8458-011b9bf1b243");
}

impl AzRtti for MeshAssetData {}

impl AssetData for MeshAssetData {
    const STABLE_NAME: &'static str = MESH_ASSET_TYPE_NAME;
}

pub const MESH_ASSET_TYPE: AssetType = MeshAssetData::ASSET_TYPE;

pub struct SkinnedMeshAssetData;

impl AzTypeInfo for SkinnedMeshAssetData {
    const NAME: &'static str = "Azoth::SkinnedMeshAsset";
    const TYPE_ID: Uuid = uuid!("89a00f7d-a17e-427f-03cf-471912023b08");
}

impl AzRtti for SkinnedMeshAssetData {}

impl AssetData for SkinnedMeshAssetData {
    const STABLE_NAME: &'static str = SKINNED_MESH_ASSET_TYPE_NAME;
}

pub const SKINNED_MESH_ASSET_TYPE: AssetType = SkinnedMeshAssetData::ASSET_TYPE;

pub struct SkeletonAssetData;

impl AzTypeInfo for SkeletonAssetData {
    const NAME: &'static str = "Azoth::SkeletonAsset";
    const TYPE_ID: Uuid = uuid!("104c78fb-54bc-4ec3-886c-79eafd0b6a86");
}

impl AzRtti for SkeletonAssetData {}

impl AssetData for SkeletonAssetData {
    const STABLE_NAME: &'static str = SKELETON_ASSET_TYPE_NAME;
}

pub const SKELETON_ASSET_TYPE: AssetType = SkeletonAssetData::ASSET_TYPE;

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 3] {
    [
        AssetTypeRegistration::for_asset::<MeshAssetData>().with_owner("az-mesh"),
        AssetTypeRegistration::for_asset::<SkinnedMeshAssetData>().with_owner("az-mesh"),
        AssetTypeRegistration::for_asset::<SkeletonAssetData>().with_owner("az-mesh"),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
}
