//! Shared rendering types and preview mesh helpers.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Lumberyard renderer system spec metadata.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/IEntityRenderState.h:768`.
///
/// Some converted Lumberyard slices still store `CryEngine`'s
/// `CONFIG_AUTO_SPEC = 0`; Lumberyard's legacy conversion maps that to
/// `EngineSpec::Low` because Auto is presented as "All" in the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum EngineSpec {
    #[default]
    Low,
    Medium,
    High,
    VeryHigh,
    Never,
}

impl EngineSpec {
    #[must_use]
    pub const fn enables_shadow_casting(self) -> bool {
        !matches!(self, Self::Never)
    }

    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            // Native 0 (unset) and 1 both mean the lowest spec.
            0 | 1 => Some(Self::Low),
            2 => Some(Self::Medium),
            3 => Some(Self::High),
            4 => Some(Self::VeryHigh),
            u32::MAX => Some(Self::Never),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::VeryHigh => 4,
            Self::Never => u32::MAX,
        }
    }
}

pub(super) fn preview_quad_mesh(size: f32, offset: Vec3) -> Mesh {
    let half_size = size.max(0.01) * 0.5;
    let positions = vec![
        [offset.x - half_size, offset.y - half_size, offset.z],
        [offset.x + half_size, offset.y - half_size, offset.z],
        [offset.x + half_size, offset.y + half_size, offset.z],
        [offset.x - half_size, offset.y + half_size, offset.z],
    ];
    let normals = vec![[0.0, 0.0, 1.0]; 4];
    let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}
