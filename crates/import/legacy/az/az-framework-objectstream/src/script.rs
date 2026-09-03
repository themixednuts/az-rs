//! `ObjectStream` readers for `AzFramework` script data.

use az_core::component::{ComponentId, EntityId};
use az_objectstream::asset_reference::{AssetHintError, asset_hint_from_data_owned};
use az_objectstream::context::ContainerShape;
use az_objectstream::query::type_name as objectstream_type_name;
use az_objectstream::value::{self, FieldAccess, ObjectStreamValueError};
use az_objectstream::{Element, types};
use thiserror::Error;
use uuid::Uuid;

use az_framework::{
    SCRIPT_COMPONENT_TYPE_ID, SCRIPT_PROPERTY_ASSET_TYPE_ID, SCRIPT_PROPERTY_BOOLEAN_ARRAY_TYPE_ID,
    SCRIPT_PROPERTY_BOOLEAN_TYPE_ID, SCRIPT_PROPERTY_ENTITY_REF_TYPE_ID,
    SCRIPT_PROPERTY_GENERIC_CLASS_ARRAY_TYPE_ID, SCRIPT_PROPERTY_GENERIC_CLASS_TYPE_ID,
    SCRIPT_PROPERTY_GROUP_TYPE_ID, SCRIPT_PROPERTY_NIL_TYPE_ID,
    SCRIPT_PROPERTY_NUMBER_ARRAY_TYPE_ID, SCRIPT_PROPERTY_NUMBER_TYPE_ID,
    SCRIPT_PROPERTY_STRING_ARRAY_TYPE_ID, SCRIPT_PROPERTY_STRING_TYPE_ID, SCRIPT_PROPERTY_TYPE_ID,
    ScriptComponent, ScriptDynamicClassArrayValue, ScriptDynamicClassValue, ScriptDynamicValue,
    ScriptProperty, ScriptPropertyGroup, ScriptPropertyKey, ScriptPropertyValue,
};

#[derive(Debug, Error)]
pub enum ScriptObjectStreamError {
    #[error("script ObjectStream field `{field}` could not be read")]
    Field {
        field: &'static str,
        #[source]
        source: ObjectStreamValueError,
    },
    #[error("script context id {0} does not fit in u32")]
    ContextIdOutOfRange(u64),
    #[error("expected {expected}, got {actual}")]
    UnexpectedType {
        expected: &'static str,
        actual: Uuid,
    },
    #[error("unknown script property type {0}")]
    ScriptPropertyType(Uuid),
    #[error("{owner} smart pointer has {actual} children, expected exactly one")]
    InvalidPointeeCount { owner: &'static str, actual: usize },
    #[error("script dynamic class array length {0} does not fit in u32")]
    DynamicClassArrayLengthOutOfRange(usize),
    #[error("unknown script dynamic class payload type {0}")]
    DynamicClassPayloadType(Uuid),
    #[error("script ObjectStream asset field `{field}` could not be read")]
    AssetHint {
        field: &'static str,
        #[source]
        source: AssetHintError,
    },
}

/// Read an `AzFramework::ScriptComponent` context id.
///
/// Lumberyard-authored `ObjectStreams` have used both `unsigned int`
/// and widened `AZ::u64` scalars for this field. The runtime model
/// stores the value as `u32`, so this helper keeps the scalar widening
/// and bounds check together.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "ContextID"` if
/// `element` is not a readable unsigned scalar, or
/// [`ScriptObjectStreamError::ContextIdOutOfRange`] if the widened value does
/// not fit in `u32`.
pub fn read_script_context_id(element: &Element) -> Result<u32, ScriptObjectStreamError> {
    let value =
        value::read_u64_scalar(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "ContextID",
            source,
        })?;
    u32::try_from(value).map_err(|_| ScriptObjectStreamError::ContextIdOutOfRange(value))
}

/// Read an `AzFramework::ScriptComponent` `ObjectStream` element.
///
/// The component has accumulated editor spelling differences across
/// Lumberyard-authored assets (`ContextID`/`contextId`,
/// `Properties`/`properties`) and inherits serialized AZ component and
/// `NetBindable` base payloads. Keeping those layout choices here lets
/// callers treat the component as `AzFramework` data instead of owning
/// its reflected `ObjectStream` shape.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::UnexpectedType`] if `element`'s semantic
/// type is not `AzFramework::ScriptComponent`, or
/// [`ScriptObjectStreamError::Field`] when a present field fails to decode —
/// `BaseClass1.ID` as a `u64`, or `m_isNetSyncEnabled`, `IsRunOnServer` and
/// `IsRunOnClient` as booleans. A `Script` field that is not a well-formed
/// asset hint yields [`ScriptObjectStreamError::AssetHint`]. Also propagates
/// any error [`read_script_context_id`] or [`read_script_property_group`]
/// returns. Absent optional fields are not errors; they keep their
/// [`ScriptComponent::default`] value.
pub fn read_script_component(
    element: &Element,
) -> Result<ScriptComponent, ScriptObjectStreamError> {
    let actual =
        value::semantic_type_id(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "ScriptComponent",
            source,
        })?;
    if actual != SCRIPT_COMPONENT_TYPE_ID {
        return Err(ScriptObjectStreamError::UnexpectedType {
            expected: "AzFramework::ScriptComponent",
            actual,
        });
    }

    let mut component = ScriptComponent::default();
    if let Some(base) = value::child_by_field(element, "BaseClass1")
        && let Some(field) = value::child_by_field_any(base, &["ID", "Id"])
    {
        component.az_component.id = value::read_u64_scalar(field)
            .map(ComponentId::new)
            .map_err(|source| ScriptObjectStreamError::Field {
                field: "BaseClass1.ID",
                source,
            })?;
    }
    if let Some(field) = value::child_by_field_any(element, &["ContextID", "contextId"]) {
        component.context_id = read_script_context_id(field)?;
    }
    if let Some(field) = value::child_by_field_any(element, &["Properties", "properties"]) {
        component.properties = read_script_property_group(field)?;
    }
    if let Some(field) = value::child_by_field(element, "Script") {
        component.script = asset_hint_from_data_owned(field).map_err(|source| {
            ScriptObjectStreamError::AssetHint {
                field: "Script",
                source,
            }
        })?;
    }
    if let Some(base) = value::child_by_field(element, "BaseClass2")
        && let Some(field) =
            value::child_by_field_any(base, &["m_isNetSyncEnabled", "m_isSyncEnabled"])
    {
        component.net_bindable.is_net_sync_enabled =
            value::read_bool(field).map_err(|source| ScriptObjectStreamError::Field {
                field: "m_isNetSyncEnabled",
                source,
            })?;
    }
    if let Some(field) = value::child_by_field(element, "IsRunOnServer") {
        component.run_on_server =
            value::read_bool(field).map_err(|source| ScriptObjectStreamError::Field {
                field: "IsRunOnServer",
                source,
            })?;
    }
    if let Some(field) = value::child_by_field(element, "IsRunOnClient") {
        component.run_on_client =
            value::read_bool(field).map_err(|source| ScriptObjectStreamError::Field {
                field: "IsRunOnClient",
                source,
            })?;
    }

    Ok(component)
}

/// Read an `AzFramework::ScriptPropertyNumber` scalar value.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "value"` if
/// `element` does not hold a floating-point scalar of a width the reader
/// accepts.
pub fn read_script_number_scalar(element: &Element) -> Result<f64, ScriptObjectStreamError> {
    value::read_f64_scalar(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "value",
        source,
    })
}

/// Read an `AzFramework::ScriptPropertyBoolean` scalar value.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "value"` if
/// `element` does not hold a single-byte boolean payload.
pub fn read_script_bool_scalar(element: &Element) -> Result<bool, ScriptObjectStreamError> {
    value::read_bool(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "value",
        source,
    })
}

/// Read an `AzFramework::ScriptPropertyString` scalar value.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "value"` if
/// `element`'s payload is not a string the reader can decode. An empty or
/// whitespace-only string is `Ok(None)`, not an error.
pub fn read_script_string_scalar(
    element: &Element,
) -> Result<Option<String>, ScriptObjectStreamError> {
    value::read_trimmed_string_owned(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "value",
        source,
    })
}

/// Read an `AzFramework::ScriptPropertyBooleanArray` value vector.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "values"` if
/// `element` is not a sequence container or any child is not a readable
/// boolean.
pub fn read_script_bool_vector(element: &Element) -> Result<Vec<bool>, ScriptObjectStreamError> {
    value::read_bool_vector(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "values",
        source,
    })
}

/// Read an `AzFramework::ScriptPropertyNumberArray` value vector.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "values"` if
/// `element` is not a sequence container or any child is not a readable
/// floating-point scalar.
pub fn read_script_number_vector(element: &Element) -> Result<Vec<f64>, ScriptObjectStreamError> {
    value::read_f64_vector(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "values",
        source,
    })
}

/// Read an `AzFramework::ScriptPropertyStringArray` value vector.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "values"` if
/// `element` is not a sequence container or any child is not a readable
/// string.
pub fn read_script_string_vector(
    element: &Element,
) -> Result<Vec<String>, ScriptObjectStreamError> {
    value::read_string_vector_owned(element).map_err(|source| ScriptObjectStreamError::Field {
        field: "values",
        source,
    })
}

/// Read the `AzFramework::ScriptPropertyKey` payload shared by all
/// reflected script property variants.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "Id"` if a present
/// `Id`/`id` child is not a readable `u64`, or with `field: "Name"` if a
/// present `Name`/`name` child is not a readable string. Both fields are
/// optional; when absent the corresponding
/// [`ScriptPropertyKey::default`] value is kept.
pub fn read_script_property_key(
    element: &Element,
) -> Result<ScriptPropertyKey, ScriptObjectStreamError> {
    let mut key = ScriptPropertyKey::default();
    if let Some(field) = value::child_by_field_any(element, &["Id", "id"]) {
        key.id =
            value::read_u64_scalar(field).map_err(|source| ScriptObjectStreamError::Field {
                field: "Id",
                source,
            })?;
    }
    if let Some(field) = value::child_by_field_any(element, &["Name", "name"])
        && let Some(name) = value::read_trimmed_string_owned(field).map_err(|source| {
            ScriptObjectStreamError::Field {
                field: "Name",
                source,
            }
        })?
    {
        key.name = name;
    }
    Ok(key)
}

/// Read an `AZ::ScriptPropertyEntityRef` value payload.
///
/// Legacy data may encode either a reflected `AZ::EntityId` with
/// `id`/`Id`/`ID` aliases or a script wrapper with `Id`/`id`
/// directly under the value element.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::Field`] with `field: "value"` if
/// `element`'s semantic type cannot be read, if none of the accepted id
/// aliases (`id`/`Id`/`ID` for a reflected `AZ::EntityId`, `Id`/`id` for the
/// script wrapper) is present, or if the one that is present is not a readable
/// `u64`.
pub fn read_script_entity_ref(element: &Element) -> Result<u64, ScriptObjectStreamError> {
    let mut fields = value::ElementFields::new(element);
    let aliases: &[&str] =
        if value::semantic_type_id(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "value",
            source,
        })? == types::ENTITY_ID
        {
            &["id", "Id", "ID"]
        } else {
            &["Id", "id"]
        };
    fields
        .required_any::<u64>(aliases)
        .map(|(_, id)| id)
        .map_err(|source| ScriptObjectStreamError::Field {
            field: "value",
            source,
        })
}

/// Read the `DynamicSerializableField::m_data` payload of a dynamic class property.
///
/// `m_data` is fabricated by native reflection at load time and carries no static
/// type, so its concrete type is the instance's stored `TypeId`. This generic
/// `AzFramework` reader knows the engine-owned `AZ::EntityId` payload. Project
/// importers can extend the engine-neutral [`ScriptDynamicValue`] tree with
/// project-owned reflected types.
///
/// `AZ::EntityId::InvalidEntityId` is `0x00000000FFFFFFFF` in O3DE
/// (`Code/Framework/AzCore/AzCore/Component/EntityId.h:41`), so the engine itself
/// treats that value as "no entity"; it normalises to `None` rather than
/// manufacturing a reference to an entity that cannot exist.
fn read_dynamic_class_payload(
    element: &Element,
) -> Result<ScriptDynamicValue, ScriptObjectStreamError> {
    let Some(payload) = value::child_by_field(element, "m_data") else {
        return Ok(ScriptDynamicValue::Unit);
    };
    let actual =
        value::semantic_type_id(payload).map_err(|source| ScriptObjectStreamError::Field {
            field: "m_data",
            source,
        })?;
    if actual != types::ENTITY_ID {
        return Err(ScriptObjectStreamError::DynamicClassPayloadType(actual));
    }
    let id = read_script_entity_ref(payload)?;
    Ok(ScriptDynamicValue::EntityRef(
        EntityId::new(id).is_valid().then_some(id),
    ))
}

/// Read an `AzFramework::ScriptPropertyGroup` tree.
///
/// This owns the recursive `ScriptProperty` `ObjectStream` shape used by
/// script components. Callers should keep component-specific fields
/// local, but not duplicate the property tree format.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::InvalidPointeeCount`] if `element` is a
/// smart pointer that does not wrap exactly one child,
/// [`ScriptObjectStreamError::UnexpectedType`] if the pointee's semantic type
/// is not `AzFramework::ScriptPropertyGroup`, or
/// [`ScriptObjectStreamError::Field`] if `Name`/`Id` are unreadable strings or
/// if `Properties`/`Groups` are not sequence containers. Recursion into nested
/// groups and into [`read_script_property`] propagates their errors unchanged.
pub fn read_script_property_group(
    element: &Element,
) -> Result<ScriptPropertyGroup, ScriptObjectStreamError> {
    let element = strict_pointee(element, "ScriptPropertyGroup")?;
    let actual =
        value::semantic_type_id(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "ScriptPropertyGroup",
            source,
        })?;
    if actual != SCRIPT_PROPERTY_GROUP_TYPE_ID {
        return Err(ScriptObjectStreamError::UnexpectedType {
            expected: "AzFramework::ScriptPropertyGroup",
            actual,
        });
    }

    let mut group = ScriptPropertyGroup::default();
    if let Some(field) = value::child_by_field(element, "Name")
        && let Some(name) = read_trimmed_string_field(field, "Name")?
    {
        group.name = name;
    }
    if let Some(field) = value::child_by_field(element, "Id") {
        group.id = read_trimmed_string_field(field, "Id")?;
    }
    if let Some(properties) = value::child_by_field_any(element, &["Properties", "properties"]) {
        require_sequence(properties, "Properties")?;
        group.properties = properties
            .children()
            .iter()
            .map(read_script_property)
            .collect::<Result<_, _>>()?;
    }
    if let Some(groups) = value::child_by_field(element, "Groups") {
        require_sequence(groups, "Groups")?;
        group.groups = groups
            .children()
            .iter()
            .map(read_script_property_group)
            .collect::<Result<_, _>>()?;
    }

    Ok(group)
}

/// Read an `AzFramework::ScriptProperty` value.
///
/// # Errors
///
/// Returns [`ScriptObjectStreamError::InvalidPointeeCount`] if `element` is a
/// smart pointer that does not wrap exactly one child, or
/// [`ScriptObjectStreamError::ScriptPropertyType`] if the pointee's semantic
/// type is not one of the reflected `ScriptProperty*` types this reader
/// handles. Decoding the payload propagates the matching scalar or vector
/// reader's [`ScriptObjectStreamError::Field`]; an asset payload can yield
/// [`ScriptObjectStreamError::AssetHint`]; a generic-class payload whose
/// `m_data` is not an `AZ::EntityId` yields
/// [`ScriptObjectStreamError::DynamicClassPayloadType`]; and a generic-class
/// array longer than `u32::MAX` yields
/// [`ScriptObjectStreamError::DynamicClassArrayLengthOutOfRange`]. Also
/// propagates any error [`read_script_property_key`] returns.
pub fn read_script_property(element: &Element) -> Result<ScriptProperty, ScriptObjectStreamError> {
    let element = strict_pointee(element, "ScriptProperty")?;
    let base = base_class_of_semantic_type(element, SCRIPT_PROPERTY_TYPE_ID)?.unwrap_or(element);
    let key = read_script_property_key(base)?;
    let value = read_script_property_value(element)?;
    Ok(ScriptProperty::new(key, value))
}

/// Decode the payload half of a `ScriptProperty`, dispatching on its reflected
/// type id.
///
/// `element` is the already-dereferenced property element, not a smart pointer.
fn read_script_property_value(
    element: &Element,
) -> Result<ScriptPropertyValue, ScriptObjectStreamError> {
    let value =
        match value::semantic_type_id(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "ScriptProperty",
            source,
        })? {
            SCRIPT_PROPERTY_TYPE_ID | SCRIPT_PROPERTY_NIL_TYPE_ID => ScriptPropertyValue::Nil,
            SCRIPT_PROPERTY_BOOLEAN_TYPE_ID => {
                let value = value::child_by_field(element, "value")
                    .map(read_script_bool_scalar)
                    .transpose()?
                    .unwrap_or(false);
                ScriptPropertyValue::Boolean(value)
            }
            SCRIPT_PROPERTY_NUMBER_TYPE_ID => {
                let value = value::child_by_field(element, "value")
                    .map(read_script_number_scalar)
                    .transpose()?
                    .unwrap_or(0.0);
                ScriptPropertyValue::Number(value)
            }
            SCRIPT_PROPERTY_STRING_TYPE_ID => {
                let value = value::child_by_field(element, "value")
                    .map(read_script_string_scalar)
                    .transpose()?
                    .flatten()
                    .unwrap_or_default();
                ScriptPropertyValue::String(value)
            }
            SCRIPT_PROPERTY_BOOLEAN_ARRAY_TYPE_ID => {
                let values = value::child_by_field(element, "values")
                    .map(read_script_bool_vector)
                    .transpose()?
                    .unwrap_or_default();
                ScriptPropertyValue::BooleanArray(values)
            }
            SCRIPT_PROPERTY_NUMBER_ARRAY_TYPE_ID => {
                let values = value::child_by_field(element, "values")
                    .map(read_script_number_vector)
                    .transpose()?
                    .unwrap_or_default();
                ScriptPropertyValue::NumberArray(values)
            }
            SCRIPT_PROPERTY_STRING_ARRAY_TYPE_ID => {
                let values = value::child_by_field(element, "values")
                    .map(read_script_string_vector)
                    .transpose()?
                    .unwrap_or_default();
                ScriptPropertyValue::StringArray(values)
            }
            SCRIPT_PROPERTY_ASSET_TYPE_ID => {
                let path = value::child_by_field(element, "value")
                    .map(read_script_asset_hint)
                    .transpose()?
                    .flatten();
                ScriptPropertyValue::Asset(path)
            }
            SCRIPT_PROPERTY_ENTITY_REF_TYPE_ID => {
                let entity_id = value::child_by_field(element, "value")
                    .map(read_script_entity_ref)
                    .transpose()?;
                ScriptPropertyValue::EntityRef(entity_id)
            }
            SCRIPT_PROPERTY_GENERIC_CLASS_TYPE_ID => {
                let field = value::child_by_field(element, "value");
                let type_name = field.map(objectstream_type_name);
                let payload = field
                    .map(read_dynamic_class_payload)
                    .transpose()?
                    .unwrap_or_default();
                ScriptPropertyValue::DynamicClass(ScriptDynamicClassValue {
                    type_name,
                    payload_type_id: None,
                    payload,
                })
            }
            SCRIPT_PROPERTY_GENERIC_CLASS_ARRAY_TYPE_ID => {
                let values = value::child_by_field(element, "values");
                let len = values
                    .map(|field| {
                        require_sequence(field, "values")?;
                        u32::try_from(field.children().len()).map_err(|_| {
                            ScriptObjectStreamError::DynamicClassArrayLengthOutOfRange(
                                field.children().len(),
                            )
                        })
                    })
                    .transpose()?
                    .unwrap_or(0);
                let element_type_name =
                    value::child_by_field(element, "elementType").map(objectstream_type_name);
                ScriptPropertyValue::DynamicClassArray(ScriptDynamicClassArrayValue {
                    element_type_name,
                    len,
                })
            }
            actual => return Err(ScriptObjectStreamError::ScriptPropertyType(actual)),
        };

    Ok(value)
}

fn read_script_asset_hint(element: &Element) -> Result<Option<String>, ScriptObjectStreamError> {
    asset_hint_from_data_owned(element).map_err(|source| ScriptObjectStreamError::AssetHint {
        field: "value",
        source,
    })
}

fn read_trimmed_string_field(
    element: &Element,
    field: &'static str,
) -> Result<Option<String>, ScriptObjectStreamError> {
    value::read_trimmed_string_owned(element)
        .map_err(|source| ScriptObjectStreamError::Field { field, source })
}

fn require_sequence(element: &Element, field: &'static str) -> Result<(), ScriptObjectStreamError> {
    value::require_container_shape(element, ContainerShape::Sequence, "AZStd::vector")
        .map_err(|source| ScriptObjectStreamError::Field { field, source })
}

fn strict_pointee<'a>(
    element: &'a Element,
    owner: &'static str,
) -> Result<&'a Element, ScriptObjectStreamError> {
    if element.container_shape() != Some(ContainerShape::SmartPointer) {
        return Ok(element);
    }
    match element.children() {
        [pointee] => Ok(pointee),
        children => Err(ScriptObjectStreamError::InvalidPointeeCount {
            owner,
            actual: children.len(),
        }),
    }
}

fn base_class_of_semantic_type(
    element: &Element,
    expected: Uuid,
) -> Result<Option<&Element>, ScriptObjectStreamError> {
    let actual =
        value::semantic_type_id(element).map_err(|source| ScriptObjectStreamError::Field {
            field: "BaseClass1",
            source,
        })?;
    if actual == expected {
        return Ok(Some(element));
    }
    for child in element.children().iter().filter(|child| {
        child
            .field()
            .is_some_and(|field| field.as_str() == "BaseClass1")
    }) {
        if let Some(base) = base_class_of_semantic_type(child, expected)? {
            return Ok(Some(base));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use az_objectstream::{Element, types};

    use super::*;

    #[test]
    fn reads_script_component_fields() {
        let element = Element::new(SCRIPT_COMPONENT_TYPE_ID).with_children([
            Element::new(az_core::component::COMPONENT_TYPE_ID)
                .with_field("BaseClass1")
                .with_children([Element::new(types::AZ_U64)
                    .with_field("ID")
                    .with_data(0xCAFE_u64.to_be_bytes())]),
            Element::new(Uuid::nil())
                .with_field("BaseClass2")
                .with_children([Element::new(types::BOOL)
                    .with_field("m_isNetSyncEnabled")
                    .with_data([1])]),
            Element::new(types::UNSIGNED_INT)
                .with_field("ContextID")
                .with_data(7_u32.to_be_bytes()),
            script_property_group("properties", "Root"),
            Element::new(types::ASSET)
                .with_field("Script")
                .with_data(b"hint={scripts/props/lever.lua}".as_slice()),
            Element::new(types::BOOL)
                .with_field("IsRunOnServer")
                .with_data([0]),
            Element::new(types::BOOL)
                .with_field("IsRunOnClient")
                .with_data([1]),
        ]);

        let component = read_script_component(&element).expect("script component");

        assert_eq!(component.az_component.id, ComponentId::new(0xCAFE));
        assert_eq!(component.context_id, 7);
        assert_eq!(component.properties.name, "Root");
        assert_eq!(component.script.as_deref(), Some("scripts/props/lever.lua"));
        assert!(!component.run_on_server);
        assert!(component.run_on_client);
        assert!(component.net_bindable.is_net_sync_enabled);
    }

    #[test]
    fn rejects_non_script_component() {
        let actual = Uuid::from_u128(0xCAFE);
        let element = Element::new(actual);

        assert!(matches!(
            read_script_component(&element),
            Err(ScriptObjectStreamError::UnexpectedType { actual: id, .. }) if id == actual
        ));
    }

    #[test]
    fn reads_script_property_key_pascal_case_fields() {
        let element = key_element("Id", "Name", 0xCAFE, "Enabled");

        let key = read_script_property_key(&element).expect("key");

        assert_eq!(key.id, 0xCAFE);
        assert_eq!(key.name, "Enabled");
    }

    #[test]
    fn reads_script_property_key_camel_case_fields() {
        let element = key_element("id", "name", 0xBEEF, "Delay");

        let key = read_script_property_key(&element).expect("key");

        assert_eq!(key.id, 0xBEEF);
        assert_eq!(key.name, "Delay");
    }

    #[test]
    fn skips_blank_script_property_key_name() {
        let element = key_element("Id", "Name", 0xCAFE, "   ");

        let key = read_script_property_key(&element).expect("key");

        assert_eq!(key.id, 0xCAFE);
        assert!(key.name.is_empty());
    }

    #[test]
    fn reads_script_property_group_tree() {
        let element = Element::new(SCRIPT_PROPERTY_GROUP_TYPE_ID).with_children([
            Element::new(types::AZSTD_STRING)
                .with_field("Name")
                .with_data(b"Root".as_slice()),
            Element::new(types::AZSTD_STRING)
                .with_field("Id")
                .with_data(b"root-id".as_slice()),
            Element::new(types::AZSTD_VECTOR)
                .with_field("Properties")
                .with_children([
                    script_property(
                        SCRIPT_PROPERTY_BOOLEAN_TYPE_ID,
                        "Enabled",
                        vec![Element::new(types::BOOL).with_field("value").with_data([1])],
                    ),
                    script_property(
                        SCRIPT_PROPERTY_STRING_ARRAY_TYPE_ID,
                        "Tags",
                        vec![
                            Element::new(types::AZSTD_VECTOR)
                                .with_field("values")
                                .with_children([
                                    Element::new(types::AZSTD_STRING)
                                        .with_data(b"interact".as_slice()),
                                    Element::new(types::AZSTD_STRING)
                                        .with_data(b"quest".as_slice()),
                                ]),
                        ],
                    ),
                ]),
            Element::new(types::AZSTD_VECTOR)
                .with_field("Groups")
                .with_children([Element::new(SCRIPT_PROPERTY_GROUP_TYPE_ID).with_children([
                    Element::new(types::AZSTD_STRING)
                        .with_field("Name")
                        .with_data(b"Child".as_slice()),
                ])]),
        ]);

        let group = read_script_property_group(&element).expect("group");

        assert_eq!(group.name, "Root");
        assert_eq!(group.id.as_deref(), Some("root-id"));
        assert_eq!(group.properties.len(), 2);
        assert_eq!(group.properties[0].key.name, "Enabled");
        assert_eq!(
            group.properties[0].value,
            ScriptPropertyValue::Boolean(true)
        );
        assert_eq!(
            group.properties[1].value,
            ScriptPropertyValue::StringArray(vec!["interact".to_string(), "quest".to_string()])
        );
        assert_eq!(group.groups.len(), 1);
        assert_eq!(group.groups[0].name, "Child");
    }

    #[test]
    fn reads_script_asset_property_hint() {
        let element = script_property(
            SCRIPT_PROPERTY_ASSET_TYPE_ID,
            "Texture",
            vec![
                Element::new(types::ASSET)
                    .with_field("value")
                    .with_data(b"hint={textures/foo.dds}".as_slice()),
            ],
        );

        let property = read_script_property(&element).expect("asset property");

        assert_eq!(property.key.name, "Texture");
        assert_eq!(
            property.value,
            ScriptPropertyValue::Asset(Some("textures/foo.dds".to_string()))
        );
    }

    #[test]
    fn errors_when_script_property_type_is_unknown() {
        let actual = Uuid::from_u128(0xCAFE);
        let element = Element::new(actual);

        assert!(matches!(
            read_script_property(&element),
            Err(ScriptObjectStreamError::ScriptPropertyType(id)) if id == actual
        ));
    }

    #[test]
    fn reads_script_context_id_from_unsigned_int() {
        let element = Element::new(types::UNSIGNED_INT).with_data(0xCAFE_u32.to_be_bytes());

        let context_id = read_script_context_id(&element).expect("context id");

        assert_eq!(context_id, 0xCAFE);
    }

    #[test]
    fn reads_script_context_id_from_widened_u64() {
        let element = Element::new(types::AZ_U64).with_data(u64::from(u32::MAX).to_be_bytes());

        let context_id = read_script_context_id(&element).expect("context id");

        assert_eq!(context_id, u32::MAX);
    }

    #[test]
    fn errors_when_script_context_id_exceeds_u32() {
        let value = u64::from(u32::MAX) + 1;
        let element = Element::new(types::AZ_U64).with_data(value.to_be_bytes());

        assert!(matches!(
            read_script_context_id(&element),
            Err(ScriptObjectStreamError::ContextIdOutOfRange(actual)) if actual == value
        ));
    }

    #[test]
    fn reads_script_number_scalar_from_double() {
        let element = Element::new(types::DOUBLE).with_data(12.5_f64.to_be_bytes());

        let value = read_script_number_scalar(&element).expect("number");

        // Bit-exact: the element carries this f64 verbatim, so the read must
        // reproduce every bit rather than land within a tolerance.
        assert_eq!(value.to_bits(), 12.5_f64.to_bits());
    }

    #[test]
    fn reads_script_number_scalar_from_float() {
        let element = Element::new(types::FLOAT).with_data(1.25_f32.to_be_bytes());

        let value = read_script_number_scalar(&element).expect("number");

        // Bit-exact: 1.25 is representable in both widths, so widening the
        // stored f32 to f64 must land on exactly this bit pattern.
        assert_eq!(value.to_bits(), 1.25_f64.to_bits());
    }

    #[test]
    fn reads_script_bool_scalar_values() {
        let true_element = Element::new(types::BOOL).with_data([1]);
        let false_element = Element::new(types::BOOL).with_data([0]);

        assert!(read_script_bool_scalar(&true_element).expect("true value"));
        assert!(!read_script_bool_scalar(&false_element).expect("false value"));
    }

    #[test]
    fn reads_script_string_scalar() {
        let element = Element::new(types::AZSTD_STRING).with_data(b" Enabled ".as_slice());

        let value = read_script_string_scalar(&element).expect("string");

        assert_eq!(value.as_deref(), Some("Enabled"));
    }

    #[test]
    fn skips_blank_script_string_scalar() {
        let element = Element::new(types::AZSTD_STRING).with_data(b"   ".as_slice());

        let value = read_script_string_scalar(&element).expect("string");

        assert_eq!(value, None);
    }

    #[test]
    fn reads_script_bool_vector_values() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            Element::new(types::BOOL).with_data([1]),
            Element::new(types::AZSTD_STRING).with_data(b"ignored".as_slice()),
            Element::new(types::BOOL).with_data([0]),
        ]);

        let values = read_script_bool_vector(&element).expect("bool values");

        assert_eq!(values, vec![true, false]);
    }

    #[test]
    fn reads_script_number_vector_values() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            Element::new(types::DOUBLE).with_data(1.5_f64.to_be_bytes()),
            Element::new(types::FLOAT).with_data(2.25_f32.to_be_bytes()),
        ]);

        let values = read_script_number_vector(&element).expect("number values");

        assert_eq!(values, vec![1.5, 2.25]);
    }

    #[test]
    fn rejects_non_numeric_script_number_vector_value() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            Element::new(types::DOUBLE).with_data(1.5_f64.to_be_bytes()),
            Element::new(types::BOOL).with_data([1]),
        ]);

        assert!(matches!(
            read_script_number_vector(&element),
            Err(ScriptObjectStreamError::Field {
                source: ObjectStreamValueError::UnexpectedType { .. },
                ..
            })
        ));
    }

    #[test]
    fn reads_script_string_vector_values() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            Element::new(types::AZSTD_STRING).with_data(b"Alpha".as_slice()),
            Element::new(types::BOOL).with_data([1]),
            Element::new(types::AZSTD_BASIC_STRING).with_data(b" Beta ".as_slice()),
            Element::new(types::AZSTD_STRING).with_data(b" ".as_slice()),
        ]);

        let values = read_script_string_vector(&element).expect("string values");

        assert_eq!(values, vec!["Alpha".to_string(), "Beta".to_string()]);
    }

    #[test]
    fn reads_script_entity_ref_from_entity_id_value() {
        let element = Element::new(types::ENTITY_ID).with_children([Element::new(types::AZ_U64)
            .with_field("ID")
            .with_data(0xBEEF_u64.to_be_bytes())]);

        let id = read_script_entity_ref(&element).expect("entity ref");

        assert_eq!(id, 0xBEEF);
    }

    #[test]
    fn reads_script_entity_ref_from_direct_script_value() {
        let element =
            Element::new(types::AZSTD_VECTOR).with_children([Element::new(types::AZ_U64)
                .with_field("id")
                .with_data(0xCAFE_u64.to_be_bytes())]);

        let id = read_script_entity_ref(&element).expect("entity ref");

        assert_eq!(id, 0xCAFE);
    }

    #[test]
    fn errors_when_script_entity_ref_id_is_missing() {
        let element = Element::new(types::ENTITY_ID);

        assert!(matches!(
            read_script_entity_ref(&element),
            Err(ScriptObjectStreamError::Field {
                source: ObjectStreamValueError::MissingField { .. },
                ..
            })
        ));
    }

    fn key_element(
        id_field: &'static str,
        name_field: &'static str,
        id: u64,
        name: &str,
    ) -> Element {
        Element::new(types::AZSTD_VECTOR).with_children([
            Element::new(types::AZ_U64)
                .with_field(id_field)
                .with_data(id.to_be_bytes()),
            Element::new(types::AZSTD_STRING)
                .with_field(name_field)
                .with_data(name.as_bytes()),
        ])
    }

    fn script_property(id: Uuid, name: &str, children: Vec<Element>) -> Element {
        let mut property_children = vec![key_base(name)];
        property_children.extend(children);
        Element::new(id).with_children(property_children)
    }

    fn script_property_group(field: &str, name: &str) -> Element {
        Element::new(SCRIPT_PROPERTY_GROUP_TYPE_ID)
            .with_field(field)
            .with_children([Element::new(types::AZSTD_STRING)
                .with_field("Name")
                .with_data(name.as_bytes())])
    }

    fn key_base(name: &str) -> Element {
        Element::new(SCRIPT_PROPERTY_TYPE_ID)
            .with_field("BaseClass1")
            .with_children([
                Element::new(types::AZ_U64)
                    .with_field("Id")
                    .with_data(0xCAFE_u64.to_be_bytes()),
                Element::new(types::AZSTD_STRING)
                    .with_field("Name")
                    .with_data(name.as_bytes()),
            ])
    }
}
