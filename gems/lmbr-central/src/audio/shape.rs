//! Audio shape and spline marker components.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// `AudioShapeComponent` AZ type UUID.
pub const AUDIO_SHAPE_COMPONENT_TYPE_ID: Uuid = uuid!("58AABF8E-6954-4634-ACBD-05FE011478E1");

/// `AudioSplineComponent` AZ type UUID.
pub const AUDIO_SPLINE_COMPONENT_TYPE_ID: Uuid = uuid!("8390440C-3621-437A-9E74-4588C39F847E");

/// Runtime audio shape component.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioShapeComponent", version = 1)]
pub struct AudioShapeComponent {
    pub exterior_follow_mode: i32,
    pub interior_follow_mode: i32,
    pub interior_follow_offset: f32,
    pub send_enter_exit_messages: bool,
    pub follow_camera_subject: bool,
}

/// Runtime audio spline component.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioSplineComponent", version = 1)]
pub struct AudioSplineComponent;

pub(super) fn register_audio_shape_components(app: &mut App) {
    app.register_type::<AudioShapeComponent>()
        .register_type::<AudioSplineComponent>();
}
