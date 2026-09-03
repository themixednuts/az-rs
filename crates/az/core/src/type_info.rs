//! Static AZ type identity (`AZ_TYPE_INFO`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, LinkedList};
use std::sync::Arc;

use uuid::Uuid;

/// Static AZ type identity (`AZ_TYPE_INFO`).
///
/// Provides the UUID and name used by `SerializeContext`, `ObjectStream`, and
/// other reflection surfaces. This is the bottom layer: it does not add RTTI
/// virtuals ([`crate::rtti::AzRtti`]) or entity-component descriptor wiring
/// ([`crate::component::AzComponent`]).
pub trait AzTypeInfo {
    const NAME: &'static str;
    const TYPE_ID: Uuid;
}

macro_rules! az_type_info {
    ($ty:ty, $name:literal, $type_id:expr) => {
        impl AzTypeInfo for $ty {
            const NAME: &'static str = $name;
            const TYPE_ID: Uuid = $type_id;
        }
    };
}

az_type_info!(bool, "bool", crate::uuid::type_ids::BOOL);
az_type_info!(i8, "s8", crate::uuid::type_ids::S8);
az_type_info!(i16, "s16", crate::uuid::type_ids::SHORT);
az_type_info!(i32, "s32", crate::uuid::type_ids::INT);
az_type_info!(i64, "s64", crate::uuid::type_ids::S64);
az_type_info!(u8, "u8", crate::uuid::type_ids::U8);
az_type_info!(u16, "u16", crate::uuid::type_ids::U16);
az_type_info!(u32, "u32", crate::uuid::type_ids::U32);
az_type_info!(u64, "u64", crate::uuid::type_ids::U64);
az_type_info!(f32, "float", crate::uuid::type_ids::FLOAT);
az_type_info!(f64, "double", crate::uuid::type_ids::DOUBLE);
az_type_info!((), "void", crate::uuid::type_ids::VOID);
az_type_info!(crate::crc::Crc32, "AZ::Crc32", crate::uuid::type_ids::CRC32);
az_type_info!(String, "AZStd::string", crate::uuid::type_ids::AZSTD_STRING);
az_type_info!(Uuid, "AZ::Uuid", crate::uuid::type_ids::AZ_UUID);
az_type_info!(glam::Vec2, "AZ::Vector2", crate::uuid::type_ids::VECTOR2);
az_type_info!(glam::Vec3, "AZ::Vector3", crate::uuid::type_ids::VECTOR3);
az_type_info!(glam::Vec4, "AZ::Vector4", crate::uuid::type_ids::VECTOR4);
az_type_info!(
    bevy_color::LinearRgba,
    "AZ::Color",
    crate::uuid::type_ids::COLOR
);
az_type_info!(
    glam::Quat,
    "AZ::Quaternion",
    crate::uuid::type_ids::QUATERNION
);

impl<T: AzTypeInfo> AzTypeInfo for Vec<T> {
    const NAME: &'static str = "AZStd::vector";
    const TYPE_ID: Uuid = crate::uuid::azstd_vector(T::TYPE_ID);
}

impl<T: AzTypeInfo> AzTypeInfo for LinkedList<T> {
    const NAME: &'static str = "AZStd::list";
    const TYPE_ID: Uuid = crate::uuid::azstd_list(T::TYPE_ID);
}

// The folded `TypeId` mirrors the default-hasher `AZStd::unordered_set`; a
// generic hasher parameter would not change it, so the default-hasher impl is
// intentional.
#[allow(clippy::implicit_hasher)]
impl<T: AzTypeInfo> AzTypeInfo for HashSet<T> {
    const NAME: &'static str = "AZStd::unordered_set";
    const TYPE_ID: Uuid = crate::uuid::azstd_unordered_set(T::TYPE_ID);
}

impl<T: AzTypeInfo> AzTypeInfo for BTreeSet<T> {
    const NAME: &'static str = "AZStd::set";
    const TYPE_ID: Uuid = crate::uuid::azstd_set(T::TYPE_ID);
}

// Default-hasher impl is intentional; see the `HashSet` note above.
#[allow(clippy::implicit_hasher)]
impl<K: AzTypeInfo, V: AzTypeInfo> AzTypeInfo for HashMap<K, V> {
    const NAME: &'static str = "AZStd::unordered_map";
    const TYPE_ID: Uuid = crate::uuid::azstd_unordered_map(K::TYPE_ID, V::TYPE_ID);
}

impl<K: AzTypeInfo, V: AzTypeInfo> AzTypeInfo for BTreeMap<K, V> {
    const NAME: &'static str = "AZStd::map";
    const TYPE_ID: Uuid = crate::uuid::azstd_map(K::TYPE_ID, V::TYPE_ID);
}

impl<T: AzTypeInfo> AzTypeInfo for Box<T> {
    const NAME: &'static str = "AZStd::unique_ptr";
    const TYPE_ID: Uuid = crate::uuid::azstd_unique_ptr(T::TYPE_ID);
}

impl<T: AzTypeInfo> AzTypeInfo for Arc<T> {
    const NAME: &'static str = "AZStd::shared_ptr";
    const TYPE_ID: Uuid = crate::uuid::azstd_shared_ptr(T::TYPE_ID);
}

impl<T: AzTypeInfo> AzTypeInfo for Option<T> {
    const NAME: &'static str = "AZStd::optional";
    const TYPE_ID: Uuid = crate::uuid::azstd_optional(T::TYPE_ID);
}

impl<T: AzTypeInfo, const N: usize> AzTypeInfo for [T; N] {
    const NAME: &'static str = "AZStd::array";
    const TYPE_ID: Uuid = crate::uuid::azstd_array(T::TYPE_ID, N);
}

/// Native AZ type identity as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeInfo {
    pub name: &'static str,
    pub type_id: Uuid,
}

impl TypeInfo {
    #[inline]
    #[must_use]
    pub const fn new(name: &'static str, type_id: Uuid) -> Self {
        Self { name, type_id }
    }

    #[inline]
    #[must_use]
    pub const fn of<T: AzTypeInfo>() -> Self {
        Self::new(T::NAME, T::TYPE_ID)
    }
}

#[inline]
#[must_use]
pub const fn type_name<T: AzTypeInfo>() -> &'static str {
    T::NAME
}

#[inline]
#[must_use]
pub const fn type_id<T: AzTypeInfo>() -> Uuid {
    T::TYPE_ID
}

#[inline]
#[must_use]
pub const fn type_info<T: AzTypeInfo>() -> TypeInfo {
    TypeInfo::of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn example_type_id() -> Uuid {
        Uuid::from_u128(0x11111111_1111_1111_1111_111111111111)
    }

    #[derive(az_derive::AzTypeInfo)]
    #[az_type_info(example_type_id())]
    struct ExampleConstFunctionTypeId;

    #[test]
    fn az_type_info_derive_accepts_const_function_type_id() {
        assert_eq!(
            <ExampleConstFunctionTypeId as AzTypeInfo>::TYPE_ID,
            example_type_id()
        );
        assert_eq!(type_id::<ExampleConstFunctionTypeId>(), example_type_id());
        assert_eq!(
            type_name::<ExampleConstFunctionTypeId>(),
            "ExampleConstFunctionTypeId"
        );
        assert_eq!(
            type_info::<ExampleConstFunctionTypeId>(),
            TypeInfo::new("ExampleConstFunctionTypeId", example_type_id())
        );
    }

    #[test]
    fn std_collections_map_to_azstd_type_ids() {
        assert_eq!(
            <LinkedList<u8> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_list(crate::uuid::type_ids::U8)
        );
        assert_eq!(
            <HashSet<u32> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_unordered_set(crate::uuid::type_ids::U32)
        );
        assert_eq!(
            <BTreeSet<u32> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_set(crate::uuid::type_ids::U32)
        );
        assert_eq!(
            <HashMap<String, u32> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_unordered_map(
                crate::uuid::type_ids::AZSTD_STRING,
                crate::uuid::type_ids::U32
            )
        );
        assert_eq!(
            <BTreeMap<String, u32> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_map(
                crate::uuid::type_ids::AZSTD_STRING,
                crate::uuid::type_ids::U32
            )
        );
    }

    #[test]
    fn std_smart_pointers_map_to_azstd_type_ids() {
        assert_eq!(
            <Box<u8> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_unique_ptr(crate::uuid::type_ids::U8)
        );
        assert_eq!(
            <Arc<String> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_shared_ptr(crate::uuid::type_ids::AZSTD_STRING)
        );
    }
}
