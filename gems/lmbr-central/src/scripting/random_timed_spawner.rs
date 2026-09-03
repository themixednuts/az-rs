use az_core::math::RandomDistributionType;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::RandomTimedSpawnerComponent` AZ component UUID.
pub const RANDOM_TIMED_SPAWNER_COMPONENT_TYPE_ID: Uuid =
    uuid!("8EE9EC2C-1CC9-4F88-968F-CFD20C380694");

/// Lumberyard `LmbrCentral::RandomTimedSpawnerConfiguration` type UUID.
pub const RANDOM_TIMED_SPAWNER_CONFIGURATION_TYPE_ID: Uuid =
    uuid!("4133644F-FADA-4C82-A2A2-B587B20E81FA");

/// Reflected authoring settings for timed random spawning.
///
/// Source: `LmbrCentral::RandomTimedSpawnerConfiguration`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "LmbrCentral::RandomTimedSpawnerConfiguration",
    RANDOM_TIMED_SPAWNER_CONFIGURATION_TYPE_ID,
    register
)]
#[reflect(Serialize, Deserialize)]
pub struct RandomTimedSpawnerConfiguration {
    pub enabled: bool,
    pub random_distribution: RandomDistributionType,
    pub spawn_delay: f64,
    pub spawn_delay_variation: f64,
}

impl Default for RandomTimedSpawnerConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            random_distribution: RandomDistributionType::UniformReal,
            spawn_delay: 5.0,
            spawn_delay_variation: 0.0,
        }
    }
}

/// Spawns through the local spawner service at timed random points inside a shape.
///
/// Source: `LmbrCentral::RandomTimedSpawnerComponent`.
#[derive(
    AzRtti,
    Component,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_rtti(
    name = "LmbrCentral::RandomTimedSpawnerComponent",
    RANDOM_TIMED_SPAWNER_COMPONENT_TYPE_ID,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.RandomTimedSpawnerComponent", version = 1)]
pub struct RandomTimedSpawnerComponent {
    pub config: RandomTimedSpawnerConfiguration,
}

impl RandomTimedSpawnerComponent {
    #[inline]
    #[must_use]
    pub const fn new(config: RandomTimedSpawnerConfiguration) -> Self {
        Self { config }
    }

    #[inline]
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn defaults_match_source() {
        let component = RandomTimedSpawnerComponent::default();

        assert!(component.is_enabled());
        assert_eq!(
            component.config.random_distribution,
            RandomDistributionType::UniformReal
        );
        assert_eq!(component.config.spawn_delay, 5.0);
        assert_eq!(component.config.spawn_delay_variation, 0.0);
        assert_eq!(
            RANDOM_TIMED_SPAWNER_COMPONENT_TYPE_ID,
            uuid!("8EE9EC2C-1CC9-4F88-968F-CFD20C380694")
        );
    }

    #[test]
    fn random_distribution_preserves_unknown_values() {
        assert_eq!(
            RandomDistributionType::from(0),
            RandomDistributionType::Normal
        );
        assert_eq!(
            RandomDistributionType::from(1),
            RandomDistributionType::UniformReal
        );
        assert_eq!(u32::from(RandomDistributionType::from(7)), 7);
    }
}
