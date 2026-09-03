//! Checked-in serialization identities used by engine render components.
//!
//! The legacy identities are validated against O3DE and Lumberyard public source:
//! `dev/Gems/LmbrCentral/Code/Source/Rendering/MeshComponent.h`,
//! `dev/Gems/LmbrCentral/Code/include/LmbrCentral/Rendering/MaterialHandle.h`,
//! O3DE's `Gems/Camera/Code/Source/CameraComponent.cpp`, and
//! `dev/Gems/LmbrCentral/Code/Source/Rendering/LightComponent.h`.

use uuid::{Uuid, uuid};

/// Concrete serialized `MeshComponent` (version 1), not the converter-only
/// legacy shell with UUID `9697D425-...`.
pub const MESH_COMPONENT_TYPE_ID: Uuid = uuid!("2f4bad46-c857-4dcb-a454-c412de67852a");

/// Serialized `MaterialHandle` value identity. The dump contains no material
/// assignment component, so assignment remains an Azoth-authored component.
pub const MATERIAL_HANDLE_TYPE_ID: Uuid = uuid!("bf659dc6-acdd-4062-a52e-4ec053286f4f");

/// Converter identity emitted by serialize-codegen for `CameraComponent`.
pub const CAMERA_COMPONENT_TYPE_ID: Uuid = uuid!("a0c21e18-f759-4e72-af26-7a36fc59e477");

/// Concrete legacy `LightComponent` identity emitted by serialize-codegen.
pub const LIGHT_COMPONENT_TYPE_ID: Uuid = uuid!("6b9ab512-ca8a-4d2b-b570-df128ea7ce6a");

/// Serialized `LightConfiguration` value identity emitted by serialize-codegen.
pub const LIGHT_CONFIGURATION_TYPE_ID: Uuid = uuid!("f4cc7bb4-c541-480c-88fc-c5a8f37cc67f");

/// The source component is a tagged mega-configuration.
///
/// Azoth deliberately exposes separate native light schemas, deriving stable
/// subtype identities with `UUIDv5` from [`LIGHT_COMPONENT_TYPE_ID`] and each
/// schema name.
pub const DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID: Uuid = uuid!("de63771b-8df3-5e22-9d07-3b4133dc8b0d");
pub const POINT_LIGHT_COMPONENT_TYPE_ID: Uuid = uuid!("ce3c0330-3acb-5a0c-ba08-19bd7ed8e73c");
pub const SPOT_LIGHT_COMPONENT_TYPE_ID: Uuid = uuid!("2d7d6141-3e65-5989-8aea-a85a2bdbe342");

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshComponentIdentity;

impl az_core::AzTypeInfo for MeshComponentIdentity {
    const NAME: &'static str = "MeshComponent";
    const TYPE_ID: Uuid = MESH_COMPONENT_TYPE_ID;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandleIdentity;

impl az_core::AzTypeInfo for MaterialHandleIdentity {
    const NAME: &'static str = "MaterialHandle";
    const TYPE_ID: Uuid = MATERIAL_HANDLE_TYPE_ID;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraComponentIdentity;

impl az_core::AzTypeInfo for CameraComponentIdentity {
    const NAME: &'static str = "CameraComponent";
    const TYPE_ID: Uuid = CAMERA_COMPONENT_TYPE_ID;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightComponentIdentity;

impl az_core::AzTypeInfo for LightComponentIdentity {
    const NAME: &'static str = "LightComponent";
    const TYPE_ID: Uuid = LIGHT_COMPONENT_TYPE_ID;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightConfigurationIdentity;

impl az_core::AzTypeInfo for LightConfigurationIdentity {
    const NAME: &'static str = "LightConfiguration";
    const TYPE_ID: Uuid = LIGHT_CONFIGURATION_TYPE_ID;
}
