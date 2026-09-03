use az_gem_lmbr_central::{
    is_static_model_source_asset_path, resolve_scene_asset_path_with_variant,
    static_model_engine_asset_path,
};
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptorListComponent;
use crate::instance::InstanceData;

pub(super) fn instance_scene_asset_path(
    entity: Entity,
    instance: &InstanceData,
    descriptor_lists: &Query<Ref<VegetationDescriptorListComponent>>,
) -> Option<String> {
    let descriptor_index = instance.descriptor_index?;
    let descriptor_entity = instance.entity.unwrap_or(entity);
    let descriptor_list = descriptor_lists.get(descriptor_entity).ok()?;
    let descriptor = descriptor_list.configuration.descriptor(descriptor_index)?;
    let source_path = descriptor.scene_asset_path()?;
    // Same source→product step `lmbr_central::sync_scene_component` takes: a
    // descriptor names a `.cgf`, and what a host binds is the `.azmesh` the
    // pipeline built from it. Without this the caller's
    // `is_static_model_engine_asset_path` check never matches and a static mesh
    // is bound as a dynamic world root instead.
    resolve_scene_asset_path_with_variant(source_path, descriptor.scene_asset_variant()).map(
        |path| {
            if is_static_model_source_asset_path(&path) {
                static_model_engine_asset_path(&path)
            } else {
                path
            }
        },
    )
}

pub(super) fn instance_descriptor_list_changed(
    entity: Entity,
    instance: &InstanceData,
    descriptor_lists: &Query<Ref<VegetationDescriptorListComponent>>,
) -> bool {
    let descriptor_entity = instance.entity.unwrap_or(entity);
    descriptor_lists
        .get(descriptor_entity)
        .is_ok_and(|descriptor_list| descriptor_list.is_changed())
}
