//! Authored render components and Bevy runtime resolution.

mod camera;
pub mod generated;
mod lights;
mod material_assignment;
mod mesh;
#[cfg(feature = "bevy")]
mod runtime;

use bevy_reflect::TypeRegistry;

pub use camera::{
    CAMERA_COMPONENT_TYPE_ID, CAMERA_PROJECTION_SCHEMA_NAME, CAMERA_SCHEMA_NAME, Camera,
    CameraProjection, ORTHOGRAPHIC_PROJECTION_SCHEMA_NAME, OrthographicCameraProjection,
    PERSPECTIVE_PROJECTION_SCHEMA_NAME, PerspectiveCameraProjection,
};
pub use lights::{
    DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID, DIRECTIONAL_LIGHT_SCHEMA_NAME, DirectionalLight,
    POINT_LIGHT_COMPONENT_TYPE_ID, POINT_LIGHT_SCHEMA_NAME, PointLight,
    SPOT_LIGHT_COMPONENT_TYPE_ID, SPOT_LIGHT_SCHEMA_NAME, SpotLight,
};
pub use material_assignment::{
    MATERIAL_ASSIGNMENT_COMPONENT_TYPE_ID, MATERIAL_ASSIGNMENT_SCHEMA_NAME,
    MATERIAL_SLOT_OVERRIDE_SCHEMA_NAME, MaterialAssignment, MaterialSlotOverride,
};
pub use mesh::{MESH_COMPONENT_TYPE_ID, MESH_SCHEMA_NAME, Mesh};
#[cfg(feature = "bevy")]
pub use runtime::{AzothRenderPlugin, MeshRenderState, RenderMeshChild, ResolvedMaterialSlot};

/// Registers the first ADR-0022 render vertical slice in a caller-owned Bevy
/// registry. Every process role uses this same registration function.
pub fn register_render_prefab_types(registry: &mut TypeRegistry) {
    mesh::register_prefab_type(registry);
    material_assignment::register_prefab_types(registry);
    camera::register_prefab_types(registry);
    lights::register_prefab_types(registry);
}

#[cfg(test)]
mod prefab_tests;

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;
    use az_core::{ApplicabilityTypeData, AzTypeInfo};

    #[test]
    fn mesh_uses_vendored_serialize_component_identity() {
        assert_eq!(Mesh::TYPE_ID, generated::MESH_COMPONENT_TYPE_ID);
        assert_eq!(
            generated::MaterialHandleIdentity::TYPE_ID,
            generated::MATERIAL_HANDLE_TYPE_ID
        );
    }

    #[test]
    fn camera_and_light_codegen_identities_are_locked() {
        assert_eq!(Camera::TYPE_ID, generated::CAMERA_COMPONENT_TYPE_ID);
        assert_eq!(
            generated::CameraComponentIdentity::TYPE_ID,
            uuid::uuid!("a0c21e18-f759-4e72-af26-7a36fc59e477")
        );
        assert_eq!(
            generated::LightComponentIdentity::TYPE_ID,
            uuid::uuid!("6b9ab512-ca8a-4d2b-b570-df128ea7ce6a")
        );
        assert_eq!(
            generated::LightConfigurationIdentity::TYPE_ID,
            uuid::uuid!("f4cc7bb4-c541-480c-88fc-c5a8f37cc67f")
        );
        assert_eq!(
            DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID,
            uuid::uuid!("de63771b-8df3-5e22-9d07-3b4133dc8b0d")
        );
        assert_eq!(
            POINT_LIGHT_COMPONENT_TYPE_ID,
            uuid::uuid!("ce3c0330-3acb-5a0c-ba08-19bd7ed8e73c")
        );
        assert_eq!(
            SPOT_LIGHT_COMPONENT_TYPE_ID,
            uuid::uuid!("2d7d6141-3e65-5989-8aea-a85a2bdbe342")
        );
    }

    #[test]
    fn camera_and_light_type_data_leave_concrete_requirements_to_bevy() {
        let mut registry = TypeRegistry::default();
        register_render_prefab_types(&mut registry);
        let camera = registry
            .get(TypeId::of::<Camera>())
            .and_then(|registration| registration.data::<ApplicabilityTypeData>())
            .expect("Camera applicability TypeData");
        assert_eq!(camera.provides, &["azoth.camera"]);
        assert!(camera.requires.is_empty());
        for type_id in [
            TypeId::of::<DirectionalLight>(),
            TypeId::of::<PointLight>(),
            TypeId::of::<SpotLight>(),
        ] {
            let light = registry
                .get(type_id)
                .and_then(|registration| registration.data::<ApplicabilityTypeData>())
                .expect("Light applicability TypeData");
            assert_eq!(light.provides, &["azoth.light"]);
            assert!(light.requires.is_empty());
        }
    }

    #[test]
    fn material_assignment_requires_mesh_slot_capabilities() {
        let mut registry = TypeRegistry::default();
        register_render_prefab_types(&mut registry);
        let registration = registry
            .get(TypeId::of::<MaterialAssignment>())
            .and_then(|registration| registration.data::<ApplicabilityTypeData>())
            .expect("MaterialAssignment applicability TypeData");
        assert_eq!(
            registration.requires,
            &["azoth.renderable", "azoth.material-slots"]
        );
    }
}
