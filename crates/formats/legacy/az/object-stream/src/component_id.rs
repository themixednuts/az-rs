//! AZ `ComponentId` / `vector<ComponentId>` `ObjectStream` value helpers.

use az_core::ComponentId;

use crate::value::{
    ObjectStreamValueError, field_name, read_u64_scalar, require_container_shape, semantic_type_id,
};
use crate::{Element, types};

/// Decode a reflected `AZ::ComponentId` (`typedef AZ::u64`).
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedType`] if the element's
/// semantic type is not [`types::AZ_U64`]. Otherwise returns any error
/// [`read_u64_scalar`] returns — [`ObjectStreamValueError::MissingSerializer`]
/// when no unsigned serializer is proven,
/// [`ObjectStreamValueError::InvalidValue`] when the proven serializer is not
/// one of the accepted unsigned widths,
/// [`ObjectStreamValueError::MissingData`] when the element carries no value
/// bytes, and [`ObjectStreamValueError::InvalidLength`] when the payload width
/// disagrees with that serializer.
pub fn read_component_id(element: &Element) -> Result<ComponentId, ObjectStreamValueError> {
    let actual = semantic_type_id(element)?;
    if actual != types::AZ_U64 {
        return Err(ObjectStreamValueError::UnexpectedType {
            field: field_name(element),
            expected: "AZ::ComponentId or AZ::u64",
            actual,
        });
    }

    read_u64_scalar(element).map(ComponentId::new)
}

/// Decode a reflected `AZStd::vector<AZ::u64>` component-id list.
///
/// Payloads use the folded vector type UUID (`COMPONENT_ID_VECTOR`) with
/// `AZ::u64` children. Generic `AZStd::vector` wrappers with `AZ::u64`
/// children are also accepted.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedType`] if the element is neither
/// [`types::COMPONENT_ID_VECTOR`] nor [`types::AZSTD_VECTOR`], or if any child
/// is not [`types::AZ_U64`], and
/// [`ObjectStreamValueError::UnexpectedContainerShape`] if the element's
/// reflected container family is not a sequence. Per-child errors from
/// [`read_component_id`] propagate unchanged.
pub fn read_component_id_vector(
    element: &Element,
) -> Result<Vec<ComponentId>, ObjectStreamValueError> {
    let actual = semantic_type_id(element)?;
    if !matches!(actual, types::COMPONENT_ID_VECTOR | types::AZSTD_VECTOR) {
        return Err(ObjectStreamValueError::UnexpectedType {
            field: field_name(element),
            expected: "AZStd::vector<AZ::u64> component-id list",
            actual,
        });
    }
    require_container_shape(
        element,
        crate::context::ContainerShape::Sequence,
        "captured sequence IDataContainer",
    )?;

    element
        .children()
        .iter()
        .map(|child| {
            let actual = semantic_type_id(child)?;
            if actual != types::AZ_U64 {
                return Err(ObjectStreamValueError::UnexpectedType {
                    field: field_name(child),
                    expected: "AZ::u64 component-id vector element",
                    actual,
                });
            }
            read_component_id(child)
        })
        .collect()
}

impl TryFrom<&Element> for ComponentId {
    type Error = ObjectStreamValueError;

    fn try_from(element: &Element) -> Result<Self, Self::Error> {
        read_component_id(element)
    }
}

impl TryFrom<&Element> for Vec<ComponentId> {
    type Error = ObjectStreamValueError;

    fn try_from(element: &Element) -> Result<Self, Self::Error> {
        read_component_id_vector(element)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_core::type_info::AzTypeInfo;
    use uuid::Uuid;

    #[test]
    fn component_id_vector_type_matches_azcore_folded_uuid() {
        assert_eq!(
            types::COMPONENT_ID_VECTOR,
            <Vec<ComponentId> as AzTypeInfo>::TYPE_ID
        );
    }

    #[test]
    fn component_id_uses_az_u64_type_uuid() {
        assert_eq!(ComponentId::TYPE_ID, az_core::uuid::type_ids::U64);
        assert_eq!(types::AZ_U64, ComponentId::TYPE_ID);
    }

    #[test]
    fn decodes_component_id_scalar() {
        let element = leaf("id", types::AZ_U64, 42_u64.to_be_bytes());

        assert_eq!(read_component_id(&element).unwrap(), ComponentId::new(42));
        assert_eq!(
            ComponentId::try_from(&element).unwrap(),
            ComponentId::new(42)
        );
    }

    #[test]
    fn decodes_folded_component_id_vector() {
        let element = Element::new(types::COMPONENT_ID_VECTOR)
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([
                leaf("Element", types::AZ_U64, 1_u64.to_be_bytes()),
                leaf("Element", types::AZ_U64, 2_u64.to_be_bytes()),
            ]);

        assert_eq!(
            read_component_id_vector(&element).unwrap(),
            vec![ComponentId::new(1), ComponentId::new(2)]
        );
        assert_eq!(
            Vec::<ComponentId>::try_from(&element).unwrap(),
            vec![ComponentId::new(1), ComponentId::new(2)]
        );
    }

    #[test]
    fn rejects_wrong_component_id_vector_child() {
        let element = Element::new(types::COMPONENT_ID_VECTOR)
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([leaf("Element", types::UNSIGNED_INT, 9_u32.to_be_bytes())]);

        assert!(matches!(
            read_component_id_vector(&element),
            Err(ObjectStreamValueError::UnexpectedType { .. })
        ));
    }

    #[test]
    fn decodes_generic_vector_of_component_ids() {
        let element = Element::new(types::AZSTD_VECTOR)
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([
                leaf("Element", types::AZ_U64, 7_u64.to_be_bytes()),
                leaf("Element", types::AZ_U64, 8_u64.to_be_bytes()),
            ]);

        assert_eq!(
            read_component_id_vector(&element).unwrap(),
            vec![ComponentId::new(7), ComponentId::new(8)]
        );
    }

    #[test]
    fn rejects_wrong_vector_type() {
        let element = Element::new(types::ENTITY_ID).with_test_class();

        assert!(matches!(
            read_component_id_vector(&element).unwrap_err(),
            ObjectStreamValueError::UnexpectedType { .. }
        ));
    }

    fn leaf(field: &str, id: Uuid, data: impl Into<Vec<u8>>) -> Element {
        let element = Element::new(id).with_field(field).with_data(data);
        if let Some(kind) = crate::codec::builtin_serializer_kind(id) {
            element.with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(kind, 0))
        } else {
            element
        }
    }
}
