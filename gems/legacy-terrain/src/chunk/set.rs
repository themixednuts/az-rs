use bevy::prelude::*;

/// Terrain system set for organizing terrain systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum TerrainSet {
    /// LOD and culling calculations.
    Lod,
    /// Mesh generation and updates.
    MeshGeneration,
}
