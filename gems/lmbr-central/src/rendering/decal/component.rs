use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::config::DecalConfiguration;

/// Decal component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/DecalComponent.h:104`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.DecalComponent", version = 1)]
pub struct DecalComponent {
    pub configuration: DecalConfiguration,
}
