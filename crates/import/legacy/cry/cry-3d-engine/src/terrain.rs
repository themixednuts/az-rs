use bevy::math::bounding::Aabb3d;

use crate::ParseError;
use crate::object_tree::ObjectTree;
use crate::read::{Cursor, Endian};

pub const OCTREE_CHUNK_VERSION: u8 = 29;
pub const TERRAIN_NODE_CHUNK_VERSION: i16 = 8;
pub const SERIALIZATION_FLAG_BIG_ENDIAN: u8 = 1;
pub const SERIALIZATION_FLAG_SECTOR_PALETTES: u8 = 2;
pub const TERRAIN_CHUNK_HEADER_SIZE: usize = 32;
pub const TERRAIN_NODE_CHUNK_SIZE: usize = 44;
pub const STAT_INST_GROUP_CHUNK_SIZE: usize = 360;
pub const NAME_CHUNK_SIZE: usize = 256;
pub const SURFACE_WEIGHT_SIZE: usize = 6;

/// `terrain/terrain.dat` compiled octree and terrain payload.
///
/// Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine/3dEngineOctreeCompile.cpp`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTerrain<'a> {
    bytes: &'a [u8],
    header: TerrainChunkHeader,
    vegetation_groups: FixedRecords<'a, STAT_INST_GROUP_CHUNK_SIZE>,
    brush_names: NameRecords<'a>,
    material_names: NameRecords<'a>,
    nodes: Vec<TerrainNode<'a>>,
    object_tree: Option<ObjectTree<'a>>,
}

impl<'a> CompiledTerrain<'a> {
    /// Parse a compiled terrain payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the header, tables, terrain node stream, or
    /// object-tree stream is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let header = read_chunk_header(bytes)?;
        let endian = header.endian;

        let mut cursor = Cursor::new(&bytes[TERRAIN_CHUNK_HEADER_SIZE..]);
        let vegetation_groups = read_fixed_records::<STAT_INST_GROUP_CHUNK_SIZE>(
            bytes,
            TERRAIN_CHUNK_HEADER_SIZE,
            &mut cursor,
            endian,
            "vegetation groups",
        )?;
        let brush_names = read_name_records(
            bytes,
            TERRAIN_CHUNK_HEADER_SIZE,
            &mut cursor,
            endian,
            "brush names",
        )?;
        let material_names = read_name_records(
            bytes,
            TERRAIN_CHUNK_HEADER_SIZE,
            &mut cursor,
            endian,
            "brush material names",
        )?;

        let nodes = read_terrain_nodes(bytes, &mut cursor, &header)?;
        let object_tree = read_trailing_object_tree(bytes, &mut cursor, endian)?;
        if cursor.remaining() != 0 {
            return Err(ParseError::ChunkSizeMismatch {
                declared: bytes.len(),
                actual: TERRAIN_CHUNK_HEADER_SIZE + cursor.position(),
            });
        }

        Ok(Self {
            bytes,
            header,
            vegetation_groups,
            brush_names,
            material_names,
            nodes,
            object_tree,
        })
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> TerrainChunkHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn vegetation_groups(&self) -> FixedRecords<'a, STAT_INST_GROUP_CHUNK_SIZE> {
        self.vegetation_groups
    }

    #[inline]
    #[must_use]
    pub const fn brush_names(&self) -> NameRecords<'a> {
        self.brush_names
    }

    #[inline]
    #[must_use]
    pub const fn material_names(&self) -> NameRecords<'a> {
        self.material_names
    }

    #[inline]
    #[must_use]
    pub fn nodes(&self) -> &[TerrainNode<'a>] {
        &self.nodes
    }

    #[inline]
    #[must_use]
    pub const fn object_tree(&self) -> Option<&ObjectTree<'a>> {
        self.object_tree.as_ref()
    }

    #[must_use]
    pub fn height_node_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.has_height_data())
            .count()
    }

    #[must_use]
    pub fn surface_palette_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| !node.surface_type_palette.is_empty())
            .count()
    }
}

/// `STerrainChunkHeader`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainChunkHeader {
    pub version: u8,
    pub dummy: u8,
    pub flags: u8,
    pub flags2: u8,
    pub chunk_size: i32,
    pub terrain_info: TerrainInfo,
    pub endian: Endian,
}

impl TerrainChunkHeader {
    #[inline]
    #[must_use]
    pub const fn has_sector_palettes(self) -> bool {
        self.flags & SERIALIZATION_FLAG_SECTOR_PALETTES != 0
    }
}

/// `STerrainInfo`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainInfo {
    pub height_map_size_units: i32,
    pub unit_size_meters: i32,
    pub sector_size_meters: i32,
    pub sectors_table_size: i32,
    pub heightmap_z_ratio: f32,
    pub ocean_water_level: f32,
}

impl TerrainInfo {
    /// Terrain edge length in meters.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::IntegerOverflow`] when the header's unit count
    /// times its unit size does not fit in `i32`.
    #[inline]
    pub fn terrain_size_meters(self) -> Result<i32, ParseError> {
        self.height_map_size_units
            .checked_mul(self.unit_size_meters)
            .ok_or(ParseError::IntegerOverflow)
    }

    /// Derive the sector shift and terrain extent this header implies.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidSize`] when `nUnitSize_InMeters` or
    /// `nSectorSize_InMeters` is not positive, or any error
    /// [`Self::terrain_size_meters`] returns.
    pub fn settings(self) -> Result<TerrainSettings, ParseError> {
        if self.unit_size_meters <= 0 {
            return Err(ParseError::InvalidSize {
                field: "STerrainInfo.nUnitSize_InMeters",
                size: self.unit_size_meters,
            });
        }
        if self.sector_size_meters <= 0 {
            return Err(ParseError::InvalidSize {
                field: "STerrainInfo.nSectorSize_InMeters",
                size: self.sector_size_meters,
            });
        }

        let mut unit_to_sector_bit_shift = 0u32;
        while (self.sector_size_meters >> unit_to_sector_bit_shift) > self.unit_size_meters {
            unit_to_sector_bit_shift += 1;
        }
        Ok(TerrainSettings {
            terrain_size_meters: self.terrain_size_meters()?,
            unit_to_sector_bit_shift,
        })
    }

    /// Count the quadtree nodes this header's terrain extent implies.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::settings`] returns, or
    /// [`ParseError::IntegerOverflow`] when the accumulated node count or a
    /// per-level row width overflows `usize`.
    pub fn terrain_node_count(self) -> Result<usize, ParseError> {
        let settings = self.settings()?;
        let mut node_size = settings.terrain_size_meters;
        let mut floor = 0u32;
        let mut count = 0usize;
        loop {
            let row = 1usize
                .checked_shl(floor)
                .ok_or(ParseError::IntegerOverflow)?;
            count = count
                .checked_add(row.checked_mul(row).ok_or(ParseError::IntegerOverflow)?)
                .ok_or(ParseError::IntegerOverflow)?;
            node_size >>= 1;
            floor += 1;
            if node_size < self.sector_size_meters || node_size <= 0 {
                break;
            }
        }
        Ok(count)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainSettings {
    pub terrain_size_meters: i32,
    pub unit_to_sector_bit_shift: u32,
}

/// `STerrainNodeChunk` plus its borrowed height and surface payloads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainNode<'a> {
    pub index: usize,
    pub header: TerrainNodeHeader,
    heights: &'a [u8],
    surface_weights: &'a [u8],
    lod_errors: &'a [u8],
    surface_type_palette: &'a [u8],
}

impl<'a> TerrainNode<'a> {
    #[inline]
    #[must_use]
    pub const fn heights(self) -> &'a [u8] {
        self.heights
    }

    #[inline]
    #[must_use]
    pub const fn surface_weights(self) -> &'a [u8] {
        self.surface_weights
    }

    #[inline]
    #[must_use]
    pub const fn lod_errors(self) -> &'a [u8] {
        self.lod_errors
    }

    #[inline]
    #[must_use]
    pub const fn surface_type_palette(self) -> &'a [u8] {
        self.surface_type_palette
    }

    #[inline]
    #[must_use]
    pub const fn has_height_data(self) -> bool {
        !self.heights.is_empty()
    }
}

/// `STerrainNodeChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainNodeHeader {
    pub version: i16,
    pub has_holes: i16,
    pub heightmap_bounds: Aabb3d,
    pub offset: f32,
    pub range: f32,
    pub size: i32,
    pub surface_types: i32,
}

/// Borrowed fixed-size record table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedRecords<'a, const RECORD_SIZE: usize> {
    bytes: &'a [u8],
    count: usize,
}

impl<'a, const RECORD_SIZE: usize> FixedRecords<'a, RECORD_SIZE> {
    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.count
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    #[inline]
    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a [u8]> {
        let start = index.checked_mul(RECORD_SIZE)?;
        self.bytes.get(start..start + RECORD_SIZE)
    }
}

pub type NameRecords<'a> = FixedRecords<'a, NAME_CHUNK_SIZE>;

impl<'a> NameRecords<'a> {
    #[must_use]
    pub fn name(self, index: usize) -> Option<&'a str> {
        let record = self.get(index)?;
        let len = record
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(record.len());
        std::str::from_utf8(&record[..len]).ok()
    }
}

/// Read and validate the 32-byte `STerrainChunkHeader` prefix.
fn read_chunk_header(bytes: &[u8]) -> Result<TerrainChunkHeader, ParseError> {
    if bytes.len() < TERRAIN_CHUNK_HEADER_SIZE {
        return Err(ParseError::UnexpectedEof {
            offset: 0,
            needed: TERRAIN_CHUNK_HEADER_SIZE,
            actual: bytes.len(),
        });
    }

    let flags = bytes[2];
    let endian = Endian::from_big_endian_flag(flags & SERIALIZATION_FLAG_BIG_ENDIAN != 0);
    let header = TerrainChunkHeader {
        version: bytes[0],
        dummy: bytes[1],
        flags,
        flags2: bytes[3],
        chunk_size: endian.read_i32(bytes, 4)?,
        terrain_info: TerrainInfo {
            height_map_size_units: endian.read_i32(bytes, 8)?,
            unit_size_meters: endian.read_i32(bytes, 12)?,
            sector_size_meters: endian.read_i32(bytes, 16)?,
            sectors_table_size: endian.read_i32(bytes, 20)?,
            heightmap_z_ratio: endian.read_f32(bytes, 24)?,
            ocean_water_level: endian.read_f32(bytes, 28)?,
        },
        endian,
    };
    if header.version != OCTREE_CHUNK_VERSION {
        return Err(ParseError::UnsupportedVersion {
            asset: "STerrainChunkHeader",
            expected: i64::from(OCTREE_CHUNK_VERSION),
            found: i64::from(header.version),
        });
    }
    let chunk_size = usize::try_from(header.chunk_size).map_err(|_| ParseError::InvalidSize {
        field: "STerrainChunkHeader.nChunkSize",
        size: header.chunk_size,
    })?;
    if chunk_size != bytes.len() {
        return Err(ParseError::ChunkSizeMismatch {
            declared: chunk_size,
            actual: bytes.len(),
        });
    }

    Ok(header)
}

/// Read the quadtree's `STerrainNodeChunk` stream in breadth order.
fn read_terrain_nodes<'a>(
    bytes: &'a [u8],
    cursor: &mut Cursor<'a>,
    header: &TerrainChunkHeader,
) -> Result<Vec<TerrainNode<'a>>, ParseError> {
    let settings = header.terrain_info.settings()?;
    let node_count = header.terrain_info.terrain_node_count()?;
    let mut nodes = Vec::with_capacity(node_count);
    for index in 0..node_count {
        nodes.push(read_terrain_node(
            bytes,
            TERRAIN_CHUNK_HEADER_SIZE,
            cursor,
            header.endian,
            settings.unit_to_sector_bit_shift,
            index,
        )?);
    }
    Ok(nodes)
}

/// Read the optional object tree that follows the terrain node stream.
fn read_trailing_object_tree<'a>(
    bytes: &'a [u8],
    cursor: &mut Cursor<'a>,
    endian: Endian,
) -> Result<Option<ObjectTree<'a>>, ParseError> {
    if cursor.remaining() == 0 {
        return Ok(None);
    }
    let tree = ObjectTree::parse(
        &bytes[TERRAIN_CHUNK_HEADER_SIZE + cursor.position()..],
        endian,
    )?;
    cursor.skip(tree.bytes_read())?;
    Ok(Some(tree))
}

fn read_fixed_records<'a, const RECORD_SIZE: usize>(
    bytes: &'a [u8],
    base_offset: usize,
    cursor: &mut Cursor<'a>,
    endian: Endian,
    field: &'static str,
) -> Result<FixedRecords<'a, RECORD_SIZE>, ParseError> {
    let declared_count = cursor.read_i32(endian)?;
    let count = usize::try_from(declared_count).map_err(|_| ParseError::InvalidCount {
        field,
        count: declared_count,
    })?;
    let len = count
        .checked_mul(RECORD_SIZE)
        .ok_or(ParseError::IntegerOverflow)?;
    let start = base_offset
        .checked_add(cursor.position())
        .ok_or(ParseError::IntegerOverflow)?;
    let records = bytes
        .get(start..start + len)
        .ok_or_else(|| ParseError::UnexpectedEof {
            offset: start,
            needed: len,
            actual: bytes.len().saturating_sub(start),
        })?;
    cursor.skip(len)?;
    Ok(FixedRecords {
        bytes: records,
        count,
    })
}

fn read_name_records<'a>(
    bytes: &'a [u8],
    base_offset: usize,
    cursor: &mut Cursor<'a>,
    endian: Endian,
    field: &'static str,
) -> Result<NameRecords<'a>, ParseError> {
    read_fixed_records::<NAME_CHUNK_SIZE>(bytes, base_offset, cursor, endian, field)
}

fn read_terrain_node<'a>(
    bytes: &'a [u8],
    base_offset: usize,
    cursor: &mut Cursor<'a>,
    endian: Endian,
    unit_to_sector_bit_shift: u32,
    index: usize,
) -> Result<TerrainNode<'a>, ParseError> {
    let node_offset = base_offset
        .checked_add(cursor.position())
        .ok_or(ParseError::IntegerOverflow)?;
    let header_bytes = bytes
        .get(node_offset..node_offset + TERRAIN_NODE_CHUNK_SIZE)
        .ok_or_else(|| ParseError::UnexpectedEof {
            offset: node_offset,
            needed: TERRAIN_NODE_CHUNK_SIZE,
            actual: bytes.len().saturating_sub(node_offset),
        })?;
    cursor.skip(TERRAIN_NODE_CHUNK_SIZE)?;

    let header = TerrainNodeHeader {
        version: endian.read_i16(header_bytes, 0)?,
        has_holes: endian.read_i16(header_bytes, 2)?,
        heightmap_bounds: endian.read_aabb3d(header_bytes, 4)?,
        offset: endian.read_f32(header_bytes, 28)?,
        range: endian.read_f32(header_bytes, 32)?,
        size: endian.read_i32(header_bytes, 36)?,
        surface_types: endian.read_i32(header_bytes, 40)?,
    };
    if header.version != TERRAIN_NODE_CHUNK_VERSION {
        return Err(ParseError::UnsupportedVersion {
            asset: "STerrainNodeChunk",
            expected: i64::from(TERRAIN_NODE_CHUNK_VERSION),
            found: i64::from(header.version),
        });
    }
    let size = usize::try_from(header.size).map_err(|_| ParseError::InvalidSize {
        field: "STerrainNodeChunk.nSize",
        size: header.size,
    })?;
    let surface_types =
        usize::try_from(header.surface_types).map_err(|_| ParseError::InvalidCount {
            field: "STerrainNodeChunk.nSurfaceTypesNum",
            count: header.surface_types,
        })?;

    let (heights, surface_weights) = if size == 0 {
        (&[][..], &[][..])
    } else {
        let square = size.checked_mul(size).ok_or(ParseError::IntegerOverflow)?;
        let heights = cursor.read_bytes(
            square
                .checked_mul(size_of::<u16>())
                .ok_or(ParseError::IntegerOverflow)?,
        )?;
        cursor.align_remaining_to_4()?;
        let surface_weights = cursor.read_bytes(
            square
                .checked_mul(SURFACE_WEIGHT_SIZE)
                .ok_or(ParseError::IntegerOverflow)?,
        )?;
        cursor.align_remaining_to_4()?;
        (heights, surface_weights)
    };

    let lod_errors = cursor.read_bytes(
        (unit_to_sector_bit_shift as usize)
            .checked_mul(size_of::<f32>())
            .ok_or(ParseError::IntegerOverflow)?,
    )?;

    let surface_type_palette = if surface_types == 0 {
        &[][..]
    } else {
        let bytes = cursor.read_bytes(surface_types)?;
        cursor.align_remaining_to_4()?;
        bytes
    };

    Ok(TerrainNode {
        index,
        header,
        heights,
        surface_weights,
        lod_errors,
        surface_type_palette,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_info_counts_quadtree_nodes() {
        let info = TerrainInfo {
            height_map_size_units: 1024,
            unit_size_meters: 2,
            sector_size_meters: 64,
            sectors_table_size: 32,
            heightmap_z_ratio: 0.0,
            ocean_water_level: 25.0,
        };

        assert_eq!(info.settings().unwrap().unit_to_sector_bit_shift, 5);
        assert_eq!(info.terrain_node_count().unwrap(), 1365);
    }
}
