//! Water data used by `LegacyTerrain` regions.

mod address;
mod node;
mod quadtree;
mod tile;
mod type_ids;

pub use address::WaterNodeAddress;
pub use node::{WATER_NODE_EMPTY_HEIGHT, WaterNodeData, WaterNodeFlags};
pub use quadtree::SerializableWaterQuadtree;
pub use tile::{WaterSurfaceTile, WaterSurfaceTiles};
pub use type_ids::*;

#[cfg(test)]
use bevy::prelude::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_ids_match_serialized_type_contract() {
        assert_eq!(
            SERIALIZABLE_WATER_QUADTREE_TYPE_ID,
            "23082A77-84B8-423E-B4CD-F601AA5D1D44"
        );
        assert_eq!(
            WATER_NODE_DATA_TYPE_ID,
            "79BCCE0C-D451-47C0-B2A1-5CAD1D7313BD"
        );
    }

    #[test]
    fn water_flags_preserve_raw_bits() {
        let flags = WaterNodeFlags::from_bits(0b1010);

        assert_eq!(flags.bits(), 0b1010);
        assert!(flags.contains(0b0010));
        assert!(!flags.contains(0b0100));
        assert!(!flags.is_empty());
    }

    #[test]
    fn water_node_surface_requires_height_and_flags() {
        let surface = WaterNodeData {
            height: 78.0,
            floor_height: 0.0,
            flags: WaterNodeFlags::from_bits(0x0a00_0000),
        };
        let empty_height = WaterNodeData {
            height: WATER_NODE_EMPTY_HEIGHT,
            ..surface
        };
        let empty_flags = WaterNodeData {
            flags: WaterNodeFlags::default(),
            ..surface
        };

        assert!(surface.has_surface());
        assert!(!empty_height.has_surface());
        assert!(!empty_flags.has_surface());
    }

    #[test]
    fn water_node_address_maps_level_major_indices() {
        assert_eq!(
            WaterNodeAddress::from_level_major_index(0),
            Some(WaterNodeAddress::new(0, 0, 0))
        );
        assert_eq!(
            WaterNodeAddress::from_level_major_index(1),
            Some(WaterNodeAddress::new(1, 0, 0))
        );
        assert_eq!(
            WaterNodeAddress::from_level_major_index(4),
            Some(WaterNodeAddress::new(1, 1, 1))
        );
        assert_eq!(
            WaterNodeAddress::from_level_major_index(5),
            Some(WaterNodeAddress::new(2, 0, 0))
        );

        assert_eq!(WaterNodeAddress::new(2, 0, 0).level_major_index(), Some(5));
        assert_eq!(WaterNodeAddress::new(2, 1, 1).level_major_index(), Some(10));
        assert_eq!(WaterNodeAddress::new(2, 4, 0).level_major_index(), None);
    }

    // The asserted height is the literal stored in the node under test, so the
    // comparison is bit-identical by construction.
    #[allow(clippy::float_cmp)]
    #[test]
    fn water_surface_tiles_prefer_deeper_surface_nodes() {
        let empty = WaterNodeData {
            height: WATER_NODE_EMPTY_HEIGHT,
            floor_height: 0.0,
            flags: WaterNodeFlags::default(),
        };
        let surface = WaterNodeData {
            height: 12.0,
            floor_height: 1.0,
            flags: WaterNodeFlags::from_bits(0x0a00_0000),
        };
        let deep_surface = WaterNodeData {
            height: 14.0,
            ..surface
        };
        let mut nodes = vec![empty; 21];
        nodes[1] = surface;
        nodes[5] = deep_surface;

        let quadtree = SerializableWaterQuadtree {
            region_size: 2048,
            quadtree_nodes: nodes,
        };
        let tiles = quadtree.surface_tiles().collect::<Vec<_>>();

        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].address, WaterNodeAddress::new(2, 0, 0));
        assert_eq!(tiles[0].node.height, 14.0);
        assert_eq!(tiles[0].min(Vec2::ZERO, 2048.0), Some(Vec2::ZERO));
        assert_eq!(tiles[0].size(2048.0), Some(512.0));
    }
}
