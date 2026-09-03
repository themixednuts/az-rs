use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::Reflect;

#[derive(Reflect)]
struct SourceTemplate;

#[derive(Component, Reflect, Prefab)]
#[reflect(Component, Prefab)]
#[prefab(tag = "TemplateMissingAdapter", version = 1, template = SourceTemplate)]
struct TemplateMissingAdapter;

fn main() {}
