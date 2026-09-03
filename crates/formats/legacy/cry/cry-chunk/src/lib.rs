//! Safe parser for public Cry 0x746 containers and geometry payloads.
//!
//! The layout matches Lumberyard's `ChunkFile::FileHeader_0x746` and
//! `ChunkFile::ChunkTableEntry_0x746` declarations in
//! `dev/Code/CryEngine/Cry3DEngine/CGF/ChunkFileComponents.h`. Geometry
//! payloads follow the public declarations in
//! `dev/Code/CryEngine/CryCommon/CryHeaders.h`.

mod model;
mod payload;

pub use model::{CryModel, CryModelError};
pub use payload::{
    CompiledBone, CompiledBonesChunk, DataStreamChunk, MaterialChildren, MaterialNameChunk,
    MeshChunk, MeshStreamType, MeshSubset, MeshSubsetsChunk, NodeChunk, PayloadError,
    SupportedChunkPayload,
};

use thiserror::Error;

const CRY_SIGNATURE: [u8; 4] = *b"CrCh";
const SPEED_TREE_SIGNATURE: [u8; 4] = *b"STCh";
const FILE_HEADER_LEN: usize = 16;
const CHUNK_TABLE_ENTRY_LEN: usize = 16;
const BIG_ENDIAN_VERSION_FLAG: u16 = 0x8000;
const MAX_CHUNK_COUNT: u32 = 10_000_000;

/// Version stored by the 0x746 container header.
pub const CHUNK_FILE_VERSION: u32 = 0x746;

/// Public chunk identifiers declared by Lumberyard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ChunkType {
    Any = 0,
    Mesh = 0x1000,
    Helper = 0x1001,
    VertexAnimation = 0x1002,
    BoneAnimation = 0x1003,
    GeometryNameList = 0x1004,
    BoneNameList = 0x1005,
    MaterialList = 0x1006,
    MipmapReductionMesh = 0x1007,
    SceneProperties = 0x1008,
    Light = 0x1009,
    PatchMesh = 0x100a,
    Node = 0x100b,
    Material = 0x100c,
    Controller = 0x100d,
    Timing = 0x100e,
    BoneMesh = 0x100f,
    BoneLightBinding = 0x1010,
    MeshMorphTarget = 0x1011,
    BoneInitialPosition = 0x1012,
    SourceInfo = 0x1013,
    MaterialName = 0x1014,
    ExportFlags = 0x1015,
    DataStream = 0x1016,
    MeshSubsets = 0x1017,
    MeshPhysicsData = 0x1018,
    CompiledBones = 0x2000,
    CompiledPhysicalBones = 0x2001,
    CompiledMorphTargets = 0x2002,
    CompiledPhysicalProxies = 0x2003,
    CompiledInternalFaces = 0x2004,
    CompiledInternalSkinVertices = 0x2005,
    CompiledExternalToInternalMap = 0x2006,
    BreakablePhysics = 0x3000,
    FaceMap = 0x3001,
    MotionParameters = 0x3002,
    FootPlantInfo = 0x3003,
    BoneBoxes = 0x3004,
    FoliageInfo = 0x3005,
    Timestamp = 0x3006,
    GlobalAnimationHeaderCaf = 0x3007,
    GlobalAnimationHeaderAim = 0x3008,
    BspTreeData = 0x3009,
}

impl ChunkType {
    /// Convert a raw table value when it is part of the public chunk enum.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Option<Self> {
        Some(match raw {
            0 => Self::Any,
            0x1000 => Self::Mesh,
            0x1001 => Self::Helper,
            0x1002 => Self::VertexAnimation,
            0x1003 => Self::BoneAnimation,
            0x1004 => Self::GeometryNameList,
            0x1005 => Self::BoneNameList,
            0x1006 => Self::MaterialList,
            0x1007 => Self::MipmapReductionMesh,
            0x1008 => Self::SceneProperties,
            0x1009 => Self::Light,
            0x100a => Self::PatchMesh,
            0x100b => Self::Node,
            0x100c => Self::Material,
            0x100d => Self::Controller,
            0x100e => Self::Timing,
            0x100f => Self::BoneMesh,
            0x1010 => Self::BoneLightBinding,
            0x1011 => Self::MeshMorphTarget,
            0x1012 => Self::BoneInitialPosition,
            0x1013 => Self::SourceInfo,
            0x1014 => Self::MaterialName,
            0x1015 => Self::ExportFlags,
            0x1016 => Self::DataStream,
            0x1017 => Self::MeshSubsets,
            0x1018 => Self::MeshPhysicsData,
            0x2000 => Self::CompiledBones,
            0x2001 => Self::CompiledPhysicalBones,
            0x2002 => Self::CompiledMorphTargets,
            0x2003 => Self::CompiledPhysicalProxies,
            0x2004 => Self::CompiledInternalFaces,
            0x2005 => Self::CompiledInternalSkinVertices,
            0x2006 => Self::CompiledExternalToInternalMap,
            0x3000 => Self::BreakablePhysics,
            0x3001 => Self::FaceMap,
            0x3002 => Self::MotionParameters,
            0x3003 => Self::FootPlantInfo,
            0x3004 => Self::BoneBoxes,
            0x3005 => Self::FoliageInfo,
            0x3006 => Self::Timestamp,
            0x3007 => Self::GlobalAnimationHeaderCaf,
            0x3008 => Self::GlobalAnimationHeaderAim,
            0x3009 => Self::BspTreeData,
            _ => return None,
        })
    }

    /// Return the value stored in a chunk table entry.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self as u16
    }
}

/// A signature accepted by Lumberyard's 0x746 reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkFileSignature {
    Cry,
    SpeedTree,
}

impl ChunkFileSignature {
    /// Return the four bytes stored in the file header.
    #[must_use]
    pub const fn bytes(self) -> [u8; 4] {
        match self {
            Self::Cry => CRY_SIGNATURE,
            Self::SpeedTree => SPEED_TREE_SIGNATURE,
        }
    }

    const fn from_bytes(bytes: [u8; 4]) -> Option<Self> {
        match bytes {
            CRY_SIGNATURE => Some(Self::Cry),
            SPEED_TREE_SIGNATURE => Some(Self::SpeedTree),
            _ => None,
        }
    }
}

/// A validated borrowed 0x746 chunk file.
#[derive(Debug, Clone, Copy)]
pub struct ChunkFile<'a> {
    bytes: &'a [u8],
    signature: ChunkFileSignature,
    chunk_table: &'a [u8],
    chunk_table_offset: u32,
}

impl<'a> ChunkFile<'a> {
    /// Parse the file header and validate the complete chunk table.
    ///
    /// # Errors
    ///
    /// Returns [`ChunkFileError`] when the header is truncated, the signature
    /// or version is unsupported, or the declared table is outside `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ChunkFileError> {
        let header = bytes
            .get(..FILE_HEADER_LEN)
            .ok_or(ChunkFileError::UnexpectedEof {
                context: "chunk file header",
            })?;
        let signature_bytes = read_array::<4>(header, 0, "chunk file signature")?;
        let signature = ChunkFileSignature::from_bytes(signature_bytes).ok_or(
            ChunkFileError::InvalidSignature {
                found: signature_bytes,
            },
        )?;

        let version = read_u32(header, 4, "chunk file version")?;
        if version != CHUNK_FILE_VERSION {
            return Err(ChunkFileError::UnsupportedVersion { version });
        }

        let chunk_count = read_u32(header, 8, "chunk count")?;
        if chunk_count > MAX_CHUNK_COUNT {
            return Err(ChunkFileError::TooManyChunks {
                count: chunk_count,
                maximum: MAX_CHUNK_COUNT,
            });
        }
        let chunk_table_offset = read_u32(header, 12, "chunk table offset")?;
        let table_start = usize::try_from(chunk_table_offset).map_err(|_| {
            ChunkFileError::ChunkTableOutOfBounds {
                offset: chunk_table_offset,
                count: chunk_count,
            }
        })?;
        let table_len = usize::try_from(chunk_count)
            .ok()
            .and_then(|count| count.checked_mul(CHUNK_TABLE_ENTRY_LEN))
            .ok_or(ChunkFileError::ChunkTableOutOfBounds {
                offset: chunk_table_offset,
                count: chunk_count,
            })?;
        let table_end =
            table_start
                .checked_add(table_len)
                .ok_or(ChunkFileError::ChunkTableOutOfBounds {
                    offset: chunk_table_offset,
                    count: chunk_count,
                })?;
        let chunk_table =
            bytes
                .get(table_start..table_end)
                .ok_or(ChunkFileError::ChunkTableOutOfBounds {
                    offset: chunk_table_offset,
                    count: chunk_count,
                })?;

        Ok(Self {
            bytes,
            signature,
            chunk_table,
            chunk_table_offset,
        })
    }

    /// Return the accepted file signature.
    #[must_use]
    pub const fn signature(self) -> ChunkFileSignature {
        self.signature
    }

    /// Return the container version.
    #[must_use]
    pub const fn version(self) -> u32 {
        CHUNK_FILE_VERSION
    }

    /// Return the number of table entries.
    #[must_use]
    pub const fn chunk_count(self) -> usize {
        self.chunk_table.len() / CHUNK_TABLE_ENTRY_LEN
    }

    /// Return the byte offset of the chunk table.
    #[must_use]
    pub const fn chunk_table_offset(self) -> u32 {
        self.chunk_table_offset
    }

    /// Iterate over validated table entries and their borrowed payloads.
    #[must_use]
    pub fn chunks(self) -> Chunks<'a> {
        Chunks {
            bytes: self.bytes,
            entries: self.chunk_table.chunks_exact(CHUNK_TABLE_ENTRY_LEN),
        }
    }
}

/// One validated table entry and its borrowed payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    header: ChunkHeader,
    payload: &'a [u8],
}

impl<'a> Chunk<'a> {
    /// Return the decoded table entry.
    #[must_use]
    pub const fn header(self) -> ChunkHeader {
        self.header
    }

    /// Return the bytes selected by the entry's offset and size.
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// Decode this chunk when it belongs to the supported geometry subset.
    ///
    /// Other public chunk types and unknown extension values return `None`.
    ///
    /// # Errors
    ///
    /// Returns [`PayloadError`] when a selected geometry payload is malformed
    /// or uses an unsupported payload version.
    pub fn decode_supported(self) -> Result<Option<SupportedChunkPayload<'a>>, PayloadError> {
        payload::decode(self)
    }
}

/// A decoded 0x746 chunk-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    kind: u16,
    version: u16,
    id: u32,
    size: u32,
    offset: u32,
    big_endian: bool,
}

impl ChunkHeader {
    fn parse(bytes: &[u8]) -> Result<Self, ChunkFileError> {
        let raw_version = read_u16(bytes, 2, "chunk version")?;
        Ok(Self {
            kind: read_u16(bytes, 0, "chunk kind")?,
            version: raw_version & !BIG_ENDIAN_VERSION_FLAG,
            id: read_u32(bytes, 4, "chunk id")?,
            size: read_u32(bytes, 8, "chunk size")?,
            offset: read_u32(bytes, 12, "chunk offset")?,
            big_endian: raw_version & BIG_ENDIAN_VERSION_FLAG != 0,
        })
    }

    /// Return the raw chunk type identifier.
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.kind
    }

    /// Return the public chunk type, if the raw identifier is declared.
    #[must_use]
    pub const fn chunk_type(self) -> Option<ChunkType> {
        ChunkType::from_raw(self.kind)
    }

    /// Return the chunk payload version without the endian flag.
    #[must_use]
    pub const fn version(self) -> u16 {
        self.version
    }

    /// Return the chunk identifier.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.id
    }

    /// Return the payload size in bytes.
    #[must_use]
    pub const fn size(self) -> u32 {
        self.size
    }

    /// Return the payload offset from the start of the file.
    #[must_use]
    pub const fn offset(self) -> u32 {
        self.offset
    }

    /// Return whether the chunk payload uses big-endian values.
    #[must_use]
    pub const fn is_big_endian(self) -> bool {
        self.big_endian
    }

    fn payload(self, bytes: &[u8]) -> Result<&[u8], ChunkFileError> {
        let start = usize::try_from(self.offset).map_err(|_| self.out_of_bounds())?;
        let size = usize::try_from(self.size).map_err(|_| self.out_of_bounds())?;
        let end = start
            .checked_add(size)
            .ok_or_else(|| self.out_of_bounds())?;
        bytes.get(start..end).ok_or_else(|| self.out_of_bounds())
    }

    const fn out_of_bounds(self) -> ChunkFileError {
        ChunkFileError::ChunkOutOfBounds {
            id: self.id,
            offset: self.offset,
            size: self.size,
        }
    }
}

/// Iterator over the chunk table.
#[derive(Debug, Clone)]
pub struct Chunks<'a> {
    bytes: &'a [u8],
    entries: std::slice::ChunksExact<'a, u8>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, ChunkFileError>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        Some(ChunkHeader::parse(entry).and_then(|header| {
            header
                .payload(self.bytes)
                .map(|payload| Chunk { header, payload })
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for Chunks<'_> {}

/// Error returned while parsing a chunk-file container.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkFileError {
    #[error("unexpected end of file while reading {context}")]
    UnexpectedEof { context: &'static str },
    #[error("invalid Cry chunk-file signature {found:?}")]
    InvalidSignature { found: [u8; 4] },
    #[error("unsupported Cry chunk-file version {version:#x}")]
    UnsupportedVersion { version: u32 },
    #[error("chunk file declares {count} chunks; the supported maximum is {maximum}")]
    TooManyChunks { count: u32, maximum: u32 },
    #[error("chunk table at {offset:#x} with {count} entries points outside the file")]
    ChunkTableOutOfBounds { offset: u32, count: u32 },
    #[error("chunk {id} at {offset:#x} with {size} bytes points outside the file")]
    ChunkOutOfBounds { id: u32, offset: u32, size: u32 },
}

fn read_u16(bytes: &[u8], offset: usize, context: &'static str) -> Result<u16, ChunkFileError> {
    Ok(u16::from_le_bytes(read_array(bytes, offset, context)?))
}

fn read_u32(bytes: &[u8], offset: usize, context: &'static str) -> Result<u32, ChunkFileError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset, context)?))
}

fn read_array<const N: usize>(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<[u8; N], ChunkFileError> {
    let end = offset
        .checked_add(N)
        .ok_or(ChunkFileError::UnexpectedEof { context })?;
    bytes
        .get(offset..end)
        .ok_or(ChunkFileError::UnexpectedEof { context })?
        .try_into()
        .map_err(|_| ChunkFileError::UnexpectedEof { context })
}

#[cfg(test)]
mod test_support {
    use super::{CHUNK_FILE_VERSION, CHUNK_TABLE_ENTRY_LEN, FILE_HEADER_LEN};
    use crate::ChunkFileSignature;

    pub fn cry_file(chunks: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
        chunk_file(ChunkFileSignature::Cry, chunks)
    }

    pub fn chunk_file(
        signature: ChunkFileSignature,
        chunks: &[(u16, u16, u32, Vec<u8>)],
    ) -> Vec<u8> {
        let table_len = chunks.len() * CHUNK_TABLE_ENTRY_LEN;
        let mut bytes = Vec::with_capacity(
            FILE_HEADER_LEN + table_len + chunks.iter().map(|chunk| chunk.3.len()).sum::<usize>(),
        );
        bytes.extend_from_slice(&signature.bytes());
        bytes.extend_from_slice(&CHUNK_FILE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(chunks.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(FILE_HEADER_LEN).unwrap().to_le_bytes());

        let mut payload_offset = FILE_HEADER_LEN + table_len;
        for (kind, version, id, payload) in chunks {
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&version.to_le_bytes());
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(payload_offset).unwrap().to_le_bytes());
            payload_offset += payload.len();
        }
        for (_, _, _, payload) in chunks {
            bytes.extend_from_slice(payload);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::chunk_file;

    #[test]
    fn parses_cry_chunks_without_copying_payloads() {
        let bytes = chunk_file(
            ChunkFileSignature::Cry,
            &[(0x1000, 0x0802, 7, vec![1, 2, 3, 4])],
        );
        let file = ChunkFile::parse(&bytes).unwrap();

        assert_eq!(file.signature(), ChunkFileSignature::Cry);
        assert_eq!(file.version(), CHUNK_FILE_VERSION);
        assert_eq!(file.chunk_count(), 1);
        assert_eq!(file.chunks().len(), 1);

        let chunk = file.chunks().next().unwrap().unwrap();
        assert_eq!(chunk.header().kind(), 0x1000);
        assert_eq!(chunk.header().version(), 0x0802);
        assert_eq!(chunk.header().id(), 7);
        assert!(!chunk.header().is_big_endian());
        assert_eq!(chunk.payload(), &[1, 2, 3, 4]);
    }

    #[test]
    fn accepts_speed_tree_signature_and_big_endian_payload_flag() {
        let bytes = chunk_file(
            ChunkFileSignature::SpeedTree,
            &[(0x1001, BIG_ENDIAN_VERSION_FLAG | 3, 9, vec![5, 6])],
        );
        let file = ChunkFile::parse(&bytes).unwrap();
        let chunk = file.chunks().next().unwrap().unwrap();

        assert_eq!(file.signature(), ChunkFileSignature::SpeedTree);
        assert_eq!(chunk.header().version(), 3);
        assert!(chunk.header().is_big_endian());
        assert_eq!(chunk.payload(), &[5, 6]);
    }

    #[test]
    fn exposes_only_public_chunk_type_values() {
        assert_eq!(
            ChunkType::from_raw(0x1018),
            Some(ChunkType::MeshPhysicsData)
        );
        assert_eq!(ChunkType::from_raw(0x3009), Some(ChunkType::BspTreeData));
        assert_eq!(ChunkType::from_raw(0x1019), None);
        assert_eq!(ChunkType::from_raw(0x300a), None);
        assert_eq!(ChunkType::from_raw(0x300b), None);
    }

    #[test]
    fn rejects_unknown_signature_and_version() {
        let mut bytes = chunk_file(ChunkFileSignature::Cry, &[]);
        bytes[..4].copy_from_slice(b"bad!");
        assert!(matches!(
            ChunkFile::parse(&bytes),
            Err(ChunkFileError::InvalidSignature { found }) if found == *b"bad!"
        ));

        bytes[..4].copy_from_slice(&CRY_SIGNATURE);
        bytes[4..8].copy_from_slice(&0x745_u32.to_le_bytes());
        assert!(matches!(
            ChunkFile::parse(&bytes),
            Err(ChunkFileError::UnsupportedVersion { version: 0x745 })
        ));
    }

    #[test]
    fn rejects_tables_and_payloads_outside_the_file() {
        let mut excessive_count = chunk_file(ChunkFileSignature::Cry, &[]);
        excessive_count[8..12].copy_from_slice(&(MAX_CHUNK_COUNT + 1).to_le_bytes());
        assert_eq!(
            ChunkFile::parse(&excessive_count).unwrap_err(),
            ChunkFileError::TooManyChunks {
                count: MAX_CHUNK_COUNT + 1,
                maximum: MAX_CHUNK_COUNT,
            }
        );

        let mut truncated_table = chunk_file(ChunkFileSignature::Cry, &[]);
        truncated_table[8..12].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            ChunkFile::parse(&truncated_table),
            Err(ChunkFileError::ChunkTableOutOfBounds {
                offset: 16,
                count: 1
            })
        ));

        let mut bad_payload = chunk_file(ChunkFileSignature::Cry, &[(0x1000, 1, 12, vec![1])]);
        bad_payload[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        let file = ChunkFile::parse(&bad_payload).unwrap();
        assert!(matches!(
            file.chunks().next().unwrap(),
            Err(ChunkFileError::ChunkOutOfBounds { id: 12, .. })
        ));
    }
}
