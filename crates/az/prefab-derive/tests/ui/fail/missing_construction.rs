use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::Reflect;

#[derive(Component, Reflect, Prefab)]
#[reflect(Component, Prefab)]
#[prefab(tag = "MissingConstruction", version = 1)]
struct MissingConstruction;

fn main() {}
