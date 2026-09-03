use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::non_empty_path;

use super::SkinnedRenderOptions;

/// Skinned mesh render node data.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/SkinnedMeshComponent.cpp:125`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct SkinnedMeshComponentRenderNode {
    pub visible: bool,
    pub skinned_mesh_asset_path: Option<String>,
    pub material_override_asset_path: Option<String>,
    pub material_overcoat_asset_path: Option<String>,
    pub render_options: SkinnedRenderOptions,
}

impl Default for SkinnedMeshComponentRenderNode {
    fn default() -> Self {
        Self {
            visible: true,
            skinned_mesh_asset_path: None,
            material_override_asset_path: None,
            material_overcoat_asset_path: None,
            render_options: SkinnedRenderOptions::default(),
        }
    }
}

impl SkinnedMeshComponentRenderNode {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        non_empty_path(self.skinned_mesh_asset_path.as_deref())
    }
}
