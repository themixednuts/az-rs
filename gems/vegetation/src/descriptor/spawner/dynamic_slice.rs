use bevy::prelude::*;

/// Dynamic slice vegetation spawner.
///
/// Lumberyard reference: `dev/Gems/Vegetation/Code/Include/Vegetation/DynamicSliceInstanceSpawner.h:30`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct DynamicSliceInstanceSpawner {
    pub slice_asset_path: Option<String>,
    pub slice_variant: Option<String>,
}

impl DynamicSliceInstanceSpawner {
    pub fn has_empty_asset_references(&self) -> bool {
        self.slice_asset_path.as_deref().is_none_or(str::is_empty)
    }
}
