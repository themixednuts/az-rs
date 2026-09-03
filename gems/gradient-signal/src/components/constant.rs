use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

/// Constant gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/ConstantGradientComponent.h:34`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct ConstantGradientConfig {
    pub value: f32,
}

impl Default for ConstantGradientConfig {
    fn default() -> Self {
        Self { value: 1.0 }
    }
}

/// Runtime constant gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/ConstantGradientComponent.h:37`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.ConstantGradientComponent", version = 1)]
pub struct ConstantGradientComponent {
    pub configuration: ConstantGradientConfig,
}

impl ConstantGradientComponent {
    #[must_use]
    pub const fn sample_value(&self) -> f32 {
        self.configuration.value
    }
}
