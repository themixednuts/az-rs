use bevy::math::{Vec3A, bounding::Aabb3d};

use crate::ParseError;
use crate::object_tree::ObjectTree;
use crate::read::{Cursor, Endian};

pub const VISAREA_MANAGER_CHUNK_VERSION: u8 = 6;
pub const VISAREA_NODE_CHUNK_VERSION: i32 = 2;
pub const SERIALIZATION_FLAG_BIG_ENDIAN: u8 = 1;
pub const VISAREA_MANAGER_HEADER_SIZE: usize = 20;
pub const VISAREA_CHUNK_SIZE: usize = 260;
pub const MAX_VIS_AREA_CONNECTIONS: usize = 30;

/// `terrain/indoor.dat` vis-area manager payload.
///
/// Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine/VisAreaManCompile.cpp`.
#[derive(Debug, Clone, PartialEq)]
pub struct VisAreaManager<'a> {
    header: VisAreaManagerHeader,
    areas: Vec<VisArea<'a>>,
    portals: Vec<VisArea<'a>>,
    occlusion_areas: Vec<VisArea<'a>>,
}

impl<'a> VisAreaManager<'a> {
    /// Parse a vis-area manager payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the manager header or any area chunk is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < VISAREA_MANAGER_HEADER_SIZE {
            return Err(ParseError::UnexpectedEof {
                offset: 0,
                needed: VISAREA_MANAGER_HEADER_SIZE,
                actual: bytes.len(),
            });
        }

        let flags = bytes[2];
        let endian = Endian::from_big_endian_flag(flags & SERIALIZATION_FLAG_BIG_ENDIAN != 0);
        let header = VisAreaManagerHeader {
            version: bytes[0],
            dummy: bytes[1],
            flags,
            flags2: bytes[3],
            chunk_size: endian.read_i32(bytes, 4)?,
            vis_area_count: endian.read_i32(bytes, 8)?,
            portal_count: endian.read_i32(bytes, 12)?,
            occlusion_area_count: endian.read_i32(bytes, 16)?,
            endian,
        };
        if header.version != VISAREA_MANAGER_CHUNK_VERSION {
            return Err(ParseError::UnsupportedVersion {
                asset: "SVisAreaManChunkHeader",
                expected: i64::from(VISAREA_MANAGER_CHUNK_VERSION),
                found: i64::from(header.version),
            });
        }
        let chunk_size = checked_size("SVisAreaManChunkHeader.nChunkSize", header.chunk_size)?;
        let vis_area_count =
            checked_size("SVisAreaManChunkHeader.nVisAreasNum", header.vis_area_count)?;
        let portal_count = checked_size("SVisAreaManChunkHeader.nPortalsNum", header.portal_count)?;
        let occlusion_area_count = checked_size(
            "SVisAreaManChunkHeader.nOcclAreasNum",
            header.occlusion_area_count,
        )?;
        if chunk_size != bytes.len() {
            return Err(ParseError::ChunkSizeMismatch {
                declared: chunk_size,
                actual: bytes.len(),
            });
        }

        let mut cursor = Cursor::new(&bytes[VISAREA_MANAGER_HEADER_SIZE..]);
        let areas = read_vis_areas(&mut cursor, endian, vis_area_count)?;
        let portals = read_vis_areas(&mut cursor, endian, portal_count)?;
        let occlusion_areas = read_vis_areas(&mut cursor, endian, occlusion_area_count)?;
        if cursor.remaining() != 0 {
            return Err(ParseError::ChunkSizeMismatch {
                declared: bytes.len(),
                actual: VISAREA_MANAGER_HEADER_SIZE + cursor.position(),
            });
        }

        Ok(Self {
            header,
            areas,
            portals,
            occlusion_areas,
        })
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> VisAreaManagerHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub fn areas(&self) -> &[VisArea<'a>] {
        &self.areas
    }

    #[inline]
    #[must_use]
    pub fn portals(&self) -> &[VisArea<'a>] {
        &self.portals
    }

    #[inline]
    #[must_use]
    pub fn occlusion_areas(&self) -> &[VisArea<'a>] {
        &self.occlusion_areas
    }
}

/// `SVisAreaManChunkHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisAreaManagerHeader {
    pub version: u8,
    pub dummy: u8,
    pub flags: u8,
    pub flags2: u8,
    pub chunk_size: i32,
    pub vis_area_count: i32,
    pub portal_count: i32,
    pub occlusion_area_count: i32,
    pub endian: Endian,
}

/// `SVisAreaChunk` plus its borrowed shape/object-tree payloads.
#[derive(Debug, Clone, PartialEq)]
pub struct VisArea<'a> {
    pub header: VisAreaHeader,
    shape_points: &'a [u8],
    object_tree: Option<ObjectTree<'a>>,
}

impl<'a> VisArea<'a> {
    #[inline]
    #[must_use]
    pub const fn shape_points(&self) -> &'a [u8] {
        self.shape_points
    }

    #[inline]
    #[must_use]
    pub const fn object_tree(&self) -> Option<&ObjectTree<'a>> {
        self.object_tree.as_ref()
    }
}

/// `SVisAreaChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisAreaHeader {
    pub version: i32,
    pub area_bounds: Aabb3d,
    pub static_bounds: Aabb3d,
    pub name: [u8; 32],
    pub object_block_size: i32,
    pub connection_ids: [i32; MAX_VIS_AREA_CONNECTIONS],
    pub flags: u32,
    pub portal_blending: f32,
    pub connection_normals: [Vec3A; 2],
    pub height: f32,
    pub ambient_color: Vec3A,
    pub view_distance_ratio: f32,
}

impl VisAreaHeader {
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        let len = self
            .name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..len]).ok()
    }
}

fn read_vis_areas<'a>(
    cursor: &mut Cursor<'a>,
    endian: Endian,
    count: usize,
) -> Result<Vec<VisArea<'a>>, ParseError> {
    let mut areas = Vec::with_capacity(count);
    for _ in 0..count {
        areas.push(read_vis_area(cursor, endian)?);
    }
    Ok(areas)
}

fn read_vis_area<'a>(cursor: &mut Cursor<'a>, endian: Endian) -> Result<VisArea<'a>, ParseError> {
    let chunk = cursor.read_bytes(VISAREA_CHUNK_SIZE)?;
    let mut name = [0; 32];
    name.copy_from_slice(&chunk[52..84]);
    let mut connection_ids = [0; MAX_VIS_AREA_CONNECTIONS];
    for (index, connection_id) in connection_ids.iter_mut().enumerate() {
        *connection_id = endian.read_i32(chunk, 88 + index * 4)?;
    }

    let header = VisAreaHeader {
        version: endian.read_i32(chunk, 0)?,
        area_bounds: endian.read_aabb3d(chunk, 4)?,
        static_bounds: endian.read_aabb3d(chunk, 28)?,
        name,
        object_block_size: endian.read_i32(chunk, 84)?,
        connection_ids,
        flags: endian.read_u32(chunk, 208)?,
        portal_blending: endian.read_f32(chunk, 212)?,
        connection_normals: [
            endian.read_vec3a(chunk, 216)?,
            endian.read_vec3a(chunk, 228)?,
        ],
        height: endian.read_f32(chunk, 240)?,
        ambient_color: endian.read_vec3a(chunk, 244)?,
        view_distance_ratio: endian.read_f32(chunk, 256)?,
    };
    if header.version != VISAREA_NODE_CHUNK_VERSION {
        return Err(ParseError::UnsupportedVersion {
            asset: "SVisAreaChunk",
            expected: i64::from(VISAREA_NODE_CHUNK_VERSION),
            found: i64::from(header.version),
        });
    }
    let object_block_size =
        checked_size("SVisAreaChunk.nObjectsBlockSize", header.object_block_size)?;

    let declared_point_count = cursor.read_i32(endian)?;
    let point_count =
        usize::try_from(declared_point_count).map_err(|_| ParseError::InvalidCount {
            field: "SVisAreaChunk shape point count",
            count: declared_point_count,
        })?;
    let shape_points = cursor.read_bytes(
        point_count
            .checked_mul(12)
            .ok_or(ParseError::IntegerOverflow)?,
    )?;

    let object_tree = if object_block_size > 4 {
        let tree = ObjectTree::parse(cursor.read_bytes(object_block_size)?, endian)?;
        Some(tree)
    } else if object_block_size > 0 {
        cursor.skip(object_block_size)?;
        None
    } else {
        None
    };

    Ok(VisArea {
        header,
        shape_points,
        object_tree,
    })
}

/// Reject a negative on-disk size or count and widen it to `usize`.
fn checked_size(field: &'static str, value: i32) -> Result<usize, ParseError> {
    usize::try_from(value).map_err(|_| ParseError::InvalidSize { field, size: value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_indoor_header() {
        let bytes = [6, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let indoor = VisAreaManager::parse(&bytes).unwrap();

        assert_eq!(indoor.header().chunk_size, 20);
        assert!(indoor.areas().is_empty());
        assert!(indoor.portals().is_empty());
        assert!(indoor.occlusion_areas().is_empty());
    }
}
