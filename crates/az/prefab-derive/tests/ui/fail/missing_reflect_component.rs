use az_prefab::{Prefab, ReflectPrefab};
use bevy_reflect::{Reflect, std_traits::ReflectDefault};

#[derive(Reflect, Default, Prefab)]
#[reflect(Default, Prefab)]
#[prefab(tag = "MissingComponent", version = 1)]
struct MissingComponent;

fn main() {}
