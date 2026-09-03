use az_prefab::{Prefab, PrefabProductPolicy, PrefabTypeData, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, std_traits::ReflectDefault};

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "SourceOnly", version = 1, product = SourceOnly)]
struct SourceOnly;

fn main() {
    let mut registry = TypeRegistry::default();
    registry.register::<SourceOnly>();
    assert_eq!(
        registry
            .get(std::any::TypeId::of::<SourceOnly>())
            .and_then(|registration| registration.data::<PrefabTypeData>())
            .map(|prefab| prefab.product_policy),
        Some(PrefabProductPolicy::SourceOnly)
    );
}
