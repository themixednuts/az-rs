use thiserror::Error;

use crate::{Chunk, ChunkType};

const NODE_HEADER_LEN: usize = 204;
const MESH_0801_LEN: usize = 264;
const MESH_0802_LEN: usize = 712;
const DATA_STREAM_0800_LEN: usize = 24;
const DATA_STREAM_0801_LEN: usize = 28;
const MESH_SUBSETS_HEADER_LEN: usize = 16;
const MESH_SUBSET_LEN: usize = 36;
const MESH_BONE_IDS_LEN: usize = 260;
const MATERIAL_NAME_0800_LEN: usize = 408;
const MATERIAL_NAME_0802_LEN: usize = 132;
const COMPILED_BONES_HEADER_LEN: usize = 32;
const COMPILED_BONE_LEN: usize = 584;
const STREAM_TYPE_COUNT: usize = 16;
const STREAM_INDEX_COUNT: usize = 8;
const MAX_SUB_MATERIALS_0800: usize = 32;
const MAX_SUB_MATERIALS_0802: usize = 128;

/// A decoded payload in the geometry conversion subset.
#[derive(Debug, Clone, PartialEq)]
pub enum SupportedChunkPayload<'a> {
    Node(NodeChunk<'a>),
    Mesh(Box<MeshChunk>),
    MeshSubsets(MeshSubsetsChunk),
    DataStream(DataStreamChunk<'a>),
    MaterialName(MaterialNameChunk),
    CompiledBones(CompiledBonesChunk),
}

/// Node version 0x0823 or 0x0824.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeChunk<'a> {
    pub name: String,
    pub object_chunk_id: i32,
    pub parent_chunk_id: i32,
    pub child_count: u32,
    pub material_chunk_id: i32,
    pub transform: [[f32; 4]; 4],
    pub position_controller_id: i32,
    pub rotation_controller_id: i32,
    pub scale_controller_id: i32,
    pub properties: &'a [u8],
}

/// Compiled mesh version 0x0800, 0x0801, or 0x0802.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshChunk {
    pub flags: i32,
    pub physicalize_flags: i32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub subset_count: u32,
    pub subsets_chunk_id: i32,
    pub vertex_animation_chunk_id: i32,
    pub stream_chunk_ids: [[i32; STREAM_INDEX_COUNT]; STREAM_TYPE_COUNT],
    pub physics_data_chunk_ids: [i32; 4],
    pub bounding_box_min: [f32; 3],
    pub bounding_box_max: [f32; 3],
    pub texture_mapping_density: f32,
    pub geometric_mean_face_area: f32,
}

/// Public `ECgfStreamType` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MeshStreamType {
    Positions = 0,
    Normals = 1,
    TextureCoordinates = 2,
    Colors = 3,
    SecondaryColors = 4,
    Indices = 5,
    Tangents = 6,
    LegacySphericalHarmonics = 7,
    LegacyShapeDeformation = 8,
    BoneMapping = 9,
    FaceMap = 10,
    VertexMaterials = 11,
    QuaternionTangents = 12,
    SkinData = 13,
    LegacyConsole = 14,
    InterleavedPositionColorUv = 15,
}

impl MeshStreamType {
    /// Convert a public stream-type integer.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            0 => Self::Positions,
            1 => Self::Normals,
            2 => Self::TextureCoordinates,
            3 => Self::Colors,
            4 => Self::SecondaryColors,
            5 => Self::Indices,
            6 => Self::Tangents,
            7 => Self::LegacySphericalHarmonics,
            8 => Self::LegacyShapeDeformation,
            9 => Self::BoneMapping,
            10 => Self::FaceMap,
            11 => Self::VertexMaterials,
            12 => Self::QuaternionTangents,
            13 => Self::SkinData,
            14 => Self::LegacyConsole,
            15 => Self::InterleavedPositionColorUv,
            _ => return None,
        })
    }
}

/// Data stream version 0x0800 or 0x0801.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataStreamChunk<'a> {
    pub flags: i32,
    pub stream_type: MeshStreamType,
    pub stream_index: u32,
    pub element_count: u32,
    pub element_size: u32,
    pub data: &'a [u8],
    pub data_is_big_endian: bool,
}

/// Mesh-subset version 0x0800.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSubsetsChunk {
    pub flags: i32,
    pub subsets: Vec<MeshSubset>,
}

/// One material and index range in a mesh-subsets chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSubset {
    pub first_index: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub material_id: i32,
    pub radius: f32,
    pub center: [f32; 3],
    pub bone_ids: Option<Vec<u16>>,
}

/// How a material-name chunk refers to sub-materials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialChildren {
    ChunkIds(Vec<i32>),
    Names(Vec<String>),
}

/// Material-name version 0x0800 or 0x0802.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialNameChunk {
    pub name: String,
    pub flags: Option<[i32; 2]>,
    pub physicalize_types: Vec<i32>,
    pub children: MaterialChildren,
}

/// Compiled-bones version 0x0800.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledBonesChunk {
    pub bones: Vec<CompiledBone>,
}

/// Skeleton fields needed to construct a neutral scene hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledBone {
    pub controller_id: u32,
    pub mass: f32,
    pub world_to_bone: [[f32; 4]; 3],
    pub bone_to_world: [[f32; 4]; 3],
    pub name: String,
    pub limb_id: i32,
    pub parent_offset: i32,
    pub child_count: u32,
    pub children_offset: i32,
}

/// Error returned by a selected payload decoder.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PayloadError {
    #[error("unsupported {chunk_type:?} payload version {version:#06x}")]
    UnsupportedVersion { chunk_type: ChunkType, version: u16 },
    #[error("unexpected end of {context} payload at byte {offset}")]
    UnexpectedEof {
        context: &'static str,
        offset: usize,
    },
    #[error("negative {field} value {value} in {context}")]
    NegativeCount {
        context: &'static str,
        field: &'static str,
        value: i32,
    },
    #[error("{field} value {value} is too large for {context}")]
    CountTooLarge {
        context: &'static str,
        field: &'static str,
        value: u32,
    },
    #[error("invalid stream type {value}")]
    InvalidStreamType { value: i32 },
    #[error("unsupported mesh-subset flags {flags:#x}")]
    UnsupportedMeshSubsetFlags { flags: i32 },
    #[error("invalid UTF-8 in {field}")]
    InvalidUtf8 { field: &'static str },
    #[error("missing NUL terminator in {field}")]
    MissingNul { field: &'static str },
    #[error("{context} has {actual} bytes; expected {expected}")]
    InvalidSize {
        context: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("nonzero trailing bytes in {context}")]
    NonzeroTrailingBytes { context: &'static str },
}

pub fn decode(chunk: Chunk<'_>) -> Result<Option<SupportedChunkPayload<'_>>, PayloadError> {
    let header = chunk.header();
    let Some(chunk_type) = header.chunk_type() else {
        return Ok(None);
    };
    let payload = chunk.payload();
    let big_endian = header.is_big_endian();
    let decoded =
        match chunk_type {
            ChunkType::Node => {
                SupportedChunkPayload::Node(decode_node(payload, header.version(), big_endian)?)
            }
            ChunkType::Mesh => SupportedChunkPayload::Mesh(Box::new(decode_mesh(
                payload,
                header.version(),
                big_endian,
            )?)),
            ChunkType::MeshSubsets => SupportedChunkPayload::MeshSubsets(decode_mesh_subsets(
                payload,
                header.version(),
                big_endian,
            )?),
            ChunkType::DataStream => SupportedChunkPayload::DataStream(decode_data_stream(
                payload,
                header.version(),
                big_endian,
            )?),
            ChunkType::MaterialName => SupportedChunkPayload::MaterialName(decode_material_name(
                payload,
                header.version(),
                big_endian,
            )?),
            ChunkType::CompiledBones => SupportedChunkPayload::CompiledBones(
                decode_compiled_bones(payload, header.version(), big_endian)?,
            ),
            _ => return Ok(None),
        };
    Ok(Some(decoded))
}

fn decode_node(
    payload: &[u8],
    version: u16,
    big_endian: bool,
) -> Result<NodeChunk<'_>, PayloadError> {
    require_version(ChunkType::Node, version, &[0x0823, 0x0824])?;
    require_min_len(payload, NODE_HEADER_LEN, "node")?;
    let mut reader = Reader::new(payload, big_endian, "node");
    let name = reader.fixed_string(64, "node name")?;
    let object_chunk_id = reader.i32()?;
    let parent_chunk_id = reader.i32()?;
    let child_count = nonnegative(reader.i32()?, "node", "child count")?;
    let material_chunk_id = reader.i32()?;
    reader.skip(4)?;
    let transform = reader.matrix::<4, 4>()?;
    reader.skip(40)?;
    let position_controller_id = reader.i32()?;
    let rotation_controller_id = reader.i32()?;
    let scale_controller_id = reader.i32()?;
    let property_len = count_usize(reader.i32()?, "node", "property length")?;
    let properties = reader.bytes(property_len)?;
    reader.finish_zero_padding()?;
    Ok(NodeChunk {
        name,
        object_chunk_id,
        parent_chunk_id,
        child_count,
        material_chunk_id,
        transform,
        position_controller_id,
        rotation_controller_id,
        scale_controller_id,
        properties,
    })
}

fn decode_mesh(payload: &[u8], version: u16, big_endian: bool) -> Result<MeshChunk, PayloadError> {
    require_version(ChunkType::Mesh, version, &[0x0800, 0x0801, 0x0802])?;
    let expected = if version == 0x0802 {
        MESH_0802_LEN
    } else {
        MESH_0801_LEN
    };
    require_exact_len(payload, expected, "mesh")?;
    let mut reader = Reader::new(payload, big_endian, "mesh");
    let flags = reader.i32()?;
    let physicalize_flags = reader.i32()?;
    let vertex_count = nonnegative(reader.i32()?, "mesh", "vertex count")?;
    let index_count = nonnegative(reader.i32()?, "mesh", "index count")?;
    let subset_count = nonnegative(reader.i32()?, "mesh", "subset count")?;
    let subsets_chunk_id = reader.i32()?;
    let vertex_animation_chunk_id = reader.i32()?;
    let mut stream_chunk_ids = [[-1; STREAM_INDEX_COUNT]; STREAM_TYPE_COUNT];
    if version == 0x0802 {
        for ids in &mut stream_chunk_ids {
            for id in ids {
                *id = reader.i32()?;
            }
        }
    } else {
        for ids in &mut stream_chunk_ids {
            ids[0] = reader.i32()?;
        }
    }
    let mut physics_data_chunk_ids = [0; 4];
    for id in &mut physics_data_chunk_ids {
        *id = reader.i32()?;
    }
    let bounding_box_min = reader.array_f32()?;
    let bounding_box_max = reader.array_f32()?;
    let texture_mapping_density = reader.f32()?;
    let geometric_mean_face_area = reader.f32()?;
    reader.skip(31 * 4)?;
    reader.finish_zero_padding()?;
    Ok(MeshChunk {
        flags,
        physicalize_flags,
        vertex_count,
        index_count,
        subset_count,
        subsets_chunk_id,
        vertex_animation_chunk_id,
        stream_chunk_ids,
        physics_data_chunk_ids,
        bounding_box_min,
        bounding_box_max,
        texture_mapping_density,
        geometric_mean_face_area,
    })
}

fn decode_data_stream(
    payload: &[u8],
    version: u16,
    big_endian: bool,
) -> Result<DataStreamChunk<'_>, PayloadError> {
    require_version(ChunkType::DataStream, version, &[0x0800, 0x0801])?;
    let header_len = if version == 0x0800 {
        DATA_STREAM_0800_LEN
    } else {
        DATA_STREAM_0801_LEN
    };
    require_min_len(payload, header_len, "data stream")?;
    let mut reader = Reader::new(payload, big_endian, "data stream");
    let flags = reader.i32()?;
    let raw_stream_type = reader.i32()?;
    let stream_type =
        MeshStreamType::from_raw(raw_stream_type).ok_or(PayloadError::InvalidStreamType {
            value: raw_stream_type,
        })?;
    let stream_index = if version == 0x0801 {
        nonnegative(reader.i32()?, "data stream", "stream index")?
    } else {
        0
    };
    if usize::try_from(stream_index).map_or(true, |index| index >= STREAM_INDEX_COUNT) {
        return Err(PayloadError::CountTooLarge {
            context: "data stream",
            field: "stream index",
            value: stream_index,
        });
    }
    let element_count = nonnegative(reader.i32()?, "data stream", "element count")?;
    let element_size = nonnegative(reader.i32()?, "data stream", "element size")?;
    reader.skip(8)?;
    let data_len = usize::try_from(element_count)
        .ok()
        .and_then(|count| {
            usize::try_from(element_size)
                .ok()
                .and_then(|size| count.checked_mul(size))
        })
        .ok_or(PayloadError::CountTooLarge {
            context: "data stream",
            field: "element byte length",
            value: element_count,
        })?;
    let data = reader.bytes(data_len)?;
    reader.finish_zero_padding()?;
    Ok(DataStreamChunk {
        flags,
        stream_type,
        stream_index,
        element_count,
        element_size,
        data,
        data_is_big_endian: big_endian,
    })
}

fn decode_mesh_subsets(
    payload: &[u8],
    version: u16,
    big_endian: bool,
) -> Result<MeshSubsetsChunk, PayloadError> {
    require_version(ChunkType::MeshSubsets, version, &[0x0800])?;
    require_min_len(payload, MESH_SUBSETS_HEADER_LEN, "mesh subsets")?;
    let mut reader = Reader::new(payload, big_endian, "mesh subsets");
    let flags = reader.i32()?;
    if flags & !0x3 != 0 {
        return Err(PayloadError::UnsupportedMeshSubsetFlags { flags });
    }
    let count = count_usize(reader.i32()?, "mesh subsets", "subset count")?;
    reader.skip(8)?;
    let suffix_per_subset = if flags & 0x2 != 0 {
        MESH_BONE_IDS_LEN
    } else {
        0
    };
    let expected = MESH_SUBSETS_HEADER_LEN
        .checked_add(
            count
                .checked_mul(MESH_SUBSET_LEN + suffix_per_subset)
                .ok_or_else(|| PayloadError::CountTooLarge {
                    context: "mesh subsets",
                    field: "subset count",
                    value: u32::try_from(count).unwrap_or(u32::MAX),
                })?,
        )
        .ok_or_else(|| PayloadError::CountTooLarge {
            context: "mesh subsets",
            field: "subset count",
            value: u32::try_from(count).unwrap_or(u32::MAX),
        })?;
    require_exact_len(payload, expected, "mesh subsets")?;
    let mut subsets = Vec::with_capacity(count);
    for _ in 0..count {
        subsets.push(MeshSubset {
            first_index: nonnegative(reader.i32()?, "mesh subsets", "first index")?,
            index_count: nonnegative(reader.i32()?, "mesh subsets", "index count")?,
            first_vertex: nonnegative(reader.i32()?, "mesh subsets", "first vertex")?,
            vertex_count: nonnegative(reader.i32()?, "mesh subsets", "vertex count")?,
            material_id: reader.i32()?,
            radius: reader.f32()?,
            center: reader.array_f32()?,
            bone_ids: None,
        });
    }
    if flags & 0x2 != 0 {
        for subset in &mut subsets {
            let bone_count =
                usize::try_from(reader.u32()?).map_err(|_| PayloadError::CountTooLarge {
                    context: "mesh subsets",
                    field: "bone id count",
                    value: u32::MAX,
                })?;
            if bone_count > 128 {
                return Err(PayloadError::CountTooLarge {
                    context: "mesh subsets",
                    field: "bone id count",
                    value: u32::try_from(bone_count).unwrap_or(u32::MAX),
                });
            }
            let mut ids = Vec::with_capacity(bone_count);
            for index in 0..128 {
                let id = reader.u16()?;
                if index < bone_count {
                    ids.push(id);
                }
            }
            subset.bone_ids = Some(ids);
        }
    }
    reader.finish_zero_padding()?;
    Ok(MeshSubsetsChunk { flags, subsets })
}

fn decode_material_name(
    payload: &[u8],
    version: u16,
    big_endian: bool,
) -> Result<MaterialNameChunk, PayloadError> {
    require_version(ChunkType::MaterialName, version, &[0x0800, 0x0802])?;
    if version == 0x0800 {
        require_exact_len(payload, MATERIAL_NAME_0800_LEN, "material name")?;
        let mut reader = Reader::new(payload, big_endian, "material name");
        let flags = [reader.i32()?, reader.i32()?];
        let name = reader.fixed_string(128, "material name")?;
        let physicalize_type = reader.i32()?;
        let child_count = count_usize(reader.i32()?, "material name", "sub-material count")?;
        if child_count > MAX_SUB_MATERIALS_0800 {
            return Err(PayloadError::CountTooLarge {
                context: "material name",
                field: "sub-material count",
                value: u32::try_from(child_count).unwrap_or(u32::MAX),
            });
        }
        let mut child_ids = Vec::with_capacity(child_count);
        for index in 0..MAX_SUB_MATERIALS_0800 {
            let id = reader.i32()?;
            if index < child_count {
                child_ids.push(id);
            }
        }
        reader.skip(4 + 4 + 32 * 4)?;
        reader.finish_zero_padding()?;
        return Ok(MaterialNameChunk {
            name,
            flags: Some(flags),
            physicalize_types: vec![physicalize_type],
            children: MaterialChildren::ChunkIds(child_ids),
        });
    }

    require_min_len(payload, MATERIAL_NAME_0802_LEN + 4, "material name")?;
    let mut reader = Reader::new(payload, big_endian, "material name");
    let name = reader.fixed_string(128, "material name")?;
    let raw_count = reader.i32()?;
    let child_count = if raw_count <= 0 {
        0
    } else {
        usize::try_from(raw_count).map_err(|_| PayloadError::CountTooLarge {
            context: "material name",
            field: "sub-material count",
            value: u32::MAX,
        })?
    };
    if child_count > MAX_SUB_MATERIALS_0802 {
        return Err(PayloadError::CountTooLarge {
            context: "material name",
            field: "sub-material count",
            value: u32::try_from(child_count).unwrap_or(u32::MAX),
        });
    }
    let slot_count = child_count.max(1);
    let mut physicalize_types = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        physicalize_types.push(reader.i32()?);
    }
    let mut names = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        names.push(reader.c_string("sub-material name")?);
    }
    reader.finish_zero_padding()?;
    Ok(MaterialNameChunk {
        name,
        flags: None,
        physicalize_types,
        children: MaterialChildren::Names(names),
    })
}

fn decode_compiled_bones(
    payload: &[u8],
    version: u16,
    big_endian: bool,
) -> Result<CompiledBonesChunk, PayloadError> {
    require_version(ChunkType::CompiledBones, version, &[0x0800])?;
    require_min_len(payload, COMPILED_BONES_HEADER_LEN, "compiled bones")?;
    let bone_bytes = payload.len() - COMPILED_BONES_HEADER_LEN;
    if !bone_bytes.is_multiple_of(COMPILED_BONE_LEN) {
        return Err(PayloadError::InvalidSize {
            context: "compiled bones",
            expected: COMPILED_BONES_HEADER_LEN
                + bone_bytes / COMPILED_BONE_LEN * COMPILED_BONE_LEN,
            actual: payload.len(),
        });
    }
    let count = bone_bytes / COMPILED_BONE_LEN;
    let mut reader = Reader::new(payload, big_endian, "compiled bones");
    reader.skip(COMPILED_BONES_HEADER_LEN)?;
    let mut bones = Vec::with_capacity(count);
    for _ in 0..count {
        let controller_id = reader.u32()?;
        reader.skip(208)?;
        let mass = reader.f32()?;
        let world_to_bone = reader.matrix::<3, 4>()?;
        let bone_to_world = reader.matrix::<3, 4>()?;
        let name = reader.fixed_string(256, "bone name")?;
        let limb_id = reader.i32()?;
        let parent_offset = reader.i32()?;
        let child_count = reader.u32()?;
        let children_offset = reader.i32()?;
        bones.push(CompiledBone {
            controller_id,
            mass,
            world_to_bone,
            bone_to_world,
            name,
            limb_id,
            parent_offset,
            child_count,
            children_offset,
        });
    }
    reader.finish_zero_padding()?;
    Ok(CompiledBonesChunk { bones })
}

fn require_version(
    chunk_type: ChunkType,
    version: u16,
    versions: &[u16],
) -> Result<(), PayloadError> {
    if versions.contains(&version) {
        Ok(())
    } else {
        Err(PayloadError::UnsupportedVersion {
            chunk_type,
            version,
        })
    }
}

const fn require_min_len(
    payload: &[u8],
    minimum: usize,
    context: &'static str,
) -> Result<(), PayloadError> {
    if payload.len() >= minimum {
        Ok(())
    } else {
        Err(PayloadError::UnexpectedEof {
            context,
            offset: payload.len(),
        })
    }
}

const fn require_exact_len(
    payload: &[u8],
    expected: usize,
    context: &'static str,
) -> Result<(), PayloadError> {
    if payload.len() == expected {
        Ok(())
    } else {
        Err(PayloadError::InvalidSize {
            context,
            expected,
            actual: payload.len(),
        })
    }
}

fn nonnegative(
    value: i32,
    context: &'static str,
    field: &'static str,
) -> Result<u32, PayloadError> {
    u32::try_from(value).map_err(|_| PayloadError::NegativeCount {
        context,
        field,
        value,
    })
}

fn count_usize(
    value: i32,
    context: &'static str,
    field: &'static str,
) -> Result<usize, PayloadError> {
    let value = nonnegative(value, context, field)?;
    usize::try_from(value).map_err(|_| PayloadError::CountTooLarge {
        context,
        field,
        value,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    big_endian: bool,
    context: &'static str,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8], big_endian: bool, context: &'static str) -> Self {
        Self {
            bytes,
            offset: 0,
            big_endian,
            context,
        }
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], PayloadError> {
        let start = self.offset;
        let end = start.checked_add(len).ok_or(PayloadError::UnexpectedEof {
            context: self.context,
            offset: start,
        })?;
        let result = self
            .bytes
            .get(start..end)
            .ok_or(PayloadError::UnexpectedEof {
                context: self.context,
                offset: start,
            })?;
        self.offset = end;
        Ok(result)
    }

    fn skip(&mut self, len: usize) -> Result<(), PayloadError> {
        self.bytes(len).map(|_| ())
    }

    fn u16(&mut self) -> Result<u16, PayloadError> {
        let bytes: [u8; 2] =
            self.bytes(2)?
                .try_into()
                .map_err(|_| PayloadError::UnexpectedEof {
                    context: self.context,
                    offset: self.offset,
                })?;
        Ok(if self.big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    }

    fn u32(&mut self) -> Result<u32, PayloadError> {
        let bytes: [u8; 4] =
            self.bytes(4)?
                .try_into()
                .map_err(|_| PayloadError::UnexpectedEof {
                    context: self.context,
                    offset: self.offset,
                })?;
        Ok(if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }

    fn i32(&mut self) -> Result<i32, PayloadError> {
        Ok(i32::from_ne_bytes(self.u32()?.to_ne_bytes()))
    }

    fn f32(&mut self) -> Result<f32, PayloadError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn array_f32<const N: usize>(&mut self) -> Result<[f32; N], PayloadError> {
        let mut values = [0.0; N];
        for value in &mut values {
            *value = self.f32()?;
        }
        Ok(values)
    }

    fn matrix<const R: usize, const C: usize>(&mut self) -> Result<[[f32; C]; R], PayloadError> {
        let mut matrix = [[0.0; C]; R];
        for row in &mut matrix {
            *row = self.array_f32()?;
        }
        Ok(matrix)
    }

    fn fixed_string(&mut self, len: usize, field: &'static str) -> Result<String, PayloadError> {
        let bytes = self.bytes(len)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(PayloadError::MissingNul { field })?;
        decode_string(&bytes[..end], field)
    }

    fn c_string(&mut self, field: &'static str) -> Result<String, PayloadError> {
        let remaining = &self.bytes[self.offset..];
        let len = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(PayloadError::MissingNul { field })?;
        let value = decode_string(&remaining[..len], field)?;
        self.offset += len + 1;
        Ok(value)
    }

    fn finish_zero_padding(self) -> Result<(), PayloadError> {
        if self.bytes[self.offset..].iter().all(|byte| *byte == 0) {
            Ok(())
        } else {
            Err(PayloadError::NonzeroTrailingBytes {
                context: self.context,
            })
        }
    }
}

fn decode_string(bytes: &[u8], field: &'static str) -> Result<String, PayloadError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| PayloadError::InvalidUtf8 { field })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkFile, test_support::cry_file};

    #[test]
    fn decodes_little_endian_node_and_big_endian_data_stream() {
        let mut node = vec![0; NODE_HEADER_LEN];
        node[..5].copy_from_slice(b"root\0");
        put_i32(&mut node, 64, 7, false);
        put_i32(&mut node, 68, -1, false);
        put_i32(&mut node, 72, 2, false);
        put_i32(&mut node, 76, 9, false);
        for index in 0..16 {
            put_f32(
                &mut node,
                84 + index * 4,
                f32::from(u16::try_from(index).unwrap()),
                false,
            );
        }
        put_i32(&mut node, 188, -1, false);
        put_i32(&mut node, 192, -1, false);
        put_i32(&mut node, 196, -1, false);
        put_i32(&mut node, 200, 3, false);
        node.extend_from_slice(b"abc");

        let mut stream = vec![0; DATA_STREAM_0801_LEN];
        put_i32(&mut stream, 4, MeshStreamType::Positions as i32, true);
        put_i32(&mut stream, 8, 1, true);
        put_i32(&mut stream, 12, 1, true);
        put_i32(&mut stream, 16, 4, true);
        stream.extend_from_slice(&1.5_f32.to_be_bytes());

        let file = cry_file(&[
            (ChunkType::Node.raw(), 0x0824, 1, node),
            (ChunkType::DataStream.raw(), 0x8000 | 0x0801, 2, stream),
        ]);
        let mut chunks = ChunkFile::parse(&file).unwrap().chunks();
        let Some(SupportedChunkPayload::Node(node)) =
            chunks.next().unwrap().unwrap().decode_supported().unwrap()
        else {
            panic!("expected node")
        };
        assert_eq!(node.name, "root");
        assert_eq!(node.object_chunk_id, 7);
        assert_eq!(node.child_count, 2);
        assert_eq!(node.transform[3][3].to_bits(), 15.0_f32.to_bits());
        assert_eq!(node.properties, b"abc");

        let Some(SupportedChunkPayload::DataStream(stream)) =
            chunks.next().unwrap().unwrap().decode_supported().unwrap()
        else {
            panic!("expected stream")
        };
        assert_eq!(stream.stream_index, 1);
        assert!(stream.data_is_big_endian);
        assert_eq!(stream.data, &1.5_f32.to_be_bytes());
    }

    #[test]
    fn decodes_mesh_versions_and_rejects_old_inline_mesh() {
        for version in [0x0800, 0x0801] {
            let mut mesh = vec![0; MESH_0801_LEN];
            put_i32(&mut mesh, 8, 3, false);
            put_i32(&mut mesh, 12, 6, false);
            put_i32(&mut mesh, 16, 1, false);
            put_i32(&mut mesh, 20, 22, false);
            put_i32(&mut mesh, 28, 31, false);
            let SupportedChunkPayload::Mesh(mesh) = decode_direct(ChunkType::Mesh, version, mesh)
            else {
                panic!("expected mesh")
            };
            assert_eq!(mesh.vertex_count, 3);
            assert_eq!(mesh.stream_chunk_ids[0][0], 31);
            assert_eq!(mesh.stream_chunk_ids[0][1], -1);
        }

        let mut mesh = vec![0; MESH_0802_LEN];
        put_i32(&mut mesh, 8, 3, true);
        put_i32(&mut mesh, 28 + 4, 33, true);
        let SupportedChunkPayload::Mesh(mesh) =
            decode_direct(ChunkType::Mesh, 0x8000 | 0x0802, mesh)
        else {
            panic!("expected mesh")
        };
        assert_eq!(mesh.vertex_count, 3);
        assert_eq!(mesh.stream_chunk_ids[0][1], 33);

        let error = decode_error(ChunkType::Mesh, 0x0745, vec![0; 18]);
        assert_eq!(
            error,
            PayloadError::UnsupportedVersion {
                chunk_type: ChunkType::Mesh,
                version: 0x0745
            }
        );
    }

    #[test]
    fn decodes_subsets_and_rejects_non_public_flags() {
        let mut payload = vec![0; MESH_SUBSETS_HEADER_LEN + MESH_SUBSET_LEN + MESH_BONE_IDS_LEN];
        put_i32(&mut payload, 0, 0x2, false);
        put_i32(&mut payload, 4, 1, false);
        put_i32(&mut payload, 16, 4, false);
        put_i32(&mut payload, 20, 6, false);
        put_i32(&mut payload, 32, 2, false);
        put_u32(&mut payload, 52, 2, false);
        put_u16(&mut payload, 56, 5, false);
        put_u16(&mut payload, 58, 8, false);
        let SupportedChunkPayload::MeshSubsets(subsets) =
            decode_direct(ChunkType::MeshSubsets, 0x0800, payload)
        else {
            panic!("expected subsets")
        };
        assert_eq!(subsets.subsets[0].bone_ids.as_deref(), Some(&[5, 8][..]));

        for flag in [0x4, 0x8] {
            let mut unsupported_flag = vec![0; MESH_SUBSETS_HEADER_LEN];
            put_i32(&mut unsupported_flag, 0, flag, false);
            assert_eq!(
                decode_error(ChunkType::MeshSubsets, 0x0800, unsupported_flag),
                PayloadError::UnsupportedMeshSubsetFlags { flags: flag }
            );
        }
    }

    #[test]
    fn decodes_big_endian_node_subsets_materials_and_compiled_bones() {
        let mut node = vec![0; NODE_HEADER_LEN];
        node[..5].copy_from_slice(b"root\0");
        put_i32(&mut node, 64, 7, true);
        put_i32(&mut node, 68, -1, true);
        put_i32(&mut node, 72, 2, true);
        put_i32(&mut node, 76, 9, true);
        put_f32(&mut node, 84 + 15 * 4, 1.0, true);
        put_i32(&mut node, 188, -1, true);
        put_i32(&mut node, 192, -1, true);
        put_i32(&mut node, 196, -1, true);
        put_i32(&mut node, 200, 3, true);
        node.extend_from_slice(b"abc");
        let SupportedChunkPayload::Node(node) =
            decode_direct(ChunkType::Node, 0x8000 | 0x0824, node)
        else {
            panic!("expected node")
        };
        assert_eq!(node.object_chunk_id, 7);
        assert_eq!(node.child_count, 2);
        assert_eq!(node.transform[3][3].to_bits(), 1.0_f32.to_bits());
        assert_eq!(node.properties, b"abc");

        let mut subsets = vec![0; MESH_SUBSETS_HEADER_LEN + MESH_SUBSET_LEN + MESH_BONE_IDS_LEN];
        put_i32(&mut subsets, 0, 0x2, true);
        put_i32(&mut subsets, 4, 1, true);
        put_i32(&mut subsets, 16, 4, true);
        put_i32(&mut subsets, 20, 6, true);
        put_i32(&mut subsets, 24, 3, true);
        put_i32(&mut subsets, 28, 8, true);
        put_i32(&mut subsets, 32, 2, true);
        put_u32(&mut subsets, 52, 2, true);
        put_u16(&mut subsets, 56, 5, true);
        put_u16(&mut subsets, 58, 8, true);
        let SupportedChunkPayload::MeshSubsets(subsets) =
            decode_direct(ChunkType::MeshSubsets, 0x8000 | 0x0800, subsets)
        else {
            panic!("expected subsets")
        };
        assert_eq!(subsets.subsets[0].first_index, 4);
        assert_eq!(subsets.subsets[0].first_vertex, 3);
        assert_eq!(subsets.subsets[0].bone_ids.as_deref(), Some(&[5, 8][..]));

        let mut old_material = vec![0; MATERIAL_NAME_0800_LEN];
        old_material[8..12].copy_from_slice(b"mat\0");
        put_i32(&mut old_material, 136, -1, true);
        put_i32(&mut old_material, 140, 2, true);
        put_i32(&mut old_material, 144, 4, true);
        put_i32(&mut old_material, 148, 5, true);
        let SupportedChunkPayload::MaterialName(old_material) =
            decode_direct(ChunkType::MaterialName, 0x8000 | 0x0800, old_material)
        else {
            panic!("expected old material")
        };
        assert_eq!(
            old_material.children,
            MaterialChildren::ChunkIds(vec![4, 5])
        );

        let mut material = vec![0; MATERIAL_NAME_0802_LEN];
        material[..6].copy_from_slice(b"multi\0");
        put_i32(&mut material, 128, 2, true);
        material.extend_from_slice(&0x1000_i32.to_be_bytes());
        material.extend_from_slice(&(-1_i32).to_be_bytes());
        material.extend_from_slice(b"a\0b\0");
        let SupportedChunkPayload::MaterialName(material) =
            decode_direct(ChunkType::MaterialName, 0x8000 | 0x0802, material)
        else {
            panic!("expected material")
        };
        assert_eq!(
            material.children,
            MaterialChildren::Names(vec!["a".into(), "b".into()])
        );

        let mut bones = vec![0; COMPILED_BONES_HEADER_LEN + COMPILED_BONE_LEN];
        let base = COMPILED_BONES_HEADER_LEN;
        put_u32(&mut bones, base, 42, true);
        put_f32(&mut bones, base + 212, 2.5, true);
        put_f32(&mut bones, base + 216, 3.5, true);
        put_f32(&mut bones, base + 264, 4.5, true);
        bones[base + 312..base + 317].copy_from_slice(b"root\0");
        put_i32(&mut bones, base + 572, 0, true);
        put_u32(&mut bones, base + 576, 1, true);
        let SupportedChunkPayload::CompiledBones(bones) =
            decode_direct(ChunkType::CompiledBones, 0x8000 | 0x0800, bones)
        else {
            panic!("expected bones")
        };
        assert_eq!(bones.bones[0].controller_id, 42);
        assert_eq!(bones.bones[0].mass.to_bits(), 2.5_f32.to_bits());
        assert_eq!(
            bones.bones[0].world_to_bone[0][0].to_bits(),
            3.5_f32.to_bits()
        );
        assert_eq!(
            bones.bones[0].bone_to_world[0][0].to_bits(),
            4.5_f32.to_bits()
        );
    }

    #[test]
    fn decodes_material_versions_and_compiled_bones() {
        let mut old_material = vec![0; MATERIAL_NAME_0800_LEN];
        old_material[8..12].copy_from_slice(b"mat\0");
        put_i32(&mut old_material, 136, -1, false);
        put_i32(&mut old_material, 140, 2, false);
        put_i32(&mut old_material, 144, 4, false);
        put_i32(&mut old_material, 148, 5, false);
        let SupportedChunkPayload::MaterialName(material) =
            decode_direct(ChunkType::MaterialName, 0x0800, old_material)
        else {
            panic!("expected material")
        };
        assert_eq!(material.name, "mat");
        assert_eq!(material.children, MaterialChildren::ChunkIds(vec![4, 5]));

        let mut material = vec![0; MATERIAL_NAME_0802_LEN];
        material[..6].copy_from_slice(b"multi\0");
        put_i32(&mut material, 128, 2, false);
        material.extend_from_slice(&0x1000_i32.to_le_bytes());
        material.extend_from_slice(&(-1_i32).to_le_bytes());
        material.extend_from_slice(b"a\0b\0");
        let SupportedChunkPayload::MaterialName(material) =
            decode_direct(ChunkType::MaterialName, 0x0802, material)
        else {
            panic!("expected material")
        };
        assert_eq!(
            material.children,
            MaterialChildren::Names(vec!["a".into(), "b".into()])
        );

        let mut bones = vec![0; COMPILED_BONES_HEADER_LEN + COMPILED_BONE_LEN];
        let base = COMPILED_BONES_HEADER_LEN;
        put_u32(&mut bones, base, 42, false);
        put_f32(&mut bones, base + 212, 2.5, false);
        bones[base + 312..base + 317].copy_from_slice(b"root\0");
        put_i32(&mut bones, base + 572, 0, false);
        put_u32(&mut bones, base + 576, 1, false);
        let SupportedChunkPayload::CompiledBones(bones) =
            decode_direct(ChunkType::CompiledBones, 0x0800, bones)
        else {
            panic!("expected bones")
        };
        assert_eq!(bones.bones[0].controller_id, 42);
        assert_eq!(bones.bones[0].name, "root");
        assert_eq!(bones.bones[0].mass.to_bits(), 2.5_f32.to_bits());
        assert_eq!(bones.bones[0].child_count, 1);
    }

    #[test]
    fn validates_stream_byte_length_and_skips_unselected_chunks() {
        let mut stream = vec![0; DATA_STREAM_0800_LEN];
        put_i32(&mut stream, 4, MeshStreamType::Indices as i32, false);
        put_i32(&mut stream, 8, 3, false);
        put_i32(&mut stream, 12, 2, false);
        stream.extend_from_slice(&[0; 5]);
        assert!(matches!(
            decode_error(ChunkType::DataStream, 0x0800, stream),
            PayloadError::UnexpectedEof {
                context: "data stream",
                ..
            }
        ));

        let file = cry_file(&[
            (ChunkType::SourceInfo.raw(), 0, 3, vec![]),
            (0x1019, 1, 4, vec![]),
        ]);
        for chunk in ChunkFile::parse(&file).unwrap().chunks() {
            assert!(chunk.unwrap().decode_supported().unwrap().is_none());
        }
    }

    #[test]
    fn accepts_all_public_node_and_data_stream_versions() {
        for version in [0x0823, 0x0824] {
            let mut node = vec![0; NODE_HEADER_LEN];
            node[..2].copy_from_slice(b"n\0");
            let SupportedChunkPayload::Node(node) = decode_direct(ChunkType::Node, version, node)
            else {
                panic!("expected node")
            };
            assert_eq!(node.name, "n");
        }

        for version in [0x0800, 0x0801] {
            let header_len = if version == 0x0800 {
                DATA_STREAM_0800_LEN
            } else {
                DATA_STREAM_0801_LEN
            };
            let mut stream = vec![0; header_len];
            put_i32(&mut stream, 4, MeshStreamType::Normals as i32, false);
            let SupportedChunkPayload::DataStream(stream) =
                decode_direct(ChunkType::DataStream, version, stream)
            else {
                panic!("expected stream")
            };
            assert_eq!(stream.stream_type, MeshStreamType::Normals);
            assert_eq!(stream.stream_index, 0);
        }
    }

    fn decode_direct(
        kind: ChunkType,
        version: u16,
        payload: Vec<u8>,
    ) -> SupportedChunkPayload<'static> {
        let bytes = Box::leak(cry_file(&[(kind.raw(), version, 1, payload)]).into_boxed_slice());
        ChunkFile::parse(bytes)
            .unwrap()
            .chunks()
            .next()
            .unwrap()
            .unwrap()
            .decode_supported()
            .unwrap()
            .unwrap()
    }

    fn decode_error(kind: ChunkType, version: u16, payload: Vec<u8>) -> PayloadError {
        let bytes = cry_file(&[(kind.raw(), version, 1, payload)]);
        ChunkFile::parse(&bytes)
            .unwrap()
            .chunks()
            .next()
            .unwrap()
            .unwrap()
            .decode_supported()
            .unwrap_err()
    }
    fn put_i32(bytes: &mut [u8], offset: usize, value: i32, big: bool) {
        let raw = if big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        bytes[offset..offset + 4].copy_from_slice(&raw);
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32, big: bool) {
        let raw = if big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        bytes[offset..offset + 4].copy_from_slice(&raw);
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16, big: bool) {
        let raw = if big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        bytes[offset..offset + 2].copy_from_slice(&raw);
    }

    fn put_f32(bytes: &mut [u8], offset: usize, value: f32, big: bool) {
        put_u32(bytes, offset, value.to_bits(), big);
    }
}
