use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::filter::MaterialTextureFilter;
use super::map::MaterialTextureMap;
use super::texture_type::MaterialTextureType;

/// Texture reference inside one material layer.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialTextureReference {
    pub map: MaterialTextureMap,
    pub image_asset_path: Option<String>,
    pub asset_id: Option<String>,
    pub filter: Option<MaterialTextureFilter>,
    pub is_tile_u: bool,
    pub is_tile_v: bool,
    pub texture_type: Option<MaterialTextureType>,
    pub texture_modifier: Vec<MaterialPublicParam>,
}

/// Public shader parameter retained in material metadata.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialPublicParam {
    pub name: String,
    pub value: String,
}
