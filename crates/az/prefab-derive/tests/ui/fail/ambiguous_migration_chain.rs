use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, std_traits::ReflectDefault};

fn migrate(
    value: az_prefab::ErasedPrefabValue,
) -> Result<az_prefab::ErasedPrefabValue, az_prefab::PrefabBuildError> {
    Ok(value)
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(
    tag = "Ambiguous",
    version = 2,
    migration(from = 1, to = 2, migrate = migrate),
    migration(from = 1, to = 2, migrate = migrate)
)]
struct AmbiguousMigration;

fn main() {}
