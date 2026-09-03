//! Native AZ entity reference value types.

use bevy_reflect::Reflect;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::EntityId;

/// Reflected `AZ::Entity::LocalEntityRef`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, az_derive::AzRtti)]
#[az_rtti(
    name = "LocalEntityRef",
    "EA5FE48A-66F7-42D7-EE11-12391C964778",
    register
)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LocalEntityRef {
    #[cfg_attr(feature = "serde", serde(rename = "EntityId", default))]
    pub entity_id: EntityId,
}

impl LocalEntityRef {
    #[must_use]
    pub const fn new(entity_id: EntityId) -> Self {
        Self { entity_id }
    }

    #[must_use]
    pub const fn invalid() -> Self {
        Self::new(EntityId::INVALID)
    }

    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.entity_id.is_invalid()
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.is_invalid()
    }
}

impl From<EntityId> for LocalEntityRef {
    fn from(entity_id: EntityId) -> Self {
        Self::new(entity_id)
    }
}

pub fn register_reflected_types(registry: &mut bevy_reflect::TypeRegistry) {
    registry.register::<LocalEntityRef>();
    registry.register_type_data::<LocalEntityRef, crate::ReflectAzTypeInfo>();
    registry.register_type_data::<LocalEntityRef, crate::ReflectAzRtti>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AzTypeInfo;

    #[test]
    fn local_entity_ref_matches_native_identity_and_invalid_default() {
        assert_eq!(
            LocalEntityRef::TYPE_ID,
            uuid::uuid!("EA5FE48A-66F7-42D7-EE11-12391C964778")
        );
        assert!(LocalEntityRef::default().is_invalid());
        assert!(LocalEntityRef::new(EntityId::new(7)).is_valid());
    }
}
