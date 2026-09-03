//! Camera Gem component data and Bevy integration.
//!
//! O3DE reference: `Gems/Camera/Code/Source/CameraComponent.h:22`.

use az_gem_contract::{Contribution, GemContext, contribution};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

type CameraSyncFilter = Or<(Changed<CameraComponent>, Without<Camera3d>)>;
type CameraSyncItem<'w> = (
    Entity,
    &'w CameraComponent,
    Option<&'w mut Camera>,
    Option<&'w mut Projection>,
    Option<&'w Transform>,
    Option<&'w Name>,
);

/// Lumberyard `Camera::CameraComponent` AZ component UUID.
pub const CAMERA_COMPONENT_TYPE_ID: Uuid = uuid!("E409F5C0-9919-4CA5-9488-1FE8A041768E");

/// Deprecated Lumberyard `Camera::CameraComponent` AZ component UUID.
pub const DEPRECATED_CAMERA_COMPONENT_TYPE_ID: Uuid = uuid!("A0C21E18-F759-4E72-AF26-7A36FC59E477");

pub const DEFAULT_FOV_DEGREES: f32 = 75.0;
pub const DEFAULT_NEAR_CLIP_DISTANCE: f32 = 0.2;
pub const DEFAULT_FAR_CLIP_DISTANCE: f32 = 1024.0;
pub const DEFAULT_FRUSTUM_DIMENSION: f32 = 256.0;
pub const DEFAULT_BLEND_FACTOR: f32 = 1.0;

/// Runtime camera component.
///
/// O3DE reference: `Gems/Camera/Code/Source/CameraComponent.cpp:105`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
pub struct CameraComponent {
    pub fov_degrees: f32,
    pub near_clip_distance: f32,
    pub far_clip_distance: f32,
    pub specify_frustum_dimensions: bool,
    pub frustum_width: f32,
    pub frustum_height: f32,
    pub enable_on_activate: bool,
    pub blend_factor: f32,
    pub outgoing_blend_factor: f32,
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraComponent {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fov_degrees: DEFAULT_FOV_DEGREES,
            near_clip_distance: DEFAULT_NEAR_CLIP_DISTANCE,
            far_clip_distance: DEFAULT_FAR_CLIP_DISTANCE,
            specify_frustum_dimensions: false,
            frustum_width: DEFAULT_FRUSTUM_DIMENSION,
            frustum_height: DEFAULT_FRUSTUM_DIMENSION,
            enable_on_activate: true,
            blend_factor: DEFAULT_BLEND_FACTOR,
            outgoing_blend_factor: DEFAULT_BLEND_FACTOR,
        }
    }

    #[must_use]
    pub const fn fov_radians(&self) -> f32 {
        self.fov_degrees.to_radians()
    }

    #[must_use]
    pub fn aspect_ratio(&self, default: f32) -> f32 {
        if self.specify_frustum_dimensions && self.frustum_height > 0.0 {
            self.frustum_width.max(0.001) / self.frustum_height
        } else {
            default
        }
    }

    #[must_use]
    pub fn perspective_projection(&self, default_aspect_ratio: f32) -> PerspectiveProjection {
        let near = self.near_clip_distance.max(0.001);
        PerspectiveProjection {
            fov: self.fov_radians(),
            aspect_ratio: self.aspect_ratio(default_aspect_ratio),
            near,
            far: self.far_clip_distance.max(near),
            near_clip_plane: Vec4::new(0.0, 0.0, -1.0, -near),
        }
    }
}

fn sync_camera_components(
    mut commands: Commands,
    mut query: Query<CameraSyncItem<'_>, CameraSyncFilter>,
) {
    for (entity, component, camera, projection, transform, name) in &mut query {
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(Camera3d::default());

        if let Some(mut camera) = camera {
            camera.is_active = component.enable_on_activate;
        } else {
            entity_commands.insert(Camera {
                is_active: component.enable_on_activate,
                ..Default::default()
            });
        }

        let native_projection = Projection::Perspective(component.perspective_projection(1.0));
        if let Some(mut projection) = projection {
            *projection = native_projection;
        } else {
            entity_commands.insert(native_projection);
        }

        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new("CameraComponent"));
        }
    }
}

/// Register Camera Gem components and runtime sync.
pub struct CameraGemPlugin;

impl Plugin for CameraGemPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<CameraComponent>()
            .add_systems(Update, sync_camera_components);
    }
}

/// The gem's `package` contribution.
///
/// Camera's content is a Bevy plugin, and a plugin is not a registry entry:
/// this gem owns no AZ type, asset type, component lowering, or prefab type,
/// so the registration surface is genuinely empty. Sealing is privacy — the
/// generated `package_contribution` is the only way in.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each assertion compares a default field against the very constant that
    // defines it, so the values are exact by construction. An epsilon would
    // weaken the property under test: that the default IS the constant.
    #[allow(clippy::float_cmp)]
    #[test]
    fn camera_component_defaults_match_source_defaults() {
        let component = CameraComponent::default();

        assert_eq!(component.fov_degrees, DEFAULT_FOV_DEGREES);
        assert_eq!(component.near_clip_distance, DEFAULT_NEAR_CLIP_DISTANCE);
        assert_eq!(component.far_clip_distance, DEFAULT_FAR_CLIP_DISTANCE);
        assert!(!component.specify_frustum_dimensions);
        assert_eq!(component.frustum_width, DEFAULT_FRUSTUM_DIMENSION);
        assert_eq!(component.frustum_height, DEFAULT_FRUSTUM_DIMENSION);
        assert!(component.enable_on_activate);
        assert_eq!(component.blend_factor, DEFAULT_BLEND_FACTOR);
        assert_eq!(component.outgoing_blend_factor, DEFAULT_BLEND_FACTOR);
    }

    // These assert bit-exact pass-through of the configured values into the
    // Bevy projection. Comparing against an epsilon would stop the test
    // catching a lossy conversion, which is the thing it exists to catch.
    #[allow(clippy::float_cmp)]
    #[test]
    fn camera_component_builds_bevy_perspective_projection() {
        let component = CameraComponent {
            fov_degrees: 90.0,
            near_clip_distance: 0.5,
            far_clip_distance: 2048.0,
            specify_frustum_dimensions: true,
            frustum_width: 1920.0,
            frustum_height: 1080.0,
            enable_on_activate: false,
            blend_factor: 0.25,
            outgoing_blend_factor: 0.75,
        };

        let projection = component.perspective_projection(1.0);

        assert_eq!(projection.fov, 90.0_f32.to_radians());
        assert_eq!(projection.aspect_ratio, 1920.0 / 1080.0);
        assert_eq!(projection.near, 0.5);
        assert_eq!(projection.far, 2048.0);
        assert_eq!(projection.near_clip_plane, Vec4::new(0.0, 0.0, -1.0, -0.5));
    }

    #[test]
    fn plugin_syncs_camera_component_to_bevy_camera() {
        let mut app = App::new();
        app.add_plugins(CameraGemPlugin);

        let source = CameraComponent {
            enable_on_activate: false,
            ..Default::default()
        };
        let entity = app.world_mut().spawn(source).id();

        app.update();

        let world = app.world();
        assert!(world.entity(entity).contains::<Camera3d>());
        assert!(!world.get::<Camera>(entity).unwrap().is_active);
        assert!(matches!(
            world.get::<Projection>(entity).unwrap(),
            Projection::Perspective(_)
        ));
        assert!(world.entity(entity).contains::<Transform>());
        assert_eq!(
            world.get::<Name>(entity).unwrap().as_str(),
            "CameraComponent"
        );
    }
}
