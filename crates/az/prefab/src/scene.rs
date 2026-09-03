//! Schema-free scene source identity retained while the legacy scene-manifest
//! builder is removed in the later Spawnable cutover.

use crate::PrefabAssetPath;

pub const SCENE_SOURCE_TYPE: &str = "azoth.scene.Scene";
pub const SCENE_SOURCE_ROOT: &str = "project:source-root";
pub const SCENE_PATH_PREFIX: &str = "scenes";
pub const SCENE_EXTENSION: &str = "scene.ron";

/// Engine-owned scene entrypoint expressed without the retired authored schema
/// model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scene {
    pub name: String,
    pub root_prefab: Option<PrefabAssetPath>,
}
