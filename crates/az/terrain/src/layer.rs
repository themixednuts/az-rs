use az_core::AssetPathBuf;
use serde::{Deserialize, Serialize};

/// Editable terrain layer set source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainLayerSetSource {
    pub name: String,

    pub layers: Vec<TerrainLayer>,
}

/// Surface tag and material bindings for one terrain layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainLayer {
    pub tag: SurfaceTag,

    pub priority: i32,

    pub material: Option<AssetPathBuf>,

    pub physics_material: Option<AssetPathBuf>,

    pub texture_scale: f32,
}

/// Stable surface tag used by terrain layers and surface maps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTag {
    pub name: String,
}
