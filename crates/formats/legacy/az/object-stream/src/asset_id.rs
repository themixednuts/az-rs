//! AZ `Data::AssetId` `ObjectStream` value helpers.

use az_asset::AssetId;

use crate::value::{
    ObjectStreamValueError, field_name, read_u32, read_uuid, require_container_shape,
    semantic_type_id,
};
use crate::{Element, types};

/// Decode an `AZ::Data::AssetId` `ObjectStream` object.
///
/// The reflected shape is an `AssetId` element with `guid` and `subId`
/// children. Child lookup is non-consuming, so XML/binary dumps with
/// equivalent reflected fields decode the same even if child order changes.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedType`] if `element` resolves to
/// neither [`types::ASSET_ID`] nor [`types::ASSET`],
/// [`ObjectStreamValueError::UnknownField`] for any child that is neither
/// `guid` nor `subId`, [`ObjectStreamValueError::MissingField`] if either of
/// those two children is absent, and
/// [`ObjectStreamValueError::InvalidValue`] if either appears more than once.
/// Errors from [`read_uuid`] on `guid` and [`read_u32`] on `subId` propagate
/// unchanged.
pub fn read_asset_id(element: &Element) -> Result<AssetId, ObjectStreamValueError> {
    if !is_asset_id_type(element) {
        return Err(ObjectStreamValueError::UnexpectedType {
            field: field_name(element),
            expected: "AZ::Data::AssetId",
            actual: semantic_type_id(element).unwrap_or_else(|_| *element.raw_type_id()),
        });
    }

    let guid_fields = element
        .children()
        .iter()
        .filter(|child| field_matches(child, "guid"))
        .collect::<Vec<_>>();
    let sub_id_fields = element
        .children()
        .iter()
        .filter(|child| field_matches(child, "subId"))
        .collect::<Vec<_>>();
    if let Some(child) = element
        .children()
        .iter()
        .find(|child| !field_matches(child, "guid") && !field_matches(child, "subId"))
    {
        return Err(ObjectStreamValueError::UnknownField {
            field: field_name(child),
        });
    }
    let guid = guid_fields
        .first()
        .copied()
        .ok_or_else(|| ObjectStreamValueError::MissingField {
            field: "guid".to_owned(),
        })
        .and_then(read_uuid)?;
    let sub_id = sub_id_fields
        .first()
        .copied()
        .ok_or_else(|| ObjectStreamValueError::MissingField {
            field: "subId".to_owned(),
        })
        .and_then(read_u32)?;
    if guid_fields.len() != 1 || sub_id_fields.len() != 1 || element.children().len() != 2 {
        return Err(ObjectStreamValueError::InvalidValue {
            field: field_name(element),
            expected: "exactly one guid and one subId field",
        });
    }
    Ok(AssetId::new(guid, sub_id))
}

fn field_matches(element: &Element, name: &str) -> bool {
    element.field().is_some_and(|field| field.as_str() == name)
        || element.name_crc() == Some(crate::field_name_crc(name))
}

/// Decode an `AZ::Data::AssetId` and treat the nil sentinel as absent.
///
/// # Errors
///
/// Returns any error [`read_asset_id`] returns; the nil check itself cannot
/// fail.
pub fn read_non_nil_asset_id(element: &Element) -> Result<Option<AssetId>, ObjectStreamValueError> {
    read_asset_id(element).map(|asset_id| (!asset_id.is_nil()).then_some(asset_id))
}

/// Decode direct `AZ::Data::AssetId` children from a reflected vector.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedContainerShape`] when `element`
/// carries a reflected container shape that is not
/// [`ContainerShape::Sequence`](crate::context::ContainerShape::Sequence);
/// elements with no captured shape at all are accepted. Errors from
/// [`read_asset_id`] on any `AssetId`-typed child propagate unchanged —
/// children of other types are skipped rather than rejected.
pub fn read_asset_id_vector(element: &Element) -> Result<Vec<AssetId>, ObjectStreamValueError> {
    // Soft sequence proof: reject only proven non-sequence containers. Raw
    // specialized Vec fixture UUIDs still decode by filtering AssetId children.
    if let Some(shape) = element.container_shape()
        && shape != crate::context::ContainerShape::Sequence
    {
        require_container_shape(
            element,
            crate::context::ContainerShape::Sequence,
            "captured sequence IDataContainer",
        )?;
    }
    element
        .children()
        .iter()
        .filter(|child| is_asset_id_type(child))
        .map(read_asset_id)
        .collect()
}

#[inline]
fn is_asset_id_type(element: &Element) -> bool {
    // Unresolved elements keep the wire UUID as their typed identity.
    let id = element
        .resolved_type_id()
        .copied()
        .unwrap_or_else(|| *element.raw_type_id());
    matches!(id, types::ASSET_ID | types::ASSET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn decodes_asset_id_fields() {
        let guid = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);
        let element = asset_id_element(guid, 7);

        assert_eq!(read_asset_id(&element).unwrap(), AssetId::new(guid, 7));
    }

    #[test]
    fn decodes_runtime_asset_id_type() {
        let guid = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);
        let element = class(types::ASSET_ID).with_children([
            leaf("guid", types::AZ_UUID, guid.as_bytes().to_vec()),
            leaf("subId", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
        ]);

        assert_eq!(read_asset_id(&element).unwrap(), AssetId::new(guid, 7));
    }

    #[test]
    fn decodes_asset_id_fields_without_order_dependency() {
        let guid = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);
        let element = class(types::ASSET_ID).with_children([
            leaf("subId", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
            leaf("guid", types::AZ_UUID, guid.as_bytes().to_vec()),
        ]);

        assert_eq!(read_asset_id(&element).unwrap(), AssetId::new(guid, 7));
    }

    #[test]
    fn nil_asset_id_is_optional_absent() {
        let element = asset_id_element(Uuid::nil(), 0);

        assert_eq!(read_non_nil_asset_id(&element).unwrap(), None);
    }

    #[test]
    fn reads_asset_id_vector_children() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let element = Element::new(types::AZSTD_VECTOR)
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([
                asset_id_element(first, 0),
                class(types::ASSET_ID).with_children([
                    leaf("guid", types::AZ_UUID, second.as_bytes().to_vec()),
                    leaf("subId", types::UNSIGNED_INT, 3_u32.to_be_bytes()),
                ]),
            ]);

        assert_eq!(
            read_asset_id_vector(&element).unwrap(),
            vec![AssetId::new(first, 0), AssetId::new(second, 3)]
        );
    }

    #[test]
    fn asset_id_vector_ignores_non_asset_children() {
        let element = Element::new(types::AZSTD_VECTOR)
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([leaf("notAsset", types::UNSIGNED_INT, 12_u32.to_be_bytes())]);

        assert_eq!(read_asset_id_vector(&element).unwrap(), Vec::new());
    }

    #[test]
    fn rejects_wrong_type_and_missing_fields() {
        assert!(matches!(
            read_asset_id(&leaf("Asset", types::UNSIGNED_INT, 7_u32.to_be_bytes())).unwrap_err(),
            ObjectStreamValueError::UnexpectedType { .. }
        ));
        assert!(matches!(
            read_asset_id(&class(types::ASSET_ID)).unwrap_err(),
            ObjectStreamValueError::MissingField { field } if field == "guid"
        ));
    }

    fn asset_id_element(guid: Uuid, sub_id: u32) -> Element {
        class(types::ASSET_ID).with_children([
            leaf("guid", types::AZ_UUID, guid.as_bytes().to_vec()),
            leaf("subId", types::UNSIGNED_INT, sub_id.to_be_bytes()),
        ])
    }

    fn class(id: Uuid) -> Element {
        Element::new(id).with_test_class()
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
