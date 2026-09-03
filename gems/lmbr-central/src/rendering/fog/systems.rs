use bevy::light::FogVolume as BevyFogVolume;
use bevy::prelude::*;

use super::component::FogVolumeComponent;

#[allow(clippy::type_complexity)]
pub(super) fn sync_fog_volume_components(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            Ref<FogVolumeComponent>,
            Option<&BevyFogVolume>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(Changed<FogVolumeComponent>, Without<BevyFogVolume>)>,
    >,
) {
    for (entity, component, fog_volume, transform, name) in &query {
        let config = &component.configuration;
        let mut entity_commands = commands.entity(entity);

        if config.is_rendered() {
            if fog_volume.is_none() || component.is_changed() {
                entity_commands.insert(config.bevy_fog_volume());
            }
            entity_commands.insert(Visibility::Visible);
        } else {
            entity_commands.remove::<BevyFogVolume>();
            entity_commands.insert(Visibility::Hidden);
        }

        if transform.is_none() {
            entity_commands.insert(Transform::from_scale(config.normalized_size()));
        }
        if name.is_none() {
            entity_commands.insert(Name::new("FogVolumeComponent"));
        }
    }
}
