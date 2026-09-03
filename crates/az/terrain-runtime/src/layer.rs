#[cfg_attr(feature = "bevy", derive(bevy_asset::Asset, bevy_reflect::TypePath))]
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainLayerSetAsset {
    pub name: String,
    pub layers: Vec<TerrainLayer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainLayer {
    pub tag: SurfaceTag,
    pub priority: i32,
    pub material: Option<String>,
    pub physics_material: Option<String>,
    pub texture_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceTag {
    pub name: String,
}
