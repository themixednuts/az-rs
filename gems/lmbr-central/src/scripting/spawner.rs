use az_asset::UntypedAssetRef;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::SpawnerComponent` AZ component UUID.
pub const SPAWNER_COMPONENT_TYPE_ID: Uuid = uuid!("8022A627-DD76-5432-C75A-7234AC2798C1");

/// Deprecated serialized `LmbrCentral::SpawnerComponent` UUID.
pub const DEPRECATED_SPAWNER_COMPONENT_TYPE_ID: Uuid =
    uuid!("8022A627-FA7D-4516-A155-657A0927A3CA");

/// Spawns a dynamic slice at this entity's transform.
///
/// Source: `LmbrCentral::SpawnerComponent`.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Eq, Reflect, Prefab)]
#[az_rtti(
    name = "LmbrCentral::SpawnerComponent",
    SPAWNER_COMPONENT_TYPE_ID,
    register
)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
// The namespace keeps this prefab tag distinct from project-defined spawner tags.
#[prefab(tag = "azoth.lmbr_central.SpawnerComponent", version = 1)]
pub struct SpawnerComponent {
    pub slice_asset: UntypedAssetRef,
    pub spawn_on_activate: bool,
    pub destroy_on_deactivate: bool,
}

impl SpawnerComponent {
    #[inline]
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.slice_asset.hint()
    }
}

impl Default for SpawnerComponent {
    fn default() -> Self {
        Self {
            slice_asset: UntypedAssetRef::empty(),
            spawn_on_activate: false,
            destroy_on_deactivate: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_source() {
        let component = SpawnerComponent::default();

        assert!(component.slice_asset.is_empty());
        assert!(!component.spawn_on_activate);
        assert!(!component.destroy_on_deactivate);
        assert_eq!(
            SPAWNER_COMPONENT_TYPE_ID,
            uuid!("8022A627-DD76-5432-C75A-7234AC2798C1")
        );
        assert_eq!(
            DEPRECATED_SPAWNER_COMPONENT_TYPE_ID,
            uuid!("8022A627-FA7D-4516-A155-657A0927A3CA")
        );
    }
}
