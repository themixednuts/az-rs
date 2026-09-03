//! `AZ::DataPatch` address-path types.
//!
//! Lumberyard/O3DE expose `DataPatch::AddressType` as a public alias over the
//! internal address path used by slice data patches. Legacy serialized assets
//! still use the `DataPatch` type id, so that wire shape is modeled separately
//! as [`LegacyDataPatch`].

use std::collections::HashMap;

use uuid::Uuid;

/// `AZ::DataPatchInternal::AddressTypeElement::ElementType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AddressElementType {
    Class,
    Index,
    #[default]
    None,
}

/// One segment in a `DataPatch::AddressType` path.
#[derive(az_derive::AzRtti, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[az_rtti(
    name = "DataPatchInternal::AddressTypeElement",
    "6882E51A-6A3E-4CE8-AF80-0989F0AE61FF",
    register
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AddressTypeElement {
    pub address_class_type_id: Uuid,
    pub path_element: String,
    pub address_element: u64,
    pub address_class_version: u32,
    pub element_type: AddressElementType,
    pub is_valid: bool,
}

impl Default for AddressTypeElement {
    #[inline]
    fn default() -> Self {
        Self {
            address_class_type_id: Uuid::nil(),
            path_element: String::new(),
            address_element: 0,
            address_class_version: 0,
            element_type: AddressElementType::None,
            is_valid: true,
        }
    }
}

impl AddressTypeElement {
    #[inline]
    #[must_use]
    pub fn new(address_element: u64) -> Self {
        Self {
            address_element,
            is_valid: true,
            ..Self::default()
        }
    }

    #[inline]
    #[must_use]
    pub fn with_class_path(
        address_class_type_id: Uuid,
        path_element: impl Into<String>,
        address_element: u64,
        address_class_version: u32,
    ) -> Self {
        Self {
            address_class_type_id,
            path_element: path_element.into(),
            address_element,
            address_class_version,
            element_type: AddressElementType::Class,
            is_valid: true,
        }
    }
}

/// `DataPatch::AddressType`, the address path used as the key in patch maps.
#[derive(az_derive::AzRtti, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[az_rtti(
    name = "DataPatch::AddressType",
    "90752F2D-CBD3-4EE9-9CDD-447E797C8408",
    register
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AddressType {
    pub elements: Vec<AddressTypeElement>,
    pub is_valid: bool,
    pub is_legacy: bool,
}

impl Default for AddressType {
    #[inline]
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            is_valid: true,
            is_legacy: false,
        }
    }
}

impl AddressType {
    #[inline]
    #[must_use]
    pub fn new(elements: Vec<AddressTypeElement>) -> Self {
        let is_valid = elements.iter().all(|element| element.is_valid);
        Self {
            elements,
            is_valid,
            is_legacy: false,
        }
    }

    #[inline]
    #[must_use]
    pub fn legacy(elements: Vec<AddressTypeElement>) -> Self {
        let is_valid = elements.iter().all(|element| element.is_valid);
        Self {
            elements,
            is_valid,
            is_legacy: true,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_from_legacy_address(&self) -> bool {
        self.elements
            .iter()
            .all(|element| element.address_class_type_id.is_nil())
    }
}

impl From<Vec<AddressTypeElement>> for AddressType {
    #[inline]
    fn from(elements: Vec<AddressTypeElement>) -> Self {
        Self::new(elements)
    }
}

impl AsRef<[AddressTypeElement]> for AddressType {
    #[inline]
    fn as_ref(&self) -> &[AddressTypeElement] {
        &self.elements
    }
}

/// Legacy serialized `DataPatch` shape used by Lumberyard slices.
#[derive(az_derive::AzRtti, Debug, Clone, Default, PartialEq, Eq)]
#[az_rtti(name = "DataPatch", "3A8D5EC9-D70E-41CB-879C-DEF6A6D6ED03", register)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LegacyDataPatch {
    pub target_class_id: Uuid,
    pub patch: HashMap<AddressType, Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtti::AzTypeRegistration;
    use crate::{AzRtti, AzTypeInfo};

    #[test]
    fn address_type_uses_data_patch_alias_identity() {
        assert_eq!(AddressType::NAME, "DataPatch::AddressType");
        assert_eq!(
            AddressType::TYPE_ID,
            uuid::uuid!("90752F2D-CBD3-4EE9-9CDD-447E797C8408")
        );
        assert!(AddressType::BASE_TYPE_IDS.is_empty());
    }

    #[test]
    fn address_paths_default_to_valid() {
        let element = AddressTypeElement::default();
        assert!(element.is_valid);
        assert!(element.address_class_type_id.is_nil());

        let address = AddressType::default();
        assert!(address.is_valid);
        assert!(!address.is_legacy);
        assert!(address.is_from_legacy_address());
    }

    /// Legacy `DataPatch` streams name these two types, so a host that reads
    /// them has to be handed both. What `az-core` offers a host is
    /// [`crate::rtti::types`]; that it reaches a composed registry from there
    /// is the compose-seam test in `rtti`.
    #[test]
    fn data_patch_types_are_offered_to_a_host() {
        let types = crate::rtti::types();
        let named = |name: &str, type_id| {
            types.iter().any(|registration: &AzTypeRegistration| {
                registration.name == name && registration.native_type_id == type_id
            })
        };

        assert!(named("DataPatch::AddressType", AddressType::TYPE_ID));
        assert!(named("DataPatch", LegacyDataPatch::TYPE_ID));
    }
}
