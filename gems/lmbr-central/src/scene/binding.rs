use bevy::prelude::*;

/// Records the engine asset resolved for a renderable entity.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct SceneAssetBinding {
    engine_path: Option<String>,
}

impl SceneAssetBinding {
    #[must_use]
    pub const fn new(engine_path: Option<String>) -> Self {
        Self { engine_path }
    }

    #[must_use]
    pub fn engine_path(&self) -> Option<&str> {
        self.engine_path.as_deref()
    }
}
