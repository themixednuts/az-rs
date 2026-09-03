use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::static_mesh::MeshComponentRenderNode;

/// Instanced mesh render node data.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct InstancedMeshComponentRenderNode {
    pub mesh: MeshComponentRenderNode,
    #[serde(with = "transform_vec_serde")]
    pub instance_transforms: Vec<Transform>,
}

impl InstancedMeshComponentRenderNode {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.mesh.scene_asset_path()
    }
}

/// Instanced mesh component.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.InstancedMeshComponent", version = 1)]
pub struct InstancedMeshComponent {
    pub render_node: InstancedMeshComponentRenderNode,
}

impl InstancedMeshComponent {
    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.render_node.scene_asset_path()
    }

    #[must_use]
    pub fn instance_transforms(&self) -> &[Transform] {
        &self.render_node.instance_transforms
    }
}

/// Child entities spawned for one instanced mesh component.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct InstancedMeshChildren(pub Vec<Entity>);

/// Marker for entities spawned from an instanced mesh component.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstancedMeshInstance;

mod transform_vec_serde {
    use bevy::prelude::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct TransformRepr {
        translation: Vec3,
        rotation: Quat,
        scale: Vec3,
    }

    impl From<&Transform> for TransformRepr {
        fn from(value: &Transform) -> Self {
            Self {
                translation: value.translation,
                rotation: value.rotation,
                scale: value.scale,
            }
        }
    }

    impl From<TransformRepr> for Transform {
        fn from(value: TransformRepr) -> Self {
            Self {
                translation: value.translation,
                rotation: value.rotation,
                scale: value.scale,
            }
        }
    }

    pub fn serialize<S>(value: &[Transform], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value
            .iter()
            .map(TransformRepr::from)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Transform>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<TransformRepr>::deserialize(deserializer)
            .map(|values| values.into_iter().map(Transform::from).collect())
    }
}
