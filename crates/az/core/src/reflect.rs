//! Bevy reflection metadata for AZ type identity.

#[path = "reflect/editor.rs"]
pub mod editor;
#[path = "reflect/validation.rs"]
pub mod validation;

pub use editor::{
    EditorFieldAttributes, EditorFieldConstraints, EditorNumericRange, EditorTypeAttributes,
    EditorWidget, register_editor_builtins,
};

use bevy_reflect::{FromType, Reflect};
use uuid::Uuid;

use crate::{
    rtti::{AzRtti, RttiInfo},
    type_info::{AzTypeInfo, TypeInfo},
};

/// Process-safe reflected value payload shared by authoring domains.
///
/// The type path identifies the Bevy-reflected value while `payload` carries
/// the representation selected by [`ReflectedValueEncoding`]. Typed RON is the
/// native authoring representation used by prefab and visual-graph edits.
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReflectedValueEnvelope {
    pub type_path: String,
    pub encoding: ReflectedValueEncoding,
    pub payload: Vec<u8>,
}

impl ReflectedValueEnvelope {
    /// Creates a typed-RON reflected value without coupling the carrier to a
    /// concrete serializer or type registry.
    #[must_use]
    pub fn typed_ron(type_path: impl Into<String>, payload: impl Into<String>) -> Self {
        Self {
            type_path: type_path.into(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: payload.into().into_bytes(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReflectedValueEncoding {
    BevyRemoteJson,
    TypedRon,
    CapnpData,
}

/// Low-level type-path contract for reflected authoring values.
///
/// Higher-level model crates use this trait without depending directly on the
/// reflection implementation that supplies the canonical path.
pub trait ReflectedTypePath {
    fn reflected_type_path() -> &'static str;
}

impl<T: bevy_reflect::TypePath> ReflectedTypePath for T {
    fn reflected_type_path() -> &'static str {
        T::type_path()
    }
}

/// Bevy type data carrying `AZ_TYPE_INFO` identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflectAzTypeInfo {
    info: TypeInfo,
}

impl ReflectAzTypeInfo {
    #[inline]
    #[must_use]
    pub const fn new(info: TypeInfo) -> Self {
        Self { info }
    }

    #[inline]
    #[must_use]
    pub const fn info(self) -> TypeInfo {
        self.info
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.info.name
    }

    #[inline]
    #[must_use]
    pub const fn type_id(self) -> Uuid {
        self.info.type_id
    }
}

impl<T: AzTypeInfo> FromType<T> for ReflectAzTypeInfo {
    fn from_type() -> Self {
        Self::new(TypeInfo::of::<T>())
    }
}

/// Bevy type data carrying AZ RTTI identity and direct base type IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflectAzRtti {
    info: RttiInfo,
}

impl ReflectAzRtti {
    #[inline]
    #[must_use]
    pub const fn new(info: RttiInfo) -> Self {
        Self { info }
    }

    #[inline]
    #[must_use]
    pub const fn info(self) -> RttiInfo {
        self.info
    }

    #[inline]
    #[must_use]
    pub const fn type_info(self) -> TypeInfo {
        self.info.type_info
    }

    #[inline]
    #[must_use]
    pub const fn base_type_ids(self) -> &'static [Uuid] {
        self.info.base_type_ids
    }

    #[inline]
    #[must_use]
    pub const fn is_type_of(self, type_id: Uuid) -> bool {
        self.info.is_type_of(type_id)
    }
}

impl<T: AzRtti> FromType<T> for ReflectAzRtti {
    fn from_type() -> Self {
        Self::new(RttiInfo::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::{Reflect, TypeRegistry};
    use uuid::uuid;

    use super::*;

    #[derive(Reflect, az_derive::AzRtti)]
    #[az_rtti("D8B76D2F-5F6C-4C47-A84B-27D42B5DCE4C")]
    struct ReflectedAzType;

    #[test]
    fn registers_az_identity_as_bevy_type_data() {
        let mut registry = TypeRegistry::default();
        registry.register::<ReflectedAzType>();
        registry.register_type_data::<ReflectedAzType, ReflectAzTypeInfo>();
        registry.register_type_data::<ReflectedAzType, ReflectAzRtti>();

        let registration = registry
            .get(std::any::TypeId::of::<ReflectedAzType>())
            .expect("reflected type registration");
        let type_info = registration
            .data::<ReflectAzTypeInfo>()
            .expect("AZ type info data");
        let rtti = registration.data::<ReflectAzRtti>().expect("AZ RTTI data");

        assert_eq!(type_info.name(), "ReflectedAzType");
        assert_eq!(
            type_info.type_id(),
            uuid!("D8B76D2F-5F6C-4C47-A84B-27D42B5DCE4C")
        );
        assert!(rtti.is_type_of(type_info.type_id()));
    }
}
