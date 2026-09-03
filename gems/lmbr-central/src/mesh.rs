//! Mesh components for `LmbrCentral`.
//!
//! Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/MeshComponent.h`.

mod ids;
mod instanced;
mod opacity;
mod skinned;
mod static_mesh;
mod static_model;

use bevy::prelude::*;

pub use az_mesh_builder::MeshAsset as NativeMeshAsset;

pub use static_model::{
    cleanup_removed_mesh_collider_geometry, cleanup_removed_static_model_physics_bindings,
    cleanup_removed_static_model_physics_geometry_requests, despawn_static_model_children,
    resolve_static_character_attachments, spawn_static_model_instances,
    sync_static_model_physics_geometry,
};

pub use ids::*;
pub use instanced::{
    InstancedMeshChildren, InstancedMeshComponent, InstancedMeshComponentRenderNode,
    InstancedMeshInstance,
};
pub use opacity::MeshOpacityRequest;
pub use skinned::{SkinnedMeshComponent, SkinnedMeshComponentRenderNode, SkinnedRenderOptions};
pub use static_mesh::{MeshComponent, MeshComponentRenderNode, MeshRenderOptions};
pub use static_model::{
    StaticCharacterAttachmentError, StaticModelBinding, StaticModelChildren,
    StaticModelPhysicsBinding, StaticModelPhysicsError, StaticModelPhysicsGeometryRequest,
    StaticModelPrimitiveVisual, is_static_model_engine_asset_path,
    is_static_model_source_asset_path, static_model_engine_asset_path,
};

pub fn register_mesh_components(app: &mut App) {
    app.register_type::<MeshRenderOptions>()
        .register_type::<MeshComponentRenderNode>()
        .register_type::<MeshComponent>()
        .register_type::<SkinnedRenderOptions>()
        .register_type::<SkinnedMeshComponentRenderNode>()
        .register_type::<SkinnedMeshComponent>()
        .register_type::<InstancedMeshComponentRenderNode>()
        .register_type::<InstancedMeshComponent>()
        .register_type::<Vec<Transform>>()
        .add_observer(opacity::begin_mesh_opacity_transition)
        .add_systems(
            Update,
            (
                spawn_static_model_instances,
                opacity::update_mesh_opacity_transitions,
            )
                .chain(),
        );
}

/// Register the renderer-independent half of static-model handling.
///
/// Mesh components own their product binding regardless of whether a client
/// later renders that product. This keeps `IStatObj::Physicalize`-style mesh
/// collision available to authoritative servers without installing the full
/// `LmbrCentral` rendering/audio/scene plugin.
pub fn register_static_model_physics_components(app: &mut App) {
    app.register_type::<MeshRenderOptions>()
        .register_type::<MeshComponentRenderNode>()
        .register_type::<MeshComponent>()
        .register_type::<crate::SceneAssetBinding>()
        .add_systems(
            Update,
            crate::sync_scene_components::<MeshComponent>
                .before(crate::physics::LmbrCentralAuthoringSet::Geometry),
        )
        .add_systems(
            Update,
            (
                resolve_static_character_attachments,
                sync_static_model_physics_geometry,
                cleanup_removed_static_model_physics_bindings,
                cleanup_removed_mesh_collider_geometry,
                cleanup_removed_static_model_physics_geometry_requests,
            )
                .chain()
                .in_set(crate::physics::LmbrCentralAuthoringSet::Geometry),
        );
}
