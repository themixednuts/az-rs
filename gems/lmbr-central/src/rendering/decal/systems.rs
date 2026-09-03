use bevy::mesh::Mesh3d;
use bevy::prelude::*;

use crate::material::{MaterialAssetBinding, sync_material_asset_binding};

use super::component::DecalComponent;

#[allow(clippy::type_complexity)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub(super) fn sync_decal_components(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    query: Query<
        (
            Entity,
            Ref<DecalComponent>,
            Option<&Mesh3d>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&MaterialAssetBinding>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(Changed<DecalComponent>, Without<Visibility>)>,
    >,
) {
    for (entity, component, mesh, material, binding, transform, name) in &query {
        let config = &component.configuration;
        let mut entity_commands = commands.entity(entity);

        entity_commands.insert(if config.is_rendered() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        let material_changed = sync_material_asset_binding(
            &mut entity_commands,
            asset_server.as_deref(),
            config.material_asset_path.as_deref().unwrap_or_default(),
            binding,
        );

        if let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
            if mesh.is_none() || component.is_changed() {
                entity_commands.insert(Mesh3d(meshes.add(config.preview_mesh())));
            }
            if material.is_none() || component.is_changed() || material_changed {
                entity_commands.insert(MeshMaterial3d(materials.add(config.preview_material())));
            }
        }

        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new("DecalComponent"));
        }
    }
}
