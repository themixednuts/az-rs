use bevy::light::{EnvironmentMapLight as BevyEnvironmentMapLight, LightProbe as BevyLightProbe};
use bevy::prelude::*;

use crate::non_empty_path;

use super::component::LightComponent;
use super::config::LightType;

#[allow(clippy::type_complexity)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub(super) fn sync_light_components(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    query: Query<
        (
            Entity,
            &LightComponent,
            Option<&BevyEnvironmentMapLight>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(Changed<LightComponent>, Without<Visibility>)>,
    >,
) {
    for (entity, component, environment_map, transform, name) in &query {
        let config = &component.configuration;
        let mut entity_commands = commands.entity(entity);

        if config.is_rendered() {
            entity_commands.insert(Visibility::Visible);
            match config.light_type {
                LightType::Point => {
                    entity_commands.insert(config.point_light());
                    entity_commands.remove::<SpotLight>();
                    entity_commands.remove::<BevyLightProbe>();
                    entity_commands.remove::<BevyEnvironmentMapLight>();
                }
                LightType::Area => {
                    entity_commands.insert(config.area_light_as_point_light());
                    entity_commands.remove::<SpotLight>();
                    entity_commands.remove::<BevyLightProbe>();
                    entity_commands.remove::<BevyEnvironmentMapLight>();
                }
                LightType::Projector => {
                    entity_commands.insert(config.spot_light());
                    entity_commands.remove::<PointLight>();
                    entity_commands.remove::<BevyLightProbe>();
                    entity_commands.remove::<BevyEnvironmentMapLight>();
                }
                LightType::Probe => {
                    entity_commands.remove::<PointLight>();
                    entity_commands.remove::<SpotLight>();
                    entity_commands.insert(BevyLightProbe::new());

                    if let (Some(asset_server), Some(path)) = (
                        asset_server.as_deref(),
                        non_empty_path(config.probe_cubemap_asset_path.as_deref()),
                    ) {
                        let cubemap: Handle<Image> = asset_server.load(path.to_owned());
                        entity_commands
                            .insert(config.environment_map_light(cubemap.clone(), cubemap));
                    } else if environment_map.is_some() {
                        entity_commands.remove::<BevyEnvironmentMapLight>();
                    }
                }
            }
        } else {
            entity_commands.insert(Visibility::Hidden);
            entity_commands.remove::<PointLight>();
            entity_commands.remove::<SpotLight>();
            entity_commands.remove::<BevyLightProbe>();
            entity_commands.remove::<BevyEnvironmentMapLight>();
        }

        if transform.is_none() {
            if config.light_type == LightType::Probe {
                entity_commands.insert(Transform::from_scale(config.probe_transform_scale()));
            } else {
                entity_commands.insert(Transform::default());
            }
        }
        if name.is_none() {
            entity_commands.insert(Name::new("LightComponent"));
        }
    }
}
