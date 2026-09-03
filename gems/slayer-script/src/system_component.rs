//! Process-level `SlayerScript` service marker.

use az_core::component::Component as AzComponent;
use az_core::rtti::AzTypeRegistration;
use az_derive::AzRtti;
use az_prefab::{Prefab, PrefabType, ReflectPrefab};
use bevy::{
    ecs::reflect::ReflectComponent,
    prelude::{App, Component, Plugin, Reflect},
    reflect::{
        GetTypeRegistration, ReflectDeserialize, ReflectSerialize, TypePath,
        std_traits::ReflectDefault,
    },
};
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use crate::{
    EntityEvent, EntityEvents, OpacityEvent, RotationEvent, SlayerScriptData,
    SlayerScriptDataContainer, SlayerScriptEditCrc, SlayerScriptEditLiteral, SlayerScriptLiteral,
    SlayerScriptName,
};

/// Native `SerializeContext` UUID for `SlayerScriptSystemComponent`.
pub const SLAYER_SCRIPT_SYSTEM_COMPONENT_TYPE_UUID: Uuid =
    uuid!("C924A868-BD47-49E7-8039-4C0FB99F74E1");

/// Process-level marker selecting the generic `SlayerScript` runtime service.
///
/// The native component has no reflected fields beyond its AZ component base.
/// Runtime instances and project-specific modules are deliberately not stored
/// on this authoring component.
#[derive(
    AzRtti,
    Component,
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
#[az_rtti(
    name = "SlayerScriptSystemComponent",
    SLAYER_SCRIPT_SYSTEM_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.slayer_script.SlayerScriptSystemComponent", version = 1)]
pub struct SlayerScriptSystemComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
}

/// Registers generic `SlayerScript` reflected authoring types.
pub struct SlayerScriptPlugin;

impl Plugin for SlayerScriptPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SlayerScriptSystemComponent>()
            .register_type_data::<SlayerScriptSystemComponent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<SlayerScriptSystemComponent, az_core::ReflectAzRtti>()
            .register_type::<SlayerScriptData>()
            .register_type_data::<SlayerScriptData, az_core::ReflectAzTypeInfo>()
            .register_type_data::<SlayerScriptData, az_core::ReflectAzRtti>()
            .register_type::<SlayerScriptLiteral>()
            .register_type_data::<SlayerScriptLiteral, az_core::ReflectAzTypeInfo>()
            .register_type_data::<SlayerScriptLiteral, az_core::ReflectAzRtti>()
            .register_type::<SlayerScriptEditLiteral>()
            .register_type_data::<SlayerScriptEditLiteral, az_core::ReflectAzTypeInfo>()
            .register_type_data::<SlayerScriptEditLiteral, az_core::ReflectAzRtti>()
            .register_type::<SlayerScriptName>()
            .register_type::<SlayerScriptEditCrc>()
            .register_type_data::<SlayerScriptEditCrc, az_core::ReflectAzTypeInfo>()
            .register_type_data::<SlayerScriptEditCrc, az_core::ReflectAzRtti>()
            .register_type::<EntityEvents>()
            .register_type::<EntityEvent>()
            .register_type_data::<EntityEvent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<EntityEvent, az_core::ReflectAzRtti>()
            .register_type::<OpacityEvent>()
            .register_type_data::<OpacityEvent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<OpacityEvent, az_core::ReflectAzRtti>()
            .register_type::<RotationEvent>()
            .register_type_data::<RotationEvent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<RotationEvent, az_core::ReflectAzRtti>();
    }
}

/// Registers the one project-selected closed source-container monomorph.
///
/// The generic native container UUID is independent of its Rust source type.
/// Registering both the base default and a project enum would therefore map one
/// UUID to two production `TypeId`s. The project owning the closed offline
/// source projection calls this once with that projection; the generic plugin
/// deliberately does not register a competing default monomorph.
pub fn register_source_container<S>(app: &mut App)
where
    S: crate::SlayerScriptSource,
    SlayerScriptDataContainer<S>:
        GetTypeRegistration + Reflect + TypePath + az_core::AzTypeInfo + az_core::AzRtti,
{
    app.register_type::<SlayerScriptDataContainer<S>>()
        .register_type_data::<SlayerScriptDataContainer<S>, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SlayerScriptDataContainer<S>, az_core::ReflectAzRtti>();
}

/// The AZ types this gem owns.
///
/// The named const is the item the derive emitted. Dropping it from this list
/// does not compile.
#[must_use]
pub const fn types() -> [AzTypeRegistration; 1] {
    [SlayerScriptSystemComponent::REGISTRATION]
}

/// The reflected authoring types this gem owns, for an asset worker compiling
/// prefabs.
///
/// The closed source-container monomorph is deliberately absent. The generic
/// native container UUID is independent of its Rust source type, so registering
/// a default here alongside a project's own enum would map one UUID to two
/// production `TypeId`s. The project that owns the closed offline source
/// projection registers `PrefabType::of::<SlayerScriptDataContainer<Its>>()`;
/// this gem does not choose a competing default.
#[must_use]
pub fn prefab_types() -> [PrefabType; 10] {
    [
        PrefabType::of::<SlayerScriptSystemComponent>(),
        PrefabType::of::<SlayerScriptData>(),
        PrefabType::of::<SlayerScriptLiteral>(),
        PrefabType::of::<SlayerScriptEditLiteral>(),
        PrefabType::of::<SlayerScriptName>(),
        PrefabType::of::<SlayerScriptEditCrc>(),
        PrefabType::of::<EntityEvents>(),
        PrefabType::of::<EntityEvent>(),
        PrefabType::of::<OpacityEvent>(),
        PrefabType::of::<RotationEvent>(),
    ]
}

#[cfg(test)]
mod tests {
    use az_core::AzTypeInfo;
    use bevy::prelude::App;
    use uuid::uuid;

    use super::{SlayerScriptPlugin, SlayerScriptSystemComponent, register_source_container};
    use crate::{
        EntityEvent, SlayerScriptData, SlayerScriptDataContainer, SlayerScriptEditCrc,
        SlayerScriptLiteral,
    };

    #[test]
    fn system_component_keeps_the_native_type_id() {
        assert_eq!(
            SlayerScriptSystemComponent::TYPE_ID,
            uuid!("C924A868-BD47-49E7-8039-4C0FB99F74E1")
        );
    }

    #[test]
    fn plugin_registers_system_component_reflection() {
        let mut app = App::new();
        app.add_plugins(SlayerScriptPlugin);
        {
            let registry = app
                .world()
                .resource::<bevy::ecs::reflect::AppTypeRegistry>();
            assert!(
                registry
                    .read()
                    .get(std::any::TypeId::of::<SlayerScriptDataContainer>())
                    .is_none(),
                "the engine plugin must not choose a concrete project source closure"
            );
        }
        register_source_container::<SlayerScriptData>(&mut app);
        let registry = app
            .world()
            .resource::<bevy::ecs::reflect::AppTypeRegistry>();
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<SlayerScriptSystemComponent>())
                .is_some()
        );
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<SlayerScriptLiteral>())
                .is_some()
        );
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<EntityEvent>())
                .is_some()
        );
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<SlayerScriptEditCrc>())
                .is_some()
        );
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<SlayerScriptData>())
                .is_some()
        );
        assert!(
            registry
                .read()
                .get(std::any::TypeId::of::<SlayerScriptDataContainer>())
                .is_some()
        );
    }
}
