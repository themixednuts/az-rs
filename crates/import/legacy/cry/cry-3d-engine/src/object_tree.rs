use bevy::math::{Affine3A, Mat3A, Vec3A, Vec4, bounding::Aabb3d};

use crate::ParseError;
use crate::read::{Cursor, Endian};

pub const OCTREE_NODE_CHUNK_VERSION_OLD: i16 = 3;
pub const OCTREE_NODE_CHUNK_VERSION: i16 = 5;
pub const OCTREE_NODE_CHUNK_SIZE: usize = 32;
pub const RENDER_NODE_CHUNK_SIZE: usize = 44;
pub const VEGETATION_CHUNK_SIZE: usize = 64;
pub const MERGED_MESH_CHUNK_SIZE: usize = 72;
pub const MERGED_MESH_GROUP_CHUNK_SIZE: usize = 8;
pub const BRUSH_CHUNK_SIZE: usize = 104;
pub const ROAD_CHUNK_SIZE: usize = 72;
pub const DECAL_CHUNK_SIZE: usize = 124;
pub const WATER_VOLUME_CHUNK_SIZE: usize = 144;
pub const DISTANCE_CLOUD_CHUNK_SIZE: usize = 72;
pub const VEC3_CHUNK_SIZE: usize = 12;
pub const F32_CHUNK_SIZE: usize = 4;

/// Compiled object tree following terrain or vis-area data.
///
/// Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine/ObjectsTree.cpp`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectTree<'a> {
    root: ObjectTreeNode<'a>,
    bytes_read: usize,
}

impl<'a> ObjectTree<'a> {
    pub(crate) fn parse(bytes: &'a [u8], endian: Endian) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(bytes);
        let root = ObjectTreeNode::parse(&mut cursor, endian)?;
        if cursor.remaining() != 0 {
            return Err(ParseError::ChunkSizeMismatch {
                declared: bytes.len(),
                actual: cursor.position(),
            });
        }
        Ok(Self {
            root,
            bytes_read: cursor.position(),
        })
    }

    #[inline]
    #[must_use]
    pub const fn root(&self) -> &ObjectTreeNode<'a> {
        &self.root
    }

    #[inline]
    #[must_use]
    pub const fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.root.subtree_node_count()
    }

    #[must_use]
    pub fn object_block_count(&self) -> usize {
        self.root.subtree_object_block_count()
    }

    #[must_use]
    pub fn object_bytes(&self) -> usize {
        self.root.subtree_object_bytes()
    }

    #[must_use]
    pub fn object_count(&self) -> usize {
        self.root.subtree_object_count()
    }
}

/// One compiled object tree node.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectTreeNode<'a> {
    pub header: ObjectTreeNodeHeader,
    endian: Endian,
    object_block: &'a [u8],
    object_count: usize,
    children: Vec<Self>,
}

impl<'a> ObjectTreeNode<'a> {
    fn parse(cursor: &mut Cursor<'a>, endian: Endian) -> Result<Self, ParseError> {
        let offset = cursor.position();
        let header_bytes = cursor.read_bytes(OCTREE_NODE_CHUNK_SIZE)?;
        let header = ObjectTreeNodeHeader {
            version: endian.read_i16(header_bytes, 0)?,
            child_mask: endian.read_u16(header_bytes, 2)?,
            bounds: endian.read_aabb3d(header_bytes, 4)?,
            object_block_size: endian.read_i32(header_bytes, 28)?,
        };
        if header.version != OCTREE_NODE_CHUNK_VERSION
            && header.version != OCTREE_NODE_CHUNK_VERSION_OLD
        {
            return Err(ParseError::UnsupportedVersion {
                asset: "SOcTreeNodeChunk",
                expected: i64::from(OCTREE_NODE_CHUNK_VERSION),
                found: i64::from(header.version),
            });
        }
        let object_block_size =
            usize::try_from(header.object_block_size).map_err(|_| ParseError::InvalidSize {
                field: "SOcTreeNodeChunk.nObjectsBlockSize",
                size: header.object_block_size,
            })?;

        let object_block = cursor.read_bytes(object_block_size)?;
        let object_count = ObjectBlock::new(object_block, endian, header.version).validate()?;
        let child_count = header.child_mask.count_ones() as usize;
        let mut children = Vec::with_capacity(child_count);
        for child_id in 0..8 {
            if header.child_mask & (1u16 << child_id) != 0 {
                children.push(Self::parse(cursor, endian)?);
            }
        }

        debug_assert!(cursor.position() >= offset + OCTREE_NODE_CHUNK_SIZE);
        Ok(Self {
            header,
            endian,
            object_block,
            object_count,
            children,
        })
    }

    #[inline]
    #[must_use]
    pub const fn object_block(&self) -> &'a [u8] {
        self.object_block
    }

    #[inline]
    #[must_use]
    pub const fn objects(&self) -> ObjectBlock<'a> {
        ObjectBlock::new(self.object_block, self.endian, self.header.version)
    }

    #[inline]
    #[must_use]
    pub const fn object_count(&self) -> usize {
        self.object_count
    }

    #[inline]
    #[must_use]
    pub fn children(&self) -> &[Self] {
        &self.children
    }

    #[must_use]
    pub fn subtree_node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(Self::subtree_node_count)
            .sum::<usize>()
    }

    #[must_use]
    pub fn subtree_object_block_count(&self) -> usize {
        usize::from(!self.object_block.is_empty())
            + self
                .children
                .iter()
                .map(Self::subtree_object_block_count)
                .sum::<usize>()
    }

    #[must_use]
    pub fn subtree_object_bytes(&self) -> usize {
        self.object_block.len()
            + self
                .children
                .iter()
                .map(Self::subtree_object_bytes)
                .sum::<usize>()
    }

    #[must_use]
    pub fn subtree_object_count(&self) -> usize {
        self.object_count
            + self
                .children
                .iter()
                .map(Self::subtree_object_count)
                .sum::<usize>()
    }
}

/// `SOcTreeNodeChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectTreeNodeHeader {
    pub version: i16,
    pub child_mask: u16,
    pub bounds: Aabb3d,
    pub object_block_size: i32,
}

/// Borrowed object stream stored in one compiled object tree node.
///
/// Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine/ObjectsTree_Serialize.cpp:313`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectBlock<'a> {
    bytes: &'a [u8],
    endian: Endian,
    node_version: i16,
}

impl<'a> ObjectBlock<'a> {
    #[inline]
    #[must_use]
    pub const fn new(bytes: &'a [u8], endian: Endian, node_version: i16) -> Self {
        Self {
            bytes,
            endian,
            node_version,
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    #[must_use]
    pub const fn iter(self) -> ObjectBlockIter<'a> {
        ObjectBlockIter {
            cursor: Cursor::new(self.bytes),
            endian: self.endian,
            node_version: self.node_version,
            failed: false,
        }
    }

    /// Walk the block and count the render-node records it holds.
    ///
    /// # Errors
    ///
    /// Returns the first error the block's iterator yields — a truncated
    /// record ([`ParseError::UnexpectedEof`]) or an unrecognized type tag
    /// ([`ParseError::UnsupportedRenderNodeType`]) — or
    /// [`ParseError::IntegerOverflow`] if the record count exceeds `usize`.
    pub fn validate(self) -> Result<usize, ParseError> {
        let mut count = 0usize;
        for object in self.iter() {
            object?;
            count = count.checked_add(1).ok_or(ParseError::IntegerOverflow)?;
        }
        Ok(count)
    }
}

#[derive(Debug, Clone)]
pub struct ObjectBlockIter<'a> {
    cursor: Cursor<'a>,
    endian: Endian,
    node_version: i16,
    failed: bool,
}

impl<'a> Iterator for ObjectBlockIter<'a> {
    type Item = Result<RenderNodeObject<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.cursor.remaining() == 0 {
            return None;
        }

        let item = read_render_node_object(&mut self.cursor, self.endian, self.node_version);
        if item.is_err() {
            self.failed = true;
        }
        Some(item)
    }
}

/// Compiled render-node kind stored before each object chunk.
///
/// Follows Lumberyard's `dev/Code/CryEngine/CryCommon/IEntityRenderState.h:39`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RenderNodeType {
    Brush = 1,
    Vegetation = 2,
    Decal = 7,
    WaterVolume = 9,
    Road = 11,
    DistanceCloud = 12,
    MergedMesh = 23,
}

impl RenderNodeType {
    const fn read(value: i32, offset: usize) -> Result<Self, ParseError> {
        match value {
            1 => Ok(Self::Brush),
            2 => Ok(Self::Vegetation),
            7 => Ok(Self::Decal),
            9 => Ok(Self::WaterVolume),
            11 => Ok(Self::Road),
            12 => Ok(Self::DistanceCloud),
            23 => Ok(Self::MergedMesh),
            _ => Err(ParseError::UnsupportedRenderNodeType { offset, value }),
        }
    }
}

/// Common `SRenderNodeChunk` fields shared by compiled object records.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderNodeCommon {
    pub world_bounds: Aabb3d,
    pub layer_id: u16,
    pub shadow_lod_bias: i8,
    pub render_flags: u32,
    pub object_type_index: u16,
    pub view_distance_multiplier: f32,
    pub lod_ratio: u8,
}

/// One object record in a compiled object block.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderNodeObject<'a> {
    Brush(BrushChunk),
    Vegetation(VegetationChunk),
    MergedMesh(MergedMeshChunk<'a>),
    Road(RoadChunk<'a>),
    Decal(DecalChunk),
    WaterVolume(WaterVolumeChunk<'a>),
    DistanceCloud(DistanceCloudChunk),
}

impl RenderNodeObject<'_> {
    #[must_use]
    pub const fn node_type(&self) -> RenderNodeType {
        match self {
            Self::Brush(_) => RenderNodeType::Brush,
            Self::Vegetation(_) => RenderNodeType::Vegetation,
            Self::MergedMesh(_) => RenderNodeType::MergedMesh,
            Self::Road(_) => RenderNodeType::Road,
            Self::Decal(_) => RenderNodeType::Decal,
            Self::WaterVolume(_) => RenderNodeType::WaterVolume,
            Self::DistanceCloud(_) => RenderNodeType::DistanceCloud,
        }
    }
}

/// `SBrushChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushChunk {
    pub common: RenderNodeCommon,
    pub transform: Affine3A,
    pub collision_class_index: i16,
    pub flags: u16,
    pub material_id: i32,
    pub material_layers: i32,
}

/// `SVegetationChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VegetationChunk {
    pub common: RenderNodeCommon,
    pub position: Vec3A,
    pub scale: f32,
    pub brightness: u8,
    pub angle: u8,
    pub angle_x: Option<u8>,
    pub angle_y: Option<u8>,
}

/// `SMergedMeshChunk` plus its borrowed `SMergedMeshGroupChunk` table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MergedMeshChunk<'a> {
    pub common: RenderNodeCommon,
    pub extents: Aabb3d,
    groups: MergedMeshGroups<'a>,
}

impl<'a> MergedMeshChunk<'a> {
    #[inline]
    #[must_use]
    pub const fn groups(self) -> MergedMeshGroups<'a> {
        self.groups
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedMeshGroups<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl MergedMeshGroups<'_> {
    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len() / MERGED_MESH_GROUP_CHUNK_SIZE
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<MergedMeshGroup> {
        let offset = index.checked_mul(MERGED_MESH_GROUP_CHUNK_SIZE)?;
        let bytes = self
            .bytes
            .get(offset..offset + MERGED_MESH_GROUP_CHUNK_SIZE)?;
        Some(MergedMeshGroup {
            stat_inst_group_id: self.endian.read_u32(bytes, 0).ok()?,
            samples: self.endian.read_u32(bytes, 4).ok()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedMeshGroup {
    pub stat_inst_group_id: u32,
    pub samples: u32,
}

/// `SRoadChunk` plus its borrowed vertex table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadChunk<'a> {
    pub common: RenderNodeCommon,
    pub sort_priority: i16,
    pub flags: i16,
    pub material_id: i32,
    pub tex_coords: [f32; 2],
    pub tex_coords_global: [f32; 2],
    vertices: PackedVec3Records<'a>,
}

impl<'a> RoadChunk<'a> {
    #[inline]
    #[must_use]
    pub const fn vertices(self) -> PackedVec3Records<'a> {
        self.vertices
    }
}

/// `SDecalChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecalChunk {
    pub common: RenderNodeCommon,
    pub projection_type: i16,
    pub deferred: bool,
    pub depth: f32,
    pub position: Vec3A,
    pub normal: Vec3A,
    pub explicit_right_up_front: Mat3A,
    pub radius: f32,
    pub material_id: i32,
    pub sort_priority: i32,
}

/// `SWaterVolumeChunk` plus borrowed auxiliary and vertex arrays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVolumeChunk<'a> {
    pub common: RenderNodeCommon,
    pub volume_type: u16,
    pub cap_fog_at_volume_depth: bool,
    pub fog_color_affected_by_sun: bool,
    pub volume_id: u64,
    pub material_id: i32,
    pub fog_density: f32,
    pub fog_color: Vec3A,
    pub fog_plane: Vec4,
    pub fog_shadowing: f32,
    pub caustics: bool,
    pub caustic_intensity: f32,
    pub caustic_tiling: f32,
    pub caustic_height: f32,
    pub tex_coord_begin: f32,
    pub tex_coord_end: f32,
    pub surface_u_scale: f32,
    pub surface_v_scale: f32,
    pub volume_depth: f32,
    pub stream_speed: f32,
    aux_values: PackedF32Records<'a>,
    vertices: PackedVec3Records<'a>,
    physics_area_contour: PackedVec3Records<'a>,
}

impl<'a> WaterVolumeChunk<'a> {
    #[inline]
    #[must_use]
    pub const fn aux_values(self) -> PackedF32Records<'a> {
        self.aux_values
    }

    #[inline]
    #[must_use]
    pub const fn vertices(self) -> PackedVec3Records<'a> {
        self.vertices
    }

    #[inline]
    #[must_use]
    pub const fn physics_area_contour(self) -> PackedVec3Records<'a> {
        self.physics_area_contour
    }
}

/// `SDistanceCloudChunk`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceCloudChunk {
    pub common: RenderNodeCommon,
    pub position: Vec3A,
    pub size_x: f32,
    pub size_y: f32,
    pub rotation_z: f32,
    pub material_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedVec3Records<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl PackedVec3Records<'_> {
    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len() / VEC3_CHUNK_SIZE
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<Vec3A> {
        let offset = index.checked_mul(VEC3_CHUNK_SIZE)?;
        self.endian.read_vec3a(self.bytes, offset).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedF32Records<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl PackedF32Records<'_> {
    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len() / F32_CHUNK_SIZE
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<f32> {
        let offset = index.checked_mul(F32_CHUNK_SIZE)?;
        self.endian.read_f32(self.bytes, offset).ok()
    }
}

fn read_render_node_object<'a>(
    cursor: &mut Cursor<'a>,
    endian: Endian,
    node_version: i16,
) -> Result<RenderNodeObject<'a>, ParseError> {
    let type_offset = cursor.position();
    let node_type = RenderNodeType::read(cursor.read_i32(endian)?, type_offset)?;
    match node_type {
        RenderNodeType::Brush => read_brush(cursor, endian).map(RenderNodeObject::Brush),
        RenderNodeType::Vegetation => {
            read_vegetation(cursor, endian, node_version).map(RenderNodeObject::Vegetation)
        }
        RenderNodeType::MergedMesh => {
            read_merged_mesh(cursor, endian).map(RenderNodeObject::MergedMesh)
        }
        RenderNodeType::Road => read_road(cursor, endian).map(RenderNodeObject::Road),
        RenderNodeType::Decal => read_decal(cursor, endian).map(RenderNodeObject::Decal),
        RenderNodeType::WaterVolume => {
            read_water_volume(cursor, endian).map(RenderNodeObject::WaterVolume)
        }
        RenderNodeType::DistanceCloud => {
            read_distance_cloud(cursor, endian).map(RenderNodeObject::DistanceCloud)
        }
    }
}

fn read_common(bytes: &[u8], endian: Endian) -> Result<RenderNodeCommon, ParseError> {
    Ok(RenderNodeCommon {
        world_bounds: endian.read_aabb3d(bytes, 0)?,
        layer_id: endian.read_u16(bytes, 24)?,
        shadow_lod_bias: bytes[26].cast_signed(),
        render_flags: endian.read_u32(bytes, 28)?,
        object_type_index: endian.read_u16(bytes, 32)?,
        view_distance_multiplier: endian.read_f32(bytes, 36)?,
        lod_ratio: bytes[40],
    })
}

fn read_brush(cursor: &mut Cursor<'_>, endian: Endian) -> Result<BrushChunk, ParseError> {
    let chunk = cursor.read_bytes(BRUSH_CHUNK_SIZE)?;
    Ok(BrushChunk {
        common: read_common(chunk, endian)?,
        transform: endian.read_matrix34(chunk, 44)?,
        collision_class_index: endian.read_i16(chunk, 92)?,
        flags: endian.read_u16(chunk, 94)?,
        material_id: endian.read_i32(chunk, 96)?,
        material_layers: endian.read_i32(chunk, 100)?,
    })
}

fn read_vegetation(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    node_version: i16,
) -> Result<VegetationChunk, ParseError> {
    let chunk = cursor.read_bytes(VEGETATION_CHUNK_SIZE)?;
    let is_old = node_version == OCTREE_NODE_CHUNK_VERSION_OLD;
    Ok(VegetationChunk {
        common: read_common(chunk, endian)?,
        position: endian.read_vec3a(chunk, 44)?,
        scale: endian.read_f32(chunk, 56)?,
        brightness: chunk[60],
        angle: chunk[61],
        angle_x: (!is_old).then_some(chunk[62]),
        angle_y: (!is_old).then_some(chunk[63]),
    })
}

fn read_merged_mesh<'a>(
    cursor: &mut Cursor<'a>,
    endian: Endian,
) -> Result<MergedMeshChunk<'a>, ParseError> {
    let chunk = cursor.read_bytes(MERGED_MESH_CHUNK_SIZE)?;
    let group_count = endian.read_u32(chunk, 44)? as usize;
    let groups = cursor.read_bytes(checked_record_bytes(
        group_count,
        MERGED_MESH_GROUP_CHUNK_SIZE,
    )?)?;
    Ok(MergedMeshChunk {
        common: read_common(chunk, endian)?,
        extents: endian.read_aabb3d(chunk, 48)?,
        groups: MergedMeshGroups {
            bytes: groups,
            endian,
        },
    })
}

fn read_road<'a>(cursor: &mut Cursor<'a>, endian: Endian) -> Result<RoadChunk<'a>, ParseError> {
    let chunk = cursor.read_bytes(ROAD_CHUNK_SIZE)?;
    let vertex_count = read_non_negative_count("SRoadChunk.m_nVertsNum", endian, chunk, 44)?;
    let vertices = cursor.read_bytes(checked_record_bytes(vertex_count, VEC3_CHUNK_SIZE)?)?;
    Ok(RoadChunk {
        common: read_common(chunk, endian)?,
        sort_priority: endian.read_i16(chunk, 48)?,
        flags: endian.read_i16(chunk, 50)?,
        material_id: endian.read_i32(chunk, 52)?,
        tex_coords: [endian.read_f32(chunk, 56)?, endian.read_f32(chunk, 60)?],
        tex_coords_global: [endian.read_f32(chunk, 64)?, endian.read_f32(chunk, 68)?],
        vertices: PackedVec3Records {
            bytes: vertices,
            endian,
        },
    })
}

fn read_decal(cursor: &mut Cursor<'_>, endian: Endian) -> Result<DecalChunk, ParseError> {
    let chunk = cursor.read_bytes(DECAL_CHUNK_SIZE)?;
    Ok(DecalChunk {
        common: read_common(chunk, endian)?,
        projection_type: endian.read_i16(chunk, 44)?,
        deferred: chunk[46] != 0,
        depth: endian.read_f32(chunk, 48)?,
        position: endian.read_vec3a(chunk, 52)?,
        normal: endian.read_vec3a(chunk, 64)?,
        explicit_right_up_front: endian.read_matrix33(chunk, 76)?,
        radius: endian.read_f32(chunk, 112)?,
        material_id: endian.read_i32(chunk, 116)?,
        sort_priority: endian.read_i32(chunk, 120)?,
    })
}

fn read_water_volume<'a>(
    cursor: &mut Cursor<'a>,
    endian: Endian,
) -> Result<WaterVolumeChunk<'a>, ParseError> {
    let chunk = cursor.read_bytes(WATER_VOLUME_CHUNK_SIZE)?;
    let volume_type_and_misc_bits = endian.read_u32(chunk, 44)?;
    let aux_count = (volume_type_and_misc_bits >> 24) as usize;
    let vertex_count = endian.read_u32(chunk, 128)? as usize;
    let physics_vertex_count = endian.read_u32(chunk, 140)? as usize;
    let aux_values = cursor.read_bytes(checked_record_bytes(aux_count, F32_CHUNK_SIZE)?)?;
    let vertices = cursor.read_bytes(checked_record_bytes(vertex_count, VEC3_CHUNK_SIZE)?)?;
    let physics_area_contour =
        cursor.read_bytes(checked_record_bytes(physics_vertex_count, VEC3_CHUNK_SIZE)?)?;

    Ok(WaterVolumeChunk {
        common: read_common(chunk, endian)?,
        volume_type: (volume_type_and_misc_bits & 0xffff) as u16,
        cap_fog_at_volume_depth: volume_type_and_misc_bits & 0x10000 != 0,
        fog_color_affected_by_sun: volume_type_and_misc_bits & 0x20000 == 0,
        volume_id: endian.read_u64(chunk, 48)?,
        material_id: endian.read_i32(chunk, 56)?,
        fog_density: endian.read_f32(chunk, 60)?,
        fog_color: endian.read_vec3a(chunk, 64)?,
        fog_plane: endian.read_vec4(chunk, 76)?,
        fog_shadowing: endian.read_f32(chunk, 92)?,
        caustics: chunk[96] != 0,
        caustic_intensity: endian.read_f32(chunk, 100)?,
        caustic_tiling: endian.read_f32(chunk, 104)?,
        caustic_height: endian.read_f32(chunk, 108)?,
        tex_coord_begin: endian.read_f32(chunk, 112)?,
        tex_coord_end: endian.read_f32(chunk, 116)?,
        surface_u_scale: endian.read_f32(chunk, 120)?,
        surface_v_scale: endian.read_f32(chunk, 124)?,
        volume_depth: endian.read_f32(chunk, 132)?,
        stream_speed: endian.read_f32(chunk, 136)?,
        aux_values: PackedF32Records {
            bytes: aux_values,
            endian,
        },
        vertices: PackedVec3Records {
            bytes: vertices,
            endian,
        },
        physics_area_contour: PackedVec3Records {
            bytes: physics_area_contour,
            endian,
        },
    })
}

fn read_distance_cloud(
    cursor: &mut Cursor<'_>,
    endian: Endian,
) -> Result<DistanceCloudChunk, ParseError> {
    let chunk = cursor.read_bytes(DISTANCE_CLOUD_CHUNK_SIZE)?;
    Ok(DistanceCloudChunk {
        common: read_common(chunk, endian)?,
        position: endian.read_vec3a(chunk, 44)?,
        size_x: endian.read_f32(chunk, 56)?,
        size_y: endian.read_f32(chunk, 60)?,
        rotation_z: endian.read_f32(chunk, 64)?,
        material_id: endian.read_i32(chunk, 68)?,
    })
}

fn read_non_negative_count(
    field: &'static str,
    endian: Endian,
    bytes: &[u8],
    offset: usize,
) -> Result<usize, ParseError> {
    let count = endian.read_i32(bytes, offset)?;
    usize::try_from(count).map_err(|_| ParseError::InvalidCount { field, count })
}

fn checked_record_bytes(count: usize, record_size: usize) -> Result<usize, ParseError> {
    count
        .checked_mul(record_size)
        .ok_or(ParseError::IntegerOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_block_iter_reads_brush_record() {
        let mut bytes = Vec::new();
        bytes.extend((RenderNodeType::Brush as i32).to_le_bytes());
        bytes.resize(bytes.len() + BRUSH_CHUNK_SIZE, 0);

        let block = ObjectBlock::new(&bytes, Endian::Little, OCTREE_NODE_CHUNK_VERSION);
        let objects = block.iter().collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(block.validate().unwrap(), 1);
        assert_eq!(objects[0].node_type(), RenderNodeType::Brush);
    }

    #[test]
    fn object_block_iter_reads_road_vertices() {
        let mut bytes = Vec::new();
        bytes.extend((RenderNodeType::Road as i32).to_le_bytes());
        let chunk_offset = bytes.len();
        bytes.resize(chunk_offset + ROAD_CHUNK_SIZE, 0);
        bytes[chunk_offset + 44..chunk_offset + 48].copy_from_slice(&2i32.to_le_bytes());
        push_vec3(&mut bytes, [1.0, 2.0, 3.0]);
        push_vec3(&mut bytes, [4.0, 5.0, 6.0]);

        let block = ObjectBlock::new(&bytes, Endian::Little, OCTREE_NODE_CHUNK_VERSION);
        let object = block.iter().next().unwrap().unwrap();

        let RenderNodeObject::Road(road) = object else {
            panic!("expected road object");
        };
        assert_eq!(road.vertices().len(), 2);
        assert_eq!(road.vertices().get(1), Some(Vec3A::new(4.0, 5.0, 6.0)));
    }

    #[test]
    fn object_block_rejects_unknown_render_node_type() {
        let bytes = 99i32.to_le_bytes();
        let block = ObjectBlock::new(&bytes, Endian::Little, OCTREE_NODE_CHUNK_VERSION);

        assert!(matches!(
            block.iter().next().unwrap(),
            Err(ParseError::UnsupportedRenderNodeType {
                offset: 0,
                value: 99
            })
        ));
    }

    fn push_vec3(bytes: &mut Vec<u8>, values: [f32; 3]) {
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
    }
}
