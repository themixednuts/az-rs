use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::non_empty_path;

use super::MeshRenderOptions;

/// Static mesh render node data.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/MeshComponent.cpp:238`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct MeshComponentRenderNode {
    pub visible: bool,
    pub static_mesh_asset_path: Option<String>,
    pub material_override_asset_path: Option<String>,
    pub material_overcoat_asset_path: Option<String>,
    pub render_options: MeshRenderOptions,
}

impl Default for MeshComponentRenderNode {
    fn default() -> Self {
        Self {
            visible: true,
            static_mesh_asset_path: None,
            material_override_asset_path: None,
            material_overcoat_asset_path: None,
            render_options: MeshRenderOptions::default(),
        }
    }
}

impl MeshComponentRenderNode {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        non_empty_path(self.static_mesh_asset_path.as_deref())
    }
}
