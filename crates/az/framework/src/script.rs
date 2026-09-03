//! Lua script component data and reflected property payloads.

use az_core::component::{Component as AzComponent, ComponentId};
use az_core::crc::Crc32;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// `AzFramework::ScriptComponent` AZ component UUID.
pub const SCRIPT_COMPONENT_TYPE_ID: Uuid = uuid!("8D1BC97E-C55D-4D34-A460-E63C57CD0D4B");

/// `AzFramework::ScriptPropertyGroup` AZ type UUID.
pub const SCRIPT_PROPERTY_GROUP_TYPE_ID: Uuid = uuid!("79682522-2F81-4B36-9FC2-A091C7504F7F");

/// `AzFramework::ScriptProperty` AZ type UUID.
pub const SCRIPT_PROPERTY_TYPE_ID: Uuid = uuid!("D227D737-F1ED-4FB3-A1FB-38E4985D2E7A");

/// `AzFramework::ScriptPropertyNil` AZ type UUID.
pub const SCRIPT_PROPERTY_NIL_TYPE_ID: Uuid = uuid!("ACAD23F6-5E75-460E-BD77-1B477750264F");

/// `AzFramework::ScriptPropertyBoolean` AZ type UUID.
pub const SCRIPT_PROPERTY_BOOLEAN_TYPE_ID: Uuid = uuid!("EA7335F8-5B9F-4744-B805-FEF9240451BD");

/// `AzFramework::ScriptPropertyNumber` AZ type UUID.
pub const SCRIPT_PROPERTY_NUMBER_TYPE_ID: Uuid = uuid!("5BCDFDEB-A75D-4E83-BB74-C45299CB9826");

/// `AzFramework::ScriptPropertyString` AZ type UUID.
pub const SCRIPT_PROPERTY_STRING_TYPE_ID: Uuid = uuid!("A0229C6D-B010-47E7-8985-EE220FC7BFAF");

/// `AzFramework::ScriptPropertyGenericClass` AZ type UUID.
pub const SCRIPT_PROPERTY_GENERIC_CLASS_TYPE_ID: Uuid =
    uuid!("80618224-814C-44D4-A7B8-14B5A36F96ED");

/// `AzFramework::ScriptPropertyNumberArray` AZ type UUID.
pub const SCRIPT_PROPERTY_NUMBER_ARRAY_TYPE_ID: Uuid =
    uuid!("76609A01-46CA-442E-8BA6-251D529886AF");

/// `AzFramework::ScriptPropertyBooleanArray` AZ type UUID.
pub const SCRIPT_PROPERTY_BOOLEAN_ARRAY_TYPE_ID: Uuid =
    uuid!("3A83958C-26C7-4A59-B6D7-A7805B0EC756");

/// `AzFramework::ScriptPropertyStringArray` AZ type UUID.
pub const SCRIPT_PROPERTY_STRING_ARRAY_TYPE_ID: Uuid =
    uuid!("899993A5-D717-41BB-B89B-04A27952CA6D");

/// `AzFramework::ScriptPropertyGenericClassArray` AZ type UUID.
pub const SCRIPT_PROPERTY_GENERIC_CLASS_ARRAY_TYPE_ID: Uuid =
    uuid!("28E986DD-CF7C-404D-9BEE-EEE067180CD1");

/// `AZ::ScriptPropertyAsset` AZ type UUID.
pub const SCRIPT_PROPERTY_ASSET_TYPE_ID: Uuid = uuid!("4D4B7176-A6E1-4BB9-A7B0-5977EC724CCB");

/// `AZ::ScriptPropertyEntityRef` AZ type UUID.
pub const SCRIPT_PROPERTY_ENTITY_REF_TYPE_ID: Uuid = uuid!("68EDE6C3-0A89-4C50-A86E-06C058C9F862");

/// Lua script component.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[az_rtti(
    name = "AzFramework::ScriptComponent",
    SCRIPT_COMPONENT_TYPE_ID,
    az_core::component::Component,
    crate::NetBindable,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
// Tag is namespaced (not the bare wire class name) to avoid colliding with the
// project-codegen type's bare "AzFramework::ScriptComponent" tag.
#[prefab(tag = "azoth.az_framework.ScriptComponent", version = 1)]
pub struct ScriptComponent {
    pub az_component: AzComponent,
    pub net_bindable: crate::NetBindable,
    pub context_id: u32,
    pub properties: ScriptPropertyGroup,
    pub name: String,
    pub id: Option<String>,
    pub script: Option<String>,
    pub run_on_server: bool,
    pub run_on_client: bool,
}

impl Default for ScriptComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptComponent {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            az_component: AzComponent {
                id: ComponentId::INVALID,
            },
            net_bindable: crate::NetBindable {
                is_net_sync_enabled: false,
            },
            context_id: 0,
            properties: ScriptPropertyGroup::new(),
            name: String::new(),
            id: None,
            script: None,
            run_on_server: true,
            run_on_client: true,
        }
    }
}

/// Script property group tree.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptPropertyGroup {
    pub name: String,
    pub id: Option<String>,
    pub properties: Vec<ScriptProperty>,
    pub groups: Vec<Self>,
}

impl ScriptPropertyGroup {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            name: String::new(),
            id: None,
            properties: Vec::new(),
            groups: Vec::new(),
        }
    }
}

/// Script property identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptPropertyKey {
    pub id: u64,
    pub name: String,
}

impl ScriptPropertyKey {
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }

    #[must_use]
    pub fn from_name(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: u64::from(Crc32::from_str_lower(&name).value()),
            name,
        }
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the `self.id <= u32::MAX` guard proves the cast is lossless; `u32::try_from` \
                  is not usable because this accessor is a `const fn`"
    )]
    pub const fn crc32(&self) -> Option<Crc32> {
        if self.id <= u32::MAX as u64 {
            Some(Crc32::from_u32(self.id as u32))
        } else {
            None
        }
    }
}

/// Script property value.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptProperty {
    pub key: ScriptPropertyKey,
    pub value: ScriptPropertyValue,
}

impl ScriptProperty {
    #[must_use]
    pub const fn new(key: ScriptPropertyKey, value: ScriptPropertyValue) -> Self {
        Self { key, value }
    }
}

/// Lua-compatible script property payload.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum ScriptPropertyValue {
    #[default]
    Nil,
    Boolean(bool),
    Number(f64),
    String(String),
    BooleanArray(Vec<bool>),
    NumberArray(Vec<f64>),
    StringArray(Vec<String>),
    Asset(Option<String>),
    EntityRef(Option<u64>),
    DynamicClass(ScriptDynamicClassValue),
    DynamicClassArray(ScriptDynamicClassArrayValue),
}

/// Dynamic class script property value.
///
/// Native reflection fabricates `DynamicSerializableField::m_data` at load time.
/// The payload therefore carries its concrete type on the instance rather than
/// through a statically declared field. `payload_type_id` preserves that
/// identity while [`ScriptDynamicValue`] keeps the typed value inspectable.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptDynamicClassValue {
    pub type_name: Option<String>,
    pub payload_type_id: Option<String>,
    pub payload: ScriptDynamicValue,
}

/// Engine-neutral value tree for a reflected dynamic script payload.
///
/// Struct fields use a sequence rather than a map so reflected field order is
/// deterministic in authored assets and binary products.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum ScriptDynamicValue {
    #[default]
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    Number(f64),
    String(String),
    Uuid(String),
    EntityRef(Option<u64>),
    Vector2(Vec2),
    Vector3(Vec3),
    Color(LinearRgba),
    List(Vec<Self>),
    Struct(Vec<ScriptDynamicField>),
}

/// One named field in a reflected dynamic script struct.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptDynamicField {
    pub name: String,
    pub value: ScriptDynamicValue,
}

impl ScriptDynamicField {
    #[must_use]
    pub fn new(name: impl Into<String>, value: ScriptDynamicValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// Dynamic class array script property value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ScriptDynamicClassArrayValue {
    pub element_type_name: Option<String>,
    pub len: u32,
}

/// Register `AzFramework` script data.
pub struct AzFrameworkPlugin;

impl Plugin for AzFrameworkPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<u32>()
            .register_type::<f64>()
            .register_type::<AzComponent>()
            .register_type::<ComponentId>()
            .register_type::<crate::NetBindable>()
            .register_type::<Option<u64>>()
            .register_type::<Option<String>>()
            .register_type::<Vec<bool>>()
            .register_type::<Vec<f64>>()
            .register_type::<Vec<String>>()
            .register_type::<Vec<ScriptDynamicField>>()
            .register_type::<Vec<ScriptDynamicValue>>()
            .register_type::<Vec<ScriptProperty>>()
            .register_type::<Vec<ScriptPropertyGroup>>()
            .register_type::<ScriptComponent>()
            .register_type::<ScriptPropertyGroup>()
            .register_type::<ScriptPropertyKey>()
            .register_type::<ScriptProperty>()
            .register_type::<ScriptPropertyValue>()
            .register_type::<ScriptDynamicClassValue>()
            .register_type::<ScriptDynamicField>()
            .register_type::<ScriptDynamicValue>()
            .register_type::<ScriptDynamicClassArrayValue>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_component_defaults_match_source_defaults() {
        let component = ScriptComponent::default();

        assert_eq!(component.context_id, 0);
        assert!(component.properties.properties.is_empty());
        assert!(component.name.is_empty());
        assert!(component.id.is_none());
        assert!(component.script.is_none());
        assert!(component.run_on_server);
        assert!(component.run_on_client);
        assert!(!component.net_bindable.is_net_sync_enabled);
    }

    #[test]
    fn script_property_key_uses_lumberyard_crc32() {
        let key = ScriptPropertyKey::from_name("Enabled");

        assert_eq!(key.crc32(), Some(Crc32::from_str_lower("Enabled")));
        assert_eq!(key.name, "Enabled");
    }

    #[test]
    fn plugin_registers_script_component_type() {
        let mut app = App::new();
        app.add_plugins(AzFrameworkPlugin);

        let registered = {
            let registry = app.world().resource::<AppTypeRegistry>().read();
            registry
                .get(std::any::TypeId::of::<ScriptComponent>())
                .is_some()
        };
        assert!(registered);
    }
}
