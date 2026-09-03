use az_prefab::{Prefab, PrefabTypeData, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, std_traits::ReflectDefault};

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "VersionZero", version = 0)]
struct VersionZero;

fn main() {
    let mut registry = TypeRegistry::default();
    registry.register::<VersionZero>();
    assert_eq!(
        registry
            .get(std::any::TypeId::of::<VersionZero>())
            .and_then(|registration| registration.data::<PrefabTypeData>())
            .map(|prefab| prefab.source_version),
        Some(0)
    );
}
