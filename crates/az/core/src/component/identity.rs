//! AZ entity and component identity value types.
//!
//! The identity newtypes below opt into `#[reflect(PartialEq)]`, whose
//! `bevy_reflect` expansion is an `if let Some(value) = ..downcast_ref() {..}
//! else {..}` this crate does not author and cannot rewrite. The generated
//! `impl` lands at module scope, so the suppression has to live here rather
//! than on the individual structs.
#![expect(
    clippy::option_if_let_else,
    reason = "fires only inside the bevy_reflect reflect(PartialEq) expansion"
)]

use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

#[cfg(feature = "bevy")]
use bevy_ecs::reflect::ReflectComponent;

#[cfg(feature = "serde")]
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// `AZ::EntityId`.
///
/// Native `AzCore` declares only `AZ_TYPE_INFO(EntityId, "{...}")` here — not
/// `AZ_RTTI` or `AZ_COMPONENT`. Bevy `Component` / `Reflect` are for ECS and
/// editor reflection only.
///
/// Reflected **opaque** (mirroring bevy's own `Entity`): an `EntityId` is a
/// single 64-bit identity, not a structural aggregate. The default derived
/// tuple-struct reflection would represent it as a `DynamicTupleStruct` whose
/// `reflect_hash()` returns `None`; that panics `DynamicSet`/`DynamicMap`
/// hashing (`bevy_reflect` `set.rs`/`map.rs`) whenever an `EntityId` sits in a
/// `HashSet` element or `HashMap` key position. Opaque reflection plus
/// `reflect(Hash, PartialEq)` routes `reflect_hash`/`reflect_partial_eq` through
/// the concrete `Hash`/`Eq` impls, and `to_dynamic()` clones the concrete value
/// instead of building a non-hashable dynamic aggregate.
#[cfg_attr(feature = "bevy", derive(bevy_ecs::component::Component))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, az_derive::AzTypeInfo)]
#[reflect(opaque)]
#[reflect(Hash, PartialEq, Debug, Clone, Default)]
#[cfg_attr(feature = "bevy", reflect(Component))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[az_type_info("6383F1D3-BB27-4E6B-A49A-6409B2059EAA")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(transparent)]
pub struct EntityId(u64);

impl EntityId {
    pub const INVALID: Self = Self(0x0000_0000_FFFF_FFFF);

    #[inline]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
    }

    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.is_invalid()
    }
}

impl Default for EntityId {
    #[inline]
    fn default() -> Self {
        Self::INVALID
    }
}

/// `AZ::ComponentId`.
///
/// Native `AzCore` defines this as `typedef AZ::u64 ComponentId`. `ObjectStream`
/// therefore reports the underlying storage with the `AZ::u64` type UUID
/// (`D6597933-...`), not a distinct component-id class registration.
///
/// Reflected **opaque** for the same reason as [`EntityId`]: it is a single
/// 64-bit identity used as a `HashSet`/`HashMap` key in downstream data, so the
/// derived tuple-struct reflection (whose `reflect_hash()` is `None`) would
/// panic `DynamicSet`/`DynamicMap` hashing. Opaque reflection plus
/// `reflect(Hash, PartialEq)` keeps hashing/equality on the concrete impls.
#[cfg_attr(feature = "bevy", derive(bevy_ecs::component::Component))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect, az_derive::AzTypeInfo,
)]
#[reflect(opaque)]
#[reflect(Hash, PartialEq, Debug, Clone, Default)]
#[cfg_attr(feature = "bevy", reflect(Component))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[az_type_info("D6597933-47CD-4FC8-B911-63F3E2B0993A")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(transparent)]
pub struct ComponentId(u64);

impl ComponentId {
    pub const INVALID: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
    }

    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.is_invalid()
    }
}

impl Default for ComponentId {
    #[inline]
    fn default() -> Self {
        Self::INVALID
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_info::AzTypeInfo;
    use uuid::uuid;

    #[test]
    fn entity_id_matches_az_type_info() {
        let id = EntityId::new(42);

        assert_eq!(id.value(), 42);
        assert!(id.is_valid());
        assert!(EntityId::default().is_invalid());
        assert_eq!(
            <EntityId as AzTypeInfo>::TYPE_ID,
            crate::uuid::type_ids::ENTITY_ID
        );
    }

    #[test]
    fn component_id_uses_az_u64_type_uuid() {
        let id = ComponentId::new(42);

        assert_eq!(id.value(), 42);
        assert!(id.is_valid());
        assert!(ComponentId::default().is_invalid());
        assert_eq!(
            <ComponentId as AzTypeInfo>::TYPE_ID,
            crate::uuid::type_ids::U64
        );
    }

    #[test]
    fn component_id_vector_matches_az_type_info() {
        assert_eq!(
            <Vec<ComponentId> as AzTypeInfo>::TYPE_ID,
            uuid!("E7781CB0-E712-5E6A-948D-92FD4FE87F0D")
        );
        assert_eq!(<Vec<ComponentId> as AzTypeInfo>::NAME, "AZStd::vector");
    }
}
