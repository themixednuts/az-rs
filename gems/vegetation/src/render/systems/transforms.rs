use bevy::prelude::*;

use crate::instance::InstanceData;

#[allow(clippy::type_complexity)]
pub(in crate::render) fn sync_instance_transforms(
    mut commands: Commands,
    query: Query<
        (Entity, &InstanceData, Option<&Name>),
        Or<(Changed<InstanceData>, Without<Transform>)>,
    >,
) {
    for (entity, instance, name) in &query {
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(instance.transform());
        if name.is_none() {
            let name = if instance.instance_id.is_valid() {
                format!("Vegetation Instance {}", instance.instance_id.0)
            } else {
                "Vegetation Instance".to_string()
            };
            entity_commands.insert(Name::new(name));
        }
    }
}
