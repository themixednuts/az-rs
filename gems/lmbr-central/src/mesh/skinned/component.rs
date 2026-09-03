use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::SkinnedMeshComponentRenderNode;

/// Skinned mesh component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/SkinnedMeshComponent.h:261`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct SkinnedMeshComponent {
    pub render_node: SkinnedMeshComponentRenderNode,
    pub load_mesh_on_activate: bool,
}

impl Default for SkinnedMeshComponent {
    fn default() -> Self {
        Self {
            render_node: SkinnedMeshComponentRenderNode::default(),
            load_mesh_on_activate: true,
        }
    }
}

impl SkinnedMeshComponent {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.load_mesh_on_activate
            .then(|| self.render_node.scene_asset_path())
            .flatten()
    }
}
