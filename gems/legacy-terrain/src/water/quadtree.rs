//! Serialized terrain water quadtree.

use bevy::prelude::*;

use super::node::WaterNodeData;
use super::tile::WaterSurfaceTiles;

/// Serialized water quadtree for one terrain region.
///
/// Source: `resources/serialize.json`, type
/// `23082A77-84B8-423E-B4CD-F601AA5D1D44`.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
pub struct SerializableWaterQuadtree {
    pub region_size: i32,
    pub quadtree_nodes: Vec<WaterNodeData>,
}

impl SerializableWaterQuadtree {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.quadtree_nodes.is_empty()
    }

    #[must_use]
    pub fn root(&self) -> Option<&WaterNodeData> {
        self.quadtree_nodes.first()
    }

    #[must_use]
    pub fn has_surface(&self) -> bool {
        self.quadtree_nodes.iter().any(|node| node.has_surface())
    }

    #[must_use]
    pub const fn surface_tiles(&self) -> WaterSurfaceTiles<'_> {
        WaterSurfaceTiles {
            quadtree: self,
            next_index: 0,
        }
    }
}
