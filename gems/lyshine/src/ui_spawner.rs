use az_asset::UntypedAssetRef;
use az_derive::AzRtti;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// `LyShine` `UiSpawnerComponent` AZ component UUID.
pub const UI_SPAWNER_COMPONENT_TYPE_ID: Uuid = uuid!("5AF19874-04A4-4540-82FC-5F29EC854E31");

/// Spawns a UI canvas slice.
///
/// Source: `LyShine::UiSpawnerComponent`.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "LyShine::UiSpawnerComponent",
    UI_SPAWNER_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Serialize, Deserialize)]
pub struct UiSpawnerComponent {
    pub slice_asset: UntypedAssetRef,
    pub spawn_on_activate: bool,
}

impl UiSpawnerComponent {
    #[inline]
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.slice_asset.hint()
    }
}

impl Default for UiSpawnerComponent {
    fn default() -> Self {
        Self {
            slice_asset: UntypedAssetRef::empty(),
            spawn_on_activate: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct UiSpawnerComponentPlugin;

impl Plugin for UiSpawnerComponentPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<UntypedAssetRef>()
            .register_type::<UiSpawnerComponent>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_source() {
        let component = UiSpawnerComponent::default();

        assert!(component.slice_asset.is_empty());
        assert!(!component.spawn_on_activate);
        assert_eq!(
            UI_SPAWNER_COMPONENT_TYPE_ID,
            uuid!("5AF19874-04A4-4540-82FC-5F29EC854E31")
        );
    }
}
