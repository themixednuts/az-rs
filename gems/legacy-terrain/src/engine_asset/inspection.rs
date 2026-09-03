use std::fmt;
use std::path::Path;

use crate::TerrainRegionAsset;

use super::error::TerrainRegionAssetFormatError;
use super::region::read_terrain_region_asset_from_reader;

#[derive(Debug, Clone, Copy)]
pub struct TerrainRegionAssetInspection<'a> {
    asset: &'a TerrainRegionAsset,
}

impl<'a> TerrainRegionAssetInspection<'a> {
    #[must_use]
    pub const fn new(asset: &'a TerrainRegionAsset) -> Self {
        Self { asset }
    }
}

impl fmt::Display for TerrainRegionAssetInspection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let asset = self.asset;
        writeln!(
            f,
            "  region:         {}, {}",
            asset.region.x, asset.region.y
        )?;
        writeln!(
            f,
            "  origin:         {}, {}",
            asset.origin.x, asset.origin.y
        )?;
        writeln!(f, "  world size:     {}", asset.region_world_size())?;
        writeln!(f, "  heightmap:      {}", asset.heightmap.resolution)?;
        writeln!(
            f,
            "  height min/max: {} / {}",
            asset.heightmap.min_height, asset.heightmap.max_height,
        )?;
        writeln!(f, "  surface:        {}", asset.surface_resolution)?;
        writeln!(f, "  surface cells:  {}", asset.surface_weights.len())?;
        writeln!(f, "  materials:      {}", asset.material_layers.len())?;
        if !asset.default_material_path.is_empty() {
            writeln!(f, "  default mtl:    {}", asset.default_material_path)?;
        }
        if let Some(path) = asset.render_material_path() {
            writeln!(f, "  render mtl:     {path}")?;
        }
        writeln!(
            f,
            "  water nodes:    {}",
            asset
                .water_quadtree
                .as_ref()
                .map_or(0, |water| water.quadtree_nodes.len()),
        )
    }
}

#[derive(Debug, Clone)]
pub struct TerrainRegionAssetFileInspection<'a> {
    path: &'a Path,
    asset: TerrainRegionAsset,
}

impl<'a> TerrainRegionAssetFileInspection<'a> {
    #[must_use]
    pub const fn new(path: &'a Path, asset: TerrainRegionAsset) -> Self {
        Self { path, asset }
    }
}

impl fmt::Display for TerrainRegionAssetFileInspection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.path.display())?;
        write!(f, "{}", TerrainRegionAssetInspection::new(&self.asset))
    }
}

/// Read a terrain-region asset file and wrap it for display.
///
/// # Errors
///
/// Returns [`TerrainRegionAssetFormatError::Io`] if `path` cannot be opened,
/// or any error [`read_terrain_region_asset_from_reader`] returns for the
/// file's contents.
pub fn inspect_terrain_region_asset_file(
    path: &Path,
) -> Result<TerrainRegionAssetFileInspection<'_>, TerrainRegionAssetFormatError> {
    let mut file = std::fs::File::open(path)?;
    let asset = read_terrain_region_asset_from_reader(&mut file)?;
    Ok(TerrainRegionAssetFileInspection::new(path, asset))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bevy::prelude::Vec2;

    use crate::{RegionHeightmap, TerrainRegionAsset, TerrainRegionId};

    use super::{TerrainRegionAssetFileInspection, TerrainRegionAssetInspection};

    #[test]
    fn displays_engine_terrain_region_inspection() {
        let heightmap = RegionHeightmap::from_samples(vec![1, 2, 3, 4], 2, 1.0, 2.0, -4.0).unwrap();
        let asset = TerrainRegionAsset {
            region: TerrainRegionId::new(2, 3),
            origin: Vec2::new(4096.0, 6144.0),
            heightmap,
            default_material_path: "materials/terrain/default.mtl".to_string(),
            ..Default::default()
        };

        assert_eq!(
            TerrainRegionAssetInspection::new(&asset).to_string(),
            "  region:         2, 3\n  origin:         4096, 6144\n  world size:     2\n  heightmap:      2\n  height min/max: -4 / 4\n  surface:        0\n  surface cells:  0\n  materials:      0\n  default mtl:    materials/terrain/default.mtl\n  render mtl:     materials/terrain/default.mtl\n  water nodes:    0\n"
        );
        assert_eq!(
            TerrainRegionAssetFileInspection::new(
                Path::new("terrain/r_2_3.terrain-region.bin"),
                asset
            )
            .to_string(),
            "terrain/r_2_3.terrain-region.bin\n  region:         2, 3\n  origin:         4096, 6144\n  world size:     2\n  heightmap:      2\n  height min/max: -4 / 4\n  surface:        0\n  surface cells:  0\n  materials:      0\n  default mtl:    materials/terrain/default.mtl\n  render mtl:     materials/terrain/default.mtl\n  water nodes:    0\n"
        );
    }
}
