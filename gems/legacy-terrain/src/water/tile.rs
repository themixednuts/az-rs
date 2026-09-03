//! Water surface tile iteration.

use bevy::prelude::*;

use super::address::{WaterNodeAddress, level_base, level_side};
use super::node::WaterNodeData;
use super::quadtree::SerializableWaterQuadtree;

/// Renderable water tile resolved from a quadtree node.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct WaterSurfaceTile {
    pub address: WaterNodeAddress,
    pub node: WaterNodeData,
}

impl WaterSurfaceTile {
    #[must_use]
    pub fn size(self, region_size: f32) -> Option<f32> {
        self.address.tile_size(region_size)
    }

    #[must_use]
    pub fn min(self, origin: Vec2, region_size: f32) -> Option<Vec2> {
        self.address.min(origin, region_size)
    }
}

/// Iterator over leaf water-surface tiles.
pub struct WaterSurfaceTiles<'a> {
    pub(super) quadtree: &'a SerializableWaterQuadtree,
    pub(super) next_index: usize,
}

impl Iterator for WaterSurfaceTiles<'_> {
    type Item = WaterSurfaceTile;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(node) = self.quadtree.quadtree_nodes.get(self.next_index).copied() {
            let index = self.next_index;
            self.next_index += 1;

            if !node.has_surface() {
                continue;
            }

            let Some(address) = WaterNodeAddress::from_level_major_index(index) else {
                continue;
            };
            if self.quadtree.has_surface_descendant(address) {
                continue;
            }

            return Some(WaterSurfaceTile { address, node });
        }

        None
    }
}

impl SerializableWaterQuadtree {
    fn has_surface_descendant(&self, address: WaterNodeAddress) -> bool {
        let Some(mut level) = address.level.checked_add(1) else {
            return false;
        };

        loop {
            let Some(base) = level_base(level) else {
                return false;
            };
            if base >= self.quadtree_nodes.len() {
                return false;
            }

            let Some(side) = level_side(level) else {
                return false;
            };
            let Some(scale) = 1u32.checked_shl(u32::from(level - address.level)) else {
                return false;
            };
            let Some(min_x) = address.x.checked_mul(scale) else {
                return false;
            };
            let Some(max_x) = address.x.checked_add(1).and_then(|x| x.checked_mul(scale)) else {
                return false;
            };
            let Some(min_y) = address.y.checked_mul(scale) else {
                return false;
            };
            let Some(max_y) = address.y.checked_add(1).and_then(|y| y.checked_mul(scale)) else {
                return false;
            };

            for y in min_y..max_y {
                let Some(row_offset) = (y as usize).checked_mul(side as usize) else {
                    return false;
                };
                let Some(row_start) = base.checked_add(row_offset) else {
                    return false;
                };
                let Some(start) = row_start.checked_add(min_x as usize) else {
                    return false;
                };
                if start >= self.quadtree_nodes.len() {
                    return false;
                }
                let Some(row_end) = row_start.checked_add(max_x as usize) else {
                    return false;
                };
                let end = row_end.min(self.quadtree_nodes.len());
                if self.quadtree_nodes[start..end]
                    .iter()
                    .any(|node| node.has_surface())
                {
                    return true;
                }
            }

            level = match level.checked_add(1) {
                Some(level) => level,
                None => return false,
            };
        }
    }
}
