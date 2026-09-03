use az_prefab::{Prefab, PrefabBuildError, PrefabTypeData, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent, template::TemplateContext};
use bevy_reflect::{Reflect, TypeRegistry};

#[derive(Reflect)]
struct RuntimeTemplate {
    amount: f32,
}

#[derive(Component, Reflect, Prefab)]
#[reflect(Component, Prefab)]
#[prefab(
    tag = "TemplateRuntime",
    version = 1,
    template = RuntimeTemplate,
    construct = build_runtime
)]
struct TemplateRuntime {
    amount: f32,
}

fn build_runtime(
    template: &RuntimeTemplate,
    _context: &mut TemplateContext<'_, '_>,
) -> Result<TemplateRuntime, PrefabBuildError> {
    Ok(TemplateRuntime {
        amount: template.amount,
    })
}

fn main() {
    let mut registry = TypeRegistry::default();
    registry.register::<TemplateRuntime>();
    assert!(
        registry
            .get(std::any::TypeId::of::<TemplateRuntime>())
            .and_then(|registration| registration.data::<PrefabTypeData>())
            .is_some()
    );
}
