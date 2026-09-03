use az_prefab::{Prefab, PrefabProductPolicy, PrefabTypeData, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, std_traits::ReflectDefault};

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "Direct", version = 1)]
struct Direct {
    amount: f32,
}

fn main() {
    let mut registry = TypeRegistry::default();
    registry.register::<Direct>();
    assert_eq!(
        registry
            .get(std::any::TypeId::of::<Direct>())
            .and_then(|registration| registration.data::<PrefabTypeData>())
            .map(|prefab| prefab.tag),
        Some("Direct")
    );
    assert_eq!(
        registry
            .get(std::any::TypeId::of::<Direct>())
            .and_then(|registration| registration.data::<PrefabTypeData>())
            .map(|prefab| prefab.product_policy),
        Some(PrefabProductPolicy::Runtime)
    );
}
