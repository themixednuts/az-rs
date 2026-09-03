use bevy::mesh::Mesh3d;
use bevy::prelude::*;

use super::component::LensFlareComponent;

#[allow(clippy::type_complexity)]
pub(super) fn sync_lens_flare_components(
    mut commands: Commands,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    query: Query<
        (
            Entity,
            Ref<LensFlareComponent>,
            Option<&Mesh3d>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(Changed<LensFlareComponent>, Without<Visibility>)>,
    >,
) {
    for (entity, component, mesh, material, transform, name) in &query {
        let config = &component.configuration;
        let mut entity_commands = commands.entity(entity);

        entity_commands.insert(if config.is_rendered() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });

        if let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
            if mesh.is_none() || component.is_changed() {
                entity_commands.insert(Mesh3d(meshes.add(config.preview_mesh())));
            }
            if material.is_none() || component.is_changed() {
                entity_commands.insert(MeshMaterial3d(materials.add(config.preview_material())));
            }
        }

        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new("LensFlareComponent"));
        }
    }
}
