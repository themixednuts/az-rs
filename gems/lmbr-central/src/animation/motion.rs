//! Reflected `LmbrCentral` character-motion component data.

use az_animation::motion::{
    MotionExtractionSettings, MotionParameterSmoothingState, MotionParameters,
    MotionSmoothingSettings, RootMotionDelta, drive_motion_parameters, extract_motion_parameters,
    smooth_motion_parameters,
};
use az_core::component::Component as AzComponent;
use az_derive::{AzComponent, AzTypeInfo};
use az_gem_animation::{CryAnimationPlayer, CryAnimationSet};
use az_physics::{PhysicsBodyHandle, PhysicsWorld};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const MOTION_PARAMETER_SMOOTHING_SETTINGS_TYPE_ID: Uuid =
    uuid!("7DB44746-EA1D-4A53-9270-7600A5AA8027");
pub const MOTION_PARAMETER_SMOOTHING_COMPONENT_TYPE_ID: Uuid =
    uuid!("C927CF87-CD02-4201-BFAD-CB5956586467");
pub const CHARACTER_ANIMATION_MANAGER_COMPONENT_TYPE_ID: Uuid =
    uuid!("ABD0848C-0CFC-43D3-AEFB-7EEED64AF164");

/// Current serialized version-1 motion smoothing settings.
///
/// The two fields absent from stock Lumberyard and all defaults were validated
/// against the shipping constructor. Convergence values are in seconds.
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "MotionParameterSmoothingSettings",
    MOTION_PARAMETER_SMOOTHING_SETTINGS_TYPE_ID
)]
pub struct MotionParameterSmoothingSettings {
    #[serde(
        rename = "MovementSpeedEpsilon",
        default = "default_movement_speed_epsilon"
    )]
    pub movement_speed_epsilon: f32,
    #[serde(
        rename = "GroundAngleConvergeTime",
        default = "default_standard_converge_time"
    )]
    pub ground_angle_converge_time: f32,
    #[serde(
        rename = "TravelAngleConvergeTime",
        default = "default_standard_converge_time"
    )]
    pub travel_angle_converge_time: f32,
    #[serde(
        rename = "TravelDistanceConvergeTime",
        default = "default_standard_converge_time"
    )]
    pub travel_distance_converge_time: f32,
    #[serde(
        rename = "TravelSpeedConvergeTime",
        default = "default_standard_converge_time"
    )]
    pub travel_speed_converge_time: f32,
    #[serde(
        rename = "TurnAngleConvergeTime",
        default = "default_turn_converge_time"
    )]
    pub turn_angle_converge_time: f32,
    #[serde(
        rename = "TurnSpeedConvergeTime",
        default = "default_turn_converge_time"
    )]
    pub turn_speed_converge_time: f32,
    #[serde(rename = "TurnSpeedScale", default = "default_turn_speed_scale")]
    pub turn_speed_scale: f32,
}

impl Default for MotionParameterSmoothingSettings {
    fn default() -> Self {
        Self {
            movement_speed_epsilon: default_movement_speed_epsilon(),
            ground_angle_converge_time: default_standard_converge_time(),
            travel_angle_converge_time: default_standard_converge_time(),
            travel_distance_converge_time: default_standard_converge_time(),
            travel_speed_converge_time: default_standard_converge_time(),
            turn_angle_converge_time: default_turn_converge_time(),
            turn_speed_converge_time: default_turn_converge_time(),
            turn_speed_scale: default_turn_speed_scale(),
        }
    }
}

impl MotionExtractionSettings for MotionParameterSmoothingSettings {
    fn movement_speed_epsilon(&self) -> f32 {
        self.movement_speed_epsilon
    }
}

impl MotionSmoothingSettings for MotionParameterSmoothingSettings {
    fn ground_angle_converge_time(&self) -> f32 {
        self.ground_angle_converge_time
    }
    fn travel_angle_converge_time(&self) -> f32 {
        self.travel_angle_converge_time
    }
    fn travel_distance_converge_time(&self) -> f32 {
        self.travel_distance_converge_time
    }
    fn travel_speed_converge_time(&self) -> f32 {
        self.travel_speed_converge_time
    }
    fn turn_angle_converge_time(&self) -> f32 {
        self.turn_angle_converge_time
    }
    fn turn_speed_converge_time(&self) -> f32 {
        self.turn_speed_converge_time
    }
    fn turn_speed_scale(&self) -> f32 {
        self.turn_speed_scale
    }
}

const fn default_movement_speed_epsilon() -> f32 {
    0.02
}
const fn default_standard_converge_time() -> f32 {
    0.1
}
const fn default_turn_converge_time() -> f32 {
    0.75
}
const fn default_turn_speed_scale() -> f32 {
    1.0
}

#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(
    name = "MotionParameterSmoothingComponent",
    MOTION_PARAMETER_SMOOTHING_COMPONENT_TYPE_ID
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(
    tag = "azoth.lmbr_central.MotionParameterSmoothingComponent",
    version = 1
)]
pub struct MotionParameterSmoothingComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Settings", default)]
    pub settings: MotionParameterSmoothingSettings,
}

#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(
    name = "CharacterAnimationManagerComponent",
    CHARACTER_ANIMATION_MANAGER_COMPONENT_TYPE_ID
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(
    tag = "azoth.lmbr_central.CharacterAnimationManagerComponent",
    version = 1
)]
pub struct CharacterAnimationManagerComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
}

#[derive(Component, Debug, Clone)]
struct CharacterMotionRuntime {
    previous_world_transform: Transform,
    smoothed: MotionParameters,
    smoothing: MotionParameterSmoothingState,
}

impl CharacterMotionRuntime {
    fn new(world_transform: Transform) -> Self {
        Self {
            previous_world_transform: world_transform,
            smoothed: MotionParameters::default(),
            smoothing: MotionParameterSmoothingState::default(),
        }
    }
}

pub fn register_motion_runtime(app: &mut App) {
    app.add_systems(
        Update,
        (
            initialize_character_motion_runtime,
            update_character_motion_parameters,
        )
            .chain()
            .before(CryAnimationSet::Advance),
    );
}

fn initialize_character_motion_runtime(
    mut commands: Commands,
    characters: Query<(Entity, &Transform), Added<CryAnimationPlayer>>,
) {
    for (entity, &world_transform) in &characters {
        commands
            .entity(entity)
            .insert(CharacterMotionRuntime::new(world_transform));
    }
}

/// Everything one character contributes to a frame of motion extraction.
type CharacterMotionData = (
    &'static Transform,
    &'static mut CryAnimationPlayer,
    &'static mut CharacterMotionRuntime,
    Option<&'static MotionParameterSmoothingComponent>,
    Option<&'static PhysicsBodyHandle>,
);

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn update_character_motion_parameters(
    time: Res<Time>,
    physics_world: Option<Res<PhysicsWorld>>,
    mut characters: Query<CharacterMotionData>,
) {
    let delta_time = time.delta_secs();
    for (world_transform, mut animation, mut runtime, smoothing, body) in &mut characters {
        let previous = runtime.previous_world_transform;
        let inverse_previous_rotation = previous.rotation.inverse();
        let frame_motion = RootMotionDelta {
            rotation: Quat::from_vec4(
                Vec4::from(inverse_previous_rotation * world_transform.rotation)
                    .try_normalize()
                    .unwrap_or(Vec4::W),
            ),
            translation: inverse_previous_rotation
                * (world_transform.translation - previous.translation),
        };
        let ground_slope = physics_world
            .as_deref()
            .zip(body)
            .and_then(|(world, &body)| world.living_status(body).ok())
            .map_or(Vec3::Z, |status| status.ground_slope);
        let defaults = MotionParameterSmoothingSettings::default();
        let settings = smoothing.map_or(&defaults, |component| &component.settings);
        let target = extract_motion_parameters(
            delta_time,
            world_transform.rotation,
            frame_motion,
            ground_slope,
            settings,
        );
        let runtime = runtime.as_mut();
        smooth_motion_parameters(
            &mut runtime.smoothed,
            target,
            &mut runtime.smoothing,
            settings,
            delta_time,
        );
        drive_motion_parameters(&mut *animation, runtime.smoothed, settings, delta_time);
        runtime.previous_world_transform = *world_transform;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn settings_use_current_runtime_defaults() {
        let settings = MotionParameterSmoothingSettings::default();

        assert_eq!(settings.movement_speed_epsilon, 0.02);
        assert_eq!(settings.ground_angle_converge_time, 0.1);
        assert_eq!(settings.turn_angle_converge_time, 0.75);
        assert_eq!(settings.turn_speed_converge_time, 0.75);
        assert_eq!(settings.turn_speed_scale, 1.0);
    }
}
