use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, std_traits::ReflectDefault};

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(
    tag = "Current",
    version = 1,
    alias(tag = "Old", version = 1),
    alias(tag = "Old", version = 1)
)]
struct AliasCollision;

fn main() {}
