use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::config::LightConfiguration;

/// Runtime light component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/LightComponent.h:170`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.LightComponent", version = 1)]
pub struct LightComponent {
    pub configuration: LightConfiguration,
}
