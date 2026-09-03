use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    ChunkFile, ChunkFileError, ChunkFileSignature, CompiledBonesChunk, DataStreamChunk,
    MaterialNameChunk, MeshChunk, MeshSubsetsChunk, NodeChunk, PayloadError, SupportedChunkPayload,
};

/// Indexed geometry payloads from a Cry chunk file.
#[derive(Debug, Clone)]
pub struct CryModel<'a> {
    signature: ChunkFileSignature,
    pub nodes: BTreeMap<u32, NodeChunk<'a>>,
    pub meshes: BTreeMap<u32, MeshChunk>,
    pub mesh_subsets: BTreeMap<u32, MeshSubsetsChunk>,
    pub data_streams: BTreeMap<u32, DataStreamChunk<'a>>,
    pub materials: BTreeMap<u32, MaterialNameChunk>,
    pub compiled_bones: BTreeMap<u32, CompiledBonesChunk>,
}

impl<'a> CryModel<'a> {
    /// Parse a 0x746 container and index the supported geometry payloads.
    ///
    /// Public chunks outside the supported subset and extension chunk values
    /// remain available through [`ChunkFile`] but are not indexed here.
    ///
    /// # Errors
    ///
    /// Returns [`CryModelError`] for an invalid container, malformed selected
    /// payload, or duplicate selected chunk identifier.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, CryModelError> {
        let file = ChunkFile::parse(bytes)?;
        let mut model = Self {
            signature: file.signature(),
            nodes: BTreeMap::new(),
            meshes: BTreeMap::new(),
            mesh_subsets: BTreeMap::new(),
            data_streams: BTreeMap::new(),
            materials: BTreeMap::new(),
            compiled_bones: BTreeMap::new(),
        };
        let mut selected_ids = BTreeSet::new();
        for chunk in file.chunks() {
            let chunk = chunk?;
            let id = chunk.header().id();
            let Some(payload) = chunk.decode_supported()? else {
                continue;
            };
            if !selected_ids.insert(id) {
                return Err(CryModelError::DuplicateChunkId { id });
            }
            match payload {
                SupportedChunkPayload::Node(value) => {
                    insert_new(&mut model.nodes, id, value);
                }
                SupportedChunkPayload::Mesh(value) => {
                    insert_new(&mut model.meshes, id, *value);
                }
                SupportedChunkPayload::MeshSubsets(value) => {
                    insert_new(&mut model.mesh_subsets, id, value);
                }
                SupportedChunkPayload::DataStream(value) => {
                    insert_new(&mut model.data_streams, id, value);
                }
                SupportedChunkPayload::MaterialName(value) => {
                    insert_new(&mut model.materials, id, value);
                }
                SupportedChunkPayload::CompiledBones(value) => {
                    insert_new(&mut model.compiled_bones, id, value);
                }
            }
        }
        Ok(model)
    }

    /// Return the source container signature.
    #[must_use]
    pub const fn signature(&self) -> ChunkFileSignature {
        self.signature
    }
}

fn insert_new<T>(map: &mut BTreeMap<u32, T>, id: u32, value: T) {
    let previous = map.insert(id, value);
    debug_assert!(previous.is_none());
}

/// Error returned while constructing a geometry model index.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryModelError {
    #[error(transparent)]
    Container(#[from] ChunkFileError),
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error("duplicate supported chunk identifier {id}")]
    DuplicateChunkId { id: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkFileSignature, ChunkType, test_support::cry_file};

    #[test]
    fn indexes_selected_chunks_and_ignores_extensions() {
        let mut node = vec![0; 204];
        node[..5].copy_from_slice(b"root\0");
        let bytes = cry_file(&[
            (ChunkType::Node.raw(), 0x0824, 7, node),
            (ChunkType::SourceInfo.raw(), 1, 8, vec![]),
            (0x300a, 1, 9, vec![1, 2]),
        ]);
        let model = CryModel::parse(&bytes).unwrap();

        assert_eq!(model.signature(), ChunkFileSignature::Cry);
        assert_eq!(model.nodes.get(&7).unwrap().name, "root");
        assert_eq!(model.nodes.len(), 1);
        assert!(model.meshes.is_empty());
    }

    #[test]
    fn rejects_duplicate_selected_chunk_ids() {
        let mut first = vec![0; 204];
        first[..2].copy_from_slice(b"a\0");
        let mut second = vec![0; 204];
        second[..2].copy_from_slice(b"b\0");
        let bytes = cry_file(&[
            (ChunkType::Node.raw(), 0x0824, 7, first),
            (ChunkType::Node.raw(), 0x0824, 7, second),
        ]);

        assert_eq!(
            CryModel::parse(&bytes).unwrap_err(),
            CryModelError::DuplicateChunkId { id: 7 }
        );
    }
}
