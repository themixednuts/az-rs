use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::MeshComponentRenderNode;

/// Static mesh component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/MeshComponent.h:331`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
// Mesh is a genuine native Prefab type at corpus scale (28% of all imported
// prefabs carry a mesh) — the earlier "Policy B" (register-without-
// PrefabTypeData, non-Prefab) was a placeholder and is superseded by this.
#[prefab(tag = "azoth.lmbr_central.MeshComponent", version = 1)]
pub struct MeshComponent {
    pub render_node: MeshComponentRenderNode,
    pub load_mesh_on_activate: bool,
}

impl Default for MeshComponent {
    fn default() -> Self {
        Self {
            render_node: MeshComponentRenderNode::default(),
            load_mesh_on_activate: true,
        }
    }
}

impl MeshComponent {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.render_node.scene_asset_path()
    }
}
