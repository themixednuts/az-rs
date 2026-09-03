//! Terrain world manifest assets.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Cursor, Read, Write};

use bevy::asset::AsyncReadExt;
use bevy::asset::io::Reader;
use bevy::prelude::*;

use crate::TerrainRegionId;

use super::error::TerrainWorldManifestFormatError;

const MAGIC: &[u8; 8] = b"AZTRWLD\0";
const VERSION: u32 = 1;

/// Engine terrain world manifest.
#[derive(Asset, Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct TerrainWorldManifest {
    pub world_name: String,
    pub regions: Vec<TerrainWorldManifestRegion>,
}

impl fmt::Display for TerrainWorldManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  world:   {}", self.world_name)?;
        writeln!(f, "  regions: {}", self.regions.len())?;
        for region in &self.regions {
            writeln!(f, "    {},{} = {}", region.x, region.y, region.path)?;
        }
        Ok(())
    }
}

/// One engine region entry in a [`TerrainWorldManifest`].
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub struct TerrainWorldManifestRegion {
    pub x: i32,
    pub y: i32,
    pub path: String,
}

impl TerrainWorldManifestRegion {
    #[must_use]
    pub const fn region(&self) -> TerrainRegionId {
        TerrainRegionId::new(self.x, self.y)
    }
}

/// Information about one emitted terrain region product.
///
/// Asset processors collect these while writing region-rooted terrain
/// products, then feed them into [`build_terrain_world_manifests`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainRegionProduct {
    /// World root, relative to the source tree.
    ///
    /// Example: `sharedassets/example/overworld`.
    pub world_root: String,
    /// Region engine path relative to the cache root.
    ///
    /// Example: `terrain/sharedassets/.../r_+00_+00/region.terrain-region.bin`.
    pub region_path: String,
    /// X grid coordinate.
    pub x: i32,
    /// Y grid coordinate.
    pub y: i32,
}

/// Planned output for one generated terrain world manifest product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainWorldManifestProduct {
    pub world_root: String,
    pub engine_path: String,
    pub manifest: TerrainWorldManifest,
}

/// Try to derive a terrain region product from an engine product path.
///
/// Returns `None` for terrain products that are not rooted below a
/// `regions/r_+XX_+YY` directory.
#[must_use]
pub fn terrain_region_product_from_path(product_path: &str) -> Option<TerrainRegionProduct> {
    if !product_path.ends_with(".terrain-region.bin") {
        return None;
    }
    let rest = product_path.strip_prefix("terrain/")?;
    let regions_marker = "/regions/";
    let regions_idx = rest.find(regions_marker)?;
    let world_root = &rest[..regions_idx];
    let after_regions = &rest[regions_idx + regions_marker.len()..];
    let region_dir = after_regions.split('/').next()?;
    let (x, y) = parse_region_coords(region_dir)?;
    Some(TerrainRegionProduct {
        world_root: world_root.to_string(),
        region_path: product_path.to_string(),
        x,
        y,
    })
}

/// Build one terrain world manifest per distinct world root.
#[must_use]
pub fn build_terrain_world_manifests(
    regions: &[TerrainRegionProduct],
) -> Vec<TerrainWorldManifestProduct> {
    let mut by_world: BTreeMap<&str, Vec<&TerrainRegionProduct>> = BTreeMap::new();
    for region in regions {
        by_world
            .entry(region.world_root.as_str())
            .or_default()
            .push(region);
    }

    by_world
        .into_iter()
        .map(|(world_root, regions)| {
            let world_name = world_root
                .rsplit('/')
                .next()
                .unwrap_or(world_root)
                .to_string();
            TerrainWorldManifestProduct {
                world_root: world_root.to_string(),
                engine_path: terrain_world_manifest_engine_path(world_root),
                manifest: TerrainWorldManifest {
                    world_name,
                    regions: regions
                        .iter()
                        .map(|region| TerrainWorldManifestRegion {
                            x: region.x,
                            y: region.y,
                            path: region.region_path.clone(),
                        })
                        .collect(),
                },
            }
        })
        .collect()
}

/// Manifest engine path: `terrain-world/<world_root>.terrain-world.bin`.
#[must_use]
pub fn terrain_world_manifest_engine_path(world_root: &str) -> String {
    format!("terrain-world/{world_root}.terrain-world.bin")
}

fn parse_region_coords(dir: &str) -> Option<(i32, i32)> {
    let body = dir.strip_prefix("r_")?;
    let (x, y) = body.split_once('_')?;
    let x: i32 = parse_signed_field(x)?;
    let y: i32 = parse_signed_field(y)?;
    Some((x, y))
}

fn parse_signed_field(field: &str) -> Option<i32> {
    let (sign, rest) = field.split_at(field.char_indices().next()?.1.len_utf8());
    let n: i32 = rest.parse().ok()?;
    match sign {
        "+" => Some(n),
        "-" => Some(-n),
        _ => None,
    }
}

/// Write an engine terrain world manifest.
///
/// # Errors
///
/// Returns [`TerrainWorldManifestFormatError::TooManyItems`] if the region
/// count or a stored string is longer than `u32` can hold, or
/// [`TerrainWorldManifestFormatError::Io`] if `writer` fails.
pub fn write_terrain_world_manifest(
    manifest: &TerrainWorldManifest,
    mut writer: impl Write,
) -> Result<(), TerrainWorldManifestFormatError> {
    writer.write_all(MAGIC)?;
    write_u32(&mut writer, VERSION)?;
    write_string(&mut writer, &manifest.world_name)?;
    write_u32(
        &mut writer,
        checked_u32(manifest.regions.len(), "terrain world regions")?,
    )?;
    for region in &manifest.regions {
        write_i32(&mut writer, region.x)?;
        write_i32(&mut writer, region.y)?;
        write_string(&mut writer, &region.path)?;
    }
    Ok(())
}

/// Read an engine terrain world manifest.
///
/// # Errors
///
/// Returns any error [`read_terrain_world_manifest_from_reader`] returns.
pub fn read_terrain_world_manifest(
    bytes: &[u8],
) -> Result<TerrainWorldManifest, TerrainWorldManifestFormatError> {
    read_terrain_world_manifest_from_reader(Cursor::new(bytes))
}

/// Read an engine terrain world manifest from a stream.
///
/// # Errors
///
/// Returns [`TerrainWorldManifestFormatError::BadMagic`] if the stream does
/// not start with the manifest magic,
/// [`TerrainWorldManifestFormatError::UnsupportedVersion`] for any version
/// other than the current one, [`TerrainWorldManifestFormatError::Utf8`] if
/// the world name or a region path is not UTF-8, or
/// [`TerrainWorldManifestFormatError::Io`] if `reader` ends early or fails.
pub fn read_terrain_world_manifest_from_reader(
    mut reader: impl Read,
) -> Result<TerrainWorldManifest, TerrainWorldManifestFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(TerrainWorldManifestFormatError::BadMagic { found: magic });
    }

    let version = read_u32(&mut reader)?;
    if version != VERSION {
        return Err(TerrainWorldManifestFormatError::UnsupportedVersion {
            version,
            expected: VERSION,
        });
    }

    let world_name = read_string(&mut reader)?;
    let region_count = read_u32(&mut reader)? as usize;
    let mut regions = Vec::with_capacity(region_count);
    for _ in 0..region_count {
        regions.push(TerrainWorldManifestRegion {
            x: read_i32(&mut reader)?,
            y: read_i32(&mut reader)?,
            path: read_string(&mut reader)?,
        });
    }

    Ok(TerrainWorldManifest {
        world_name,
        regions,
    })
}

pub(super) async fn read_terrain_world_manifest_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<TerrainWorldManifest, TerrainWorldManifestFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await?;
    if &magic != MAGIC {
        return Err(TerrainWorldManifestFormatError::BadMagic { found: magic });
    }

    let version = read_async_u32(reader).await?;
    if version != VERSION {
        return Err(TerrainWorldManifestFormatError::UnsupportedVersion {
            version,
            expected: VERSION,
        });
    }

    let world_name = read_async_string(reader).await?;
    let region_count = read_async_u32(reader).await? as usize;
    let mut regions = Vec::with_capacity(region_count);
    for _ in 0..region_count {
        regions.push(TerrainWorldManifestRegion {
            x: read_async_i32(reader).await?,
            y: read_async_i32(reader).await?,
            path: read_async_string(reader).await?,
        });
    }

    Ok(TerrainWorldManifest {
        world_name,
        regions,
    })
}

fn write_string(
    writer: &mut impl Write,
    value: &str,
) -> Result<(), TerrainWorldManifestFormatError> {
    write_u32(writer, checked_u32(value.len(), "string bytes")?)?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String, TerrainWorldManifestFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_async_string(
    reader: &mut dyn Reader,
) -> Result<String, TerrainWorldManifestFormatError> {
    let len = read_async_u32(reader).await? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes).await?;
    Ok(String::from_utf8(bytes)?)
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_i32(writer: &mut impl Write, value: i32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i32(reader: &mut impl Read) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

async fn read_async_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

async fn read_async_i32(reader: &mut dyn Reader) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(i32::from_le_bytes(bytes))
}

fn checked_u32(count: usize, what: &'static str) -> Result<u32, TerrainWorldManifestFormatError> {
    u32::try_from(count).map_err(|_| TerrainWorldManifestFormatError::TooManyItems { what, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_region_path() {
        let region = terrain_region_product_from_path(
            "terrain/sharedassets/example/overworld/regions/r_+00_+01/region.terrain-region.bin",
        )
        .unwrap();

        assert_eq!(region.world_root, "sharedassets/example/overworld");
        assert_eq!(region.x, 0);
        assert_eq!(region.y, 1);
    }

    #[test]
    fn parses_negative_coordinates() {
        let region = terrain_region_product_from_path(
            "terrain/sharedassets/example/overworld/regions/r_-12_+34/region.terrain-region.bin",
        )
        .unwrap();

        assert_eq!(region.x, -12);
        assert_eq!(region.y, 34);
    }

    #[test]
    fn returns_none_for_non_region_terrain_products() {
        assert!(terrain_region_product_from_path("terrain/foo/bar.heightmap").is_none());
        assert!(
            terrain_region_product_from_path(
                "terrain/sharedassets/example/overworld/regions/r_-12_+34/region.surfacemap",
            )
            .is_none()
        );
    }

    #[test]
    fn returns_none_for_unrelated_products() {
        assert!(terrain_region_product_from_path("models/foo.static-model.bin").is_none());
    }

    #[test]
    fn plans_one_manifest_per_world() {
        let regions = vec![
            terrain_region_product_from_path(
                "terrain/sharedassets/example/overworld/regions/r_+00_+00/region.terrain-region.bin",
            )
            .unwrap(),
            terrain_region_product_from_path(
                "terrain/sharedassets/example/overworld/regions/r_+00_+01/region.terrain-region.bin",
            )
            .unwrap(),
        ];

        let products = build_terrain_world_manifests(&regions);

        assert_eq!(products.len(), 1);
        assert_eq!(products[0].world_root, "sharedassets/example/overworld");
        assert_eq!(
            products[0].engine_path,
            "terrain-world/sharedassets/example/overworld.terrain-world.bin"
        );
        assert_eq!(products[0].manifest.world_name, "overworld");
        assert_eq!(products[0].manifest.regions.len(), 2);
        assert_eq!(
            products[0].manifest.to_string(),
            "  world:   overworld\n  regions: 2\n    0,0 = terrain/sharedassets/example/overworld/regions/r_+00_+00/region.terrain-region.bin\n    0,1 = terrain/sharedassets/example/overworld/regions/r_+00_+01/region.terrain-region.bin\n"
        );
    }
}
