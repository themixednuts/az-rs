use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
};

use cry_chunk::{
    CompiledBone, CryModel, DataStreamChunk, MaterialChildren, MeshChunk, MeshStreamType,
    MeshSubsetsChunk,
};
use half::f16;
use thiserror::Error;

const MESH_IS_EMPTY: i32 = 0x1;
const MESH_HAS_EXTRA_WEIGHTS: i32 = 0x4;
const SOURCE_TO_GLTF: Mat4 = Mat4([
    [-1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
]);

/// Material descriptor supplied to a neutral PBR resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialInput<'a> {
    pub chunk_id: u32,
    pub slot: u32,
    pub name: &'a str,
}

/// Metallic-roughness values written to a glTF material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PbrMaterial {
    pub base_color_factor: [f32; 4],
    pub metallic_factor: f32,
    pub roughness_factor: f32,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor: 0.0,
            roughness_factor: 1.0,
        }
    }
}

/// Checked conversion failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConversionError {
    #[error("node chunk {node_id} references missing mesh chunk {mesh_id}")]
    MissingMesh { node_id: u32, mesh_id: i32 },
    #[error("node chunk {node_id} references missing parent node chunk {parent_id}")]
    MissingParent { node_id: u32, parent_id: i32 },
    #[error("node hierarchy contains a cycle at chunk {node_id}")]
    NodeCycle { node_id: u32 },
    #[error("mesh chunk {mesh_id} is missing {stream_type:?} stream 0")]
    MissingStream {
        mesh_id: u32,
        stream_type: MeshStreamType,
    },
    #[error("mesh chunk {mesh_id} references missing data-stream chunk {stream_id}")]
    MissingDataStream { mesh_id: u32, stream_id: i32 },
    #[error(
        "mesh chunk {mesh_id} expected {expected:?} stream {expected_index}, but chunk {stream_id} contains {actual:?} stream {actual_index}"
    )]
    WrongDataStream {
        mesh_id: u32,
        stream_id: u32,
        expected: MeshStreamType,
        expected_index: u32,
        actual: MeshStreamType,
        actual_index: u32,
    },
    #[error(
        "{stream_type:?} stream chunk {stream_id} uses unsupported element size {element_size}; supported sizes are {supported:?}"
    )]
    UnsupportedElementSize {
        stream_id: u32,
        stream_type: MeshStreamType,
        element_size: u32,
        supported: &'static [u32],
    },
    #[error(
        "{stream_type:?} stream chunk {stream_id} contains {actual} elements; expected {expected}"
    )]
    StreamCountMismatch {
        stream_id: u32,
        stream_type: MeshStreamType,
        expected: u32,
        actual: u32,
    },
    #[error("mesh chunk {mesh_id} references missing mesh-subsets chunk {subsets_id}")]
    MissingMeshSubsets { mesh_id: u32, subsets_id: i32 },
    #[error(
        "mesh chunk {mesh_id} declares {declared} subsets, but chunk {subsets_id} contains {actual}"
    )]
    SubsetCountMismatch {
        mesh_id: u32,
        subsets_id: u32,
        declared: u32,
        actual: usize,
    },
    #[error("mesh chunk {mesh_id} subset {subset} index range is outside the index stream")]
    InvalidSubsetRange { mesh_id: u32, subset: usize },
    #[error(
        "mesh chunk {mesh_id} subset {subset} references vertex {vertex} outside its vertex range [{first_vertex}, {vertex_end})"
    )]
    VertexOutsideSubset {
        mesh_id: u32,
        subset: usize,
        vertex: u32,
        first_vertex: u32,
        vertex_end: u32,
    },
    #[error(
        "mesh chunk {mesh_id} subset {subset} contains {index_count} indices, which is not a triangle list"
    )]
    InvalidTriangleCount {
        mesh_id: u32,
        subset: usize,
        index_count: usize,
    },
    #[error(
        "mesh chunk {mesh_id} index {index} references vertex {vertex}, but the mesh has {vertex_count} vertices"
    )]
    InvalidVertexIndex {
        mesh_id: u32,
        index: usize,
        vertex: u32,
        vertex_count: u32,
    },
    #[error("node chunk {node_id} references missing material chunk {material_id}")]
    MissingMaterial { node_id: u32, material_id: i32 },
    #[error("node chunk {node_id} material chunk {material_id} has no slot {slot}")]
    MissingMaterialSlot {
        node_id: u32,
        material_id: u32,
        slot: i32,
    },
    #[error(
        "material chunk {material_id} slot {slot} references missing material chunk {child_id}"
    )]
    MissingChildMaterial {
        material_id: u32,
        slot: usize,
        child_id: i32,
    },
    #[error("material chunk {material_id} slot {slot} resolved invalid PBR values")]
    InvalidPbrMaterial { material_id: u32, slot: u32 },
    #[error("mesh chunk {mesh_id} has skinning data but no compiled-bones chunk")]
    MissingCompiledBones { mesh_id: u32 },
    #[error("skinning is ambiguous because the model contains {count} compiled-bones chunks")]
    AmbiguousCompiledBones { count: usize },
    #[error(
        "mesh chunk {mesh_id} bone stream has {actual} entries; expected {vertex_count} or {double_vertex_count}"
    )]
    BoneMappingCount {
        mesh_id: u32,
        vertex_count: u32,
        double_vertex_count: u32,
        actual: u32,
    },
    #[error(
        "mesh chunk {mesh_id} uses local 8-bit bone mappings without a complete public subset bone table"
    )]
    MissingSubsetBoneTable { mesh_id: u32 },
    #[error(
        "mesh chunk {mesh_id} subset {subset} local bone {local_bone} is outside its remapping table"
    )]
    InvalidLocalBone {
        mesh_id: u32,
        subset: usize,
        local_bone: u8,
    },
    #[error("mesh chunk {mesh_id} assigns conflicting bone mappings to vertex {vertex}")]
    ConflictingBoneMapping { mesh_id: u32, vertex: u32 },
    #[error(
        "mesh chunk {mesh_id} references joint {joint}, but the skeleton has {joint_count} joints"
    )]
    InvalidJoint {
        mesh_id: u32,
        joint: u16,
        joint_count: usize,
    },
    #[error("compiled bone {bone} has invalid parent offset {parent_offset}")]
    InvalidBoneParent { bone: usize, parent_offset: i32 },
    #[error("numeric size overflow while converting {context}")]
    SizeOverflow { context: &'static str },
    #[error("failed to serialize glTF JSON: {0}")]
    Json(String),
}

#[derive(Debug)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
    pub root_nodes: Vec<usize>,
    pub meshes: Vec<SceneMesh>,
    pub materials: Vec<SceneMaterial>,
    pub skin: Option<SceneSkin>,
}

#[derive(Debug)]
pub struct SceneNode {
    pub name: String,
    pub matrix: Mat4,
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
}

#[derive(Debug)]
pub struct SceneMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub texture_coordinates: Option<Vec<[f32; 2]>>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub bone_influences: Option<SceneBoneInfluences>,
    pub primitives: Vec<ScenePrimitive>,
}

#[derive(Debug)]
pub struct SceneBoneInfluences {
    pub primary: JointWeightSet,
    pub secondary: Option<JointWeightSet>,
}

#[derive(Debug)]
pub struct JointWeightSet {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[u8; 4]>,
}

#[derive(Debug)]
pub struct ScenePrimitive {
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Debug)]
pub struct SceneMaterial {
    pub name: String,
    pub pbr: PbrMaterial,
}

#[derive(Debug)]
pub struct SceneSkin {
    pub joint_nodes: Vec<usize>,
    pub root_joint_nodes: Vec<usize>,
    pub inverse_bind_matrices: Vec<Mat4>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    fn multiply(self, right: Self) -> Self {
        let mut result = [[0.0; 4]; 4];
        for (row_index, row) in result.iter_mut().enumerate() {
            for (column_index, value) in row.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|index| self.0[row_index][index] * right.0[index][column_index])
                    .sum();
            }
        }
        Self(result)
    }

    fn converted(self) -> Self {
        SOURCE_TO_GLTF.multiply(self).multiply(SOURCE_TO_GLTF)
    }

    pub fn column_major(self) -> [f32; 16] {
        let mut values = [0.0; 16];
        for column in 0..4 {
            for row in 0..4 {
                values[column * 4 + row] = self.0[row][column];
            }
        }
        values
    }
}

pub fn convert<F>(model: &CryModel<'_>, resolve_material: &F) -> Result<Scene, ConversionError>
where
    F: Fn(&MaterialInput<'_>) -> PbrMaterial,
{
    let (materials, material_indices) = build_materials(model, resolve_material)?;
    let node_ids: Vec<u32> = model.nodes.keys().copied().collect();
    let node_indices: BTreeMap<u32, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    validate_node_hierarchy(model, &node_indices)?;

    let mut nodes = Vec::with_capacity(node_ids.len());
    let mut meshes = Vec::new();
    let mut skinned_mesh_indices = Vec::new();
    for (node_index, node_id) in node_ids.iter().copied().enumerate() {
        let node = &model.nodes[&node_id];
        let mesh = if node.object_chunk_id > 0 {
            let mesh_id =
                u32::try_from(node.object_chunk_id).map_err(|_| ConversionError::MissingMesh {
                    node_id,
                    mesh_id: node.object_chunk_id,
                })?;
            let mesh_chunk = model
                .meshes
                .get(&mesh_id)
                .ok_or(ConversionError::MissingMesh {
                    node_id,
                    mesh_id: node.object_chunk_id,
                })?;
            if mesh_chunk.flags & MESH_IS_EMPTY != 0 {
                None
            } else {
                let (scene_mesh, skinned) = build_mesh(
                    model,
                    node_id,
                    mesh_id,
                    mesh_chunk,
                    node.material_chunk_id,
                    &material_indices,
                )?;
                let mesh_index = meshes.len();
                meshes.push(scene_mesh);
                if skinned {
                    skinned_mesh_indices.push((node_index, mesh_index, mesh_id));
                }
                Some(mesh_index)
            }
        } else {
            None
        };
        let mut children = Vec::new();
        for (child_id, child) in &model.nodes {
            if child.parent_chunk_id > 1
                && u32::try_from(child.parent_chunk_id).ok() == Some(node_id)
            {
                children.push(node_indices[child_id]);
            }
        }
        nodes.push(SceneNode {
            name: node.name.clone(),
            matrix: node_matrix(node.transform).converted(),
            children,
            mesh,
            skin: None,
        });
    }
    let mut root_nodes = node_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (model.nodes[id].parent_chunk_id <= 1).then_some(index))
        .collect::<Vec<_>>();

    let skin = if skinned_mesh_indices.is_empty() {
        None
    } else {
        let skin = build_skin(model, &mut nodes, skinned_mesh_indices[0].2)?;
        for (node_index, _, _) in &skinned_mesh_indices {
            nodes[*node_index].skin = Some(0);
        }
        root_nodes.extend(skin.root_joint_nodes.iter().copied());
        let joint_count = skin.joint_nodes.len();
        for (_, mesh_index, mesh_id) in skinned_mesh_indices {
            validate_joints(&meshes[mesh_index], mesh_id, joint_count)?;
        }
        Some(skin)
    };

    Ok(Scene {
        nodes,
        root_nodes,
        meshes,
        materials,
        skin,
    })
}

fn validate_node_hierarchy(
    model: &CryModel<'_>,
    node_indices: &BTreeMap<u32, usize>,
) -> Result<(), ConversionError> {
    for (node_id, node) in &model.nodes {
        if node.parent_chunk_id > 1
            && !node_indices.contains_key(&u32::try_from(node.parent_chunk_id).unwrap_or(u32::MAX))
        {
            return Err(ConversionError::MissingParent {
                node_id: *node_id,
                parent_id: node.parent_chunk_id,
            });
        }
        let mut seen = BTreeSet::new();
        let mut current = *node_id;
        while let Some(current_node) = model.nodes.get(&current) {
            if !seen.insert(current) {
                return Err(ConversionError::NodeCycle { node_id: current });
            }
            if current_node.parent_chunk_id <= 1 {
                break;
            }
            current = u32::try_from(current_node.parent_chunk_id).map_err(|_| {
                ConversionError::MissingParent {
                    node_id: current,
                    parent_id: current_node.parent_chunk_id,
                }
            })?;
        }
    }
    Ok(())
}

type MaterialIndices = BTreeMap<(u32, u32), usize>;
type BuiltMaterials = (Vec<SceneMaterial>, MaterialIndices);

fn build_materials<F>(model: &CryModel<'_>, resolve: &F) -> Result<BuiltMaterials, ConversionError>
where
    F: Fn(&MaterialInput<'_>) -> PbrMaterial,
{
    let mut descriptors = Vec::new();
    for (chunk_id, material) in &model.materials {
        match &material.children {
            MaterialChildren::Names(names) if !names.is_empty() => {
                for (slot, name) in names.iter().enumerate() {
                    descriptors.push((
                        *chunk_id,
                        u32_from_usize(slot, "material slot")?,
                        name.as_str(),
                    ));
                }
            }
            MaterialChildren::ChunkIds(ids) if !ids.is_empty() => {
                for (slot, child_id) in ids.iter().copied().enumerate() {
                    let child_key = u32::try_from(child_id).map_err(|_| {
                        ConversionError::MissingChildMaterial {
                            material_id: *chunk_id,
                            slot,
                            child_id,
                        }
                    })?;
                    let child = model.materials.get(&child_key).ok_or(
                        ConversionError::MissingChildMaterial {
                            material_id: *chunk_id,
                            slot,
                            child_id,
                        },
                    )?;
                    descriptors.push((
                        *chunk_id,
                        u32_from_usize(slot, "material slot")?,
                        child.name.as_str(),
                    ));
                }
            }
            _ => descriptors.push((*chunk_id, 0, material.name.as_str())),
        }
    }
    let mut materials = Vec::with_capacity(descriptors.len());
    let mut indices = BTreeMap::new();
    for (chunk_id, slot, name) in descriptors {
        let input = MaterialInput {
            chunk_id,
            slot,
            name,
        };
        let pbr = resolve(&input);
        if !valid_pbr(pbr) {
            return Err(ConversionError::InvalidPbrMaterial {
                material_id: chunk_id,
                slot,
            });
        }
        indices.insert((chunk_id, slot), materials.len());
        materials.push(SceneMaterial {
            name: name.to_owned(),
            pbr,
        });
    }
    Ok((materials, indices))
}

fn valid_pbr(pbr: PbrMaterial) -> bool {
    pbr.base_color_factor
        .iter()
        .chain([&pbr.metallic_factor, &pbr.roughness_factor])
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
}

fn build_mesh(
    model: &CryModel<'_>,
    node_id: u32,
    mesh_id: u32,
    mesh: &MeshChunk,
    material_id: i32,
    material_indices: &BTreeMap<(u32, u32), usize>,
) -> Result<(SceneMesh, bool), ConversionError> {
    let position_stream = required_stream(model, mesh_id, mesh, MeshStreamType::Positions, 0)?;
    let index_stream = required_stream(model, mesh_id, mesh, MeshStreamType::Indices, 0)?;
    require_count(position_stream, mesh.vertex_count)?;
    require_count(index_stream, mesh.index_count)?;
    let positions = decode_positions(position_stream)?;
    let normals = optional_stream(model, mesh_id, mesh, MeshStreamType::Normals, 0)?
        .map(|stream| {
            require_count(stream, mesh.vertex_count)?;
            Ok(decode_vec3(stream, &[12])?
                .into_iter()
                .map(convert_direction)
                .collect())
        })
        .transpose()?;
    let indices = decode_indices(index_stream)?;
    validate_vertex_indices(mesh_id, mesh.vertex_count, &indices)?;
    let texture_coordinates =
        optional_stream(model, mesh_id, mesh, MeshStreamType::TextureCoordinates, 0)?
            .map(|stream| {
                require_count(stream, mesh.vertex_count)?;
                decode_vec2(stream, &[8])
            })
            .transpose()?;
    let colors = optional_stream(model, mesh_id, mesh, MeshStreamType::Colors, 0)?
        .map(|stream| {
            require_count(stream, mesh.vertex_count)?;
            decode_colors(stream)
        })
        .transpose()?;

    let subsets = mesh_subsets(model, mesh_id, mesh)?;
    let primitives = build_primitives(
        node_id,
        mesh_id,
        material_id,
        material_indices,
        subsets,
        &indices,
    )?;
    let bone_stream = optional_stream(model, mesh_id, mesh, MeshStreamType::BoneMapping, 0)?;
    let bone_influences = if let Some(stream) = bone_stream {
        let mut mapping = decode_bone_mapping(mesh_id, mesh, subsets, &indices, stream)?;
        if mesh.flags & MESH_HAS_EXTRA_WEIGHTS == 0 {
            mapping.secondary = None;
        }
        Some(mapping)
    } else {
        None
    };
    Ok((
        SceneMesh {
            name: format!("mesh-{mesh_id}"),
            positions,
            normals,
            texture_coordinates,
            colors,
            bone_influences,
            primitives,
        },
        bone_stream.is_some(),
    ))
}

#[derive(Clone, Copy)]
struct StreamRef<'a> {
    id: u32,
    value: &'a DataStreamChunk<'a>,
}

impl<'a> Deref for StreamRef<'a> {
    type Target = DataStreamChunk<'a>;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

fn required_stream<'a>(
    model: &'a CryModel<'a>,
    mesh_id: u32,
    mesh: &MeshChunk,
    stream_type: MeshStreamType,
    index: usize,
) -> Result<StreamRef<'a>, ConversionError> {
    optional_stream(model, mesh_id, mesh, stream_type, index)?.ok_or(
        ConversionError::MissingStream {
            mesh_id,
            stream_type,
        },
    )
}

fn optional_stream<'a>(
    model: &'a CryModel<'a>,
    mesh_id: u32,
    mesh: &MeshChunk,
    stream_type: MeshStreamType,
    index: usize,
) -> Result<Option<StreamRef<'a>>, ConversionError> {
    let id = mesh.stream_chunk_ids[stream_type as usize][index];
    if id <= 0 {
        return Ok(None);
    }
    let stream_id = u32::try_from(id).map_err(|_| ConversionError::MissingDataStream {
        mesh_id,
        stream_id: id,
    })?;
    let stream = model
        .data_streams
        .get(&stream_id)
        .ok_or(ConversionError::MissingDataStream {
            mesh_id,
            stream_id: id,
        })?;
    let expected_index = u32_from_usize(index, "stream index")?;
    if stream.stream_type != stream_type || stream.stream_index != expected_index {
        return Err(ConversionError::WrongDataStream {
            mesh_id,
            stream_id,
            expected: stream_type,
            expected_index,
            actual: stream.stream_type,
            actual_index: stream.stream_index,
        });
    }
    Ok(Some(StreamRef {
        id: stream_id,
        value: stream,
    }))
}

fn require_count(stream: StreamRef<'_>, expected: u32) -> Result<(), ConversionError> {
    if stream.element_count == expected {
        Ok(())
    } else {
        Err(ConversionError::StreamCountMismatch {
            stream_id: stream.id,
            stream_type: stream.stream_type,
            expected,
            actual: stream.element_count,
        })
    }
}

fn decode_positions(stream: StreamRef<'_>) -> Result<Vec<[f32; 3]>, ConversionError> {
    match stream.element_size {
        12 => Ok(decode_vec3(stream, &[12])?
            .into_iter()
            .map(convert_point)
            .collect()),
        8 => {
            let mut result = Vec::with_capacity(usize_from_u32(stream.element_count, "positions")?);
            for element in stream.data.chunks_exact(8) {
                let x = half_value(element, 0, stream.data_is_big_endian);
                let y = half_value(element, 2, stream.data_is_big_endian);
                let z = half_value(element, 4, stream.data_is_big_endian);
                result.push(convert_point([x, y, z]));
            }
            Ok(result)
        }
        element_size => Err(unsupported_size(stream, element_size, &[8, 12])),
    }
}

fn decode_vec3(
    stream: StreamRef<'_>,
    supported: &'static [u32],
) -> Result<Vec<[f32; 3]>, ConversionError> {
    if stream.element_size != 12 {
        return Err(unsupported_size(stream, stream.element_size, supported));
    }
    Ok(stream
        .data
        .chunks_exact(12)
        .map(|bytes| {
            [
                float_value(bytes, 0, stream.data_is_big_endian),
                float_value(bytes, 4, stream.data_is_big_endian),
                float_value(bytes, 8, stream.data_is_big_endian),
            ]
        })
        .collect())
}

fn decode_vec2(
    stream: StreamRef<'_>,
    supported: &'static [u32],
) -> Result<Vec<[f32; 2]>, ConversionError> {
    if stream.element_size != 8 {
        return Err(unsupported_size(stream, stream.element_size, supported));
    }
    Ok(stream
        .data
        .chunks_exact(8)
        .map(|bytes| {
            [
                float_value(bytes, 0, stream.data_is_big_endian),
                float_value(bytes, 4, stream.data_is_big_endian),
            ]
        })
        .collect())
}

fn decode_colors(stream: StreamRef<'_>) -> Result<Vec<[u8; 4]>, ConversionError> {
    if stream.element_size != 4 {
        return Err(unsupported_size(stream, stream.element_size, &[4]));
    }
    Ok(stream
        .data
        .chunks_exact(4)
        .map(|bytes| [bytes[0], bytes[1], bytes[2], bytes[3]])
        .collect())
}

fn decode_indices(stream: StreamRef<'_>) -> Result<Vec<u32>, ConversionError> {
    match stream.element_size {
        2 => Ok(stream
            .data
            .chunks_exact(2)
            .map(|bytes| integer_u16(bytes, stream.data_is_big_endian).into())
            .collect()),
        4 => Ok(stream
            .data
            .chunks_exact(4)
            .map(|bytes| integer_u32(bytes, stream.data_is_big_endian))
            .collect()),
        element_size => Err(unsupported_size(stream, element_size, &[2, 4])),
    }
}

fn unsupported_size(
    stream: StreamRef<'_>,
    element_size: u32,
    supported: &'static [u32],
) -> ConversionError {
    ConversionError::UnsupportedElementSize {
        stream_id: stream.id,
        stream_type: stream.stream_type,
        element_size,
        supported,
    }
}

fn validate_vertex_indices(
    mesh_id: u32,
    vertex_count: u32,
    indices: &[u32],
) -> Result<(), ConversionError> {
    for (index, vertex) in indices.iter().copied().enumerate() {
        if vertex >= vertex_count {
            return Err(ConversionError::InvalidVertexIndex {
                mesh_id,
                index,
                vertex,
                vertex_count,
            });
        }
    }
    Ok(())
}

fn mesh_subsets<'a>(
    model: &'a CryModel<'_>,
    mesh_id: u32,
    mesh: &MeshChunk,
) -> Result<Option<&'a MeshSubsetsChunk>, ConversionError> {
    if mesh.subset_count == 0 {
        return Ok(None);
    }
    let subsets_id =
        u32::try_from(mesh.subsets_chunk_id).map_err(|_| ConversionError::MissingMeshSubsets {
            mesh_id,
            subsets_id: mesh.subsets_chunk_id,
        })?;
    let subsets =
        model
            .mesh_subsets
            .get(&subsets_id)
            .ok_or(ConversionError::MissingMeshSubsets {
                mesh_id,
                subsets_id: mesh.subsets_chunk_id,
            })?;
    if usize_from_u32(mesh.subset_count, "subset count")? != subsets.subsets.len() {
        return Err(ConversionError::SubsetCountMismatch {
            mesh_id,
            subsets_id,
            declared: mesh.subset_count,
            actual: subsets.subsets.len(),
        });
    }
    Ok(Some(subsets))
}

fn build_primitives(
    node_id: u32,
    mesh_id: u32,
    material_id: i32,
    material_indices: &BTreeMap<(u32, u32), usize>,
    subsets: Option<&MeshSubsetsChunk>,
    indices: &[u32],
) -> Result<Vec<ScenePrimitive>, ConversionError> {
    let ranges: Vec<(usize, usize, i32)> = if let Some(subsets) = subsets {
        subsets
            .subsets
            .iter()
            .map(|subset| {
                Ok((
                    usize_from_u32(subset.first_index, "subset first index")?,
                    usize_from_u32(subset.index_count, "subset index count")?,
                    subset.material_id,
                ))
            })
            .collect::<Result<_, ConversionError>>()?
    } else {
        vec![(0, indices.len(), 0)]
    };
    let material_key = if material_id > 0 {
        let material_id =
            u32::try_from(material_id).map_err(|_| ConversionError::MissingMaterial {
                node_id,
                material_id,
            })?;
        if !material_indices
            .keys()
            .any(|(chunk_id, _)| *chunk_id == material_id)
        {
            return Err(ConversionError::MissingMaterial {
                node_id,
                material_id: i32::try_from(material_id).unwrap_or(i32::MAX),
            });
        }
        Some(material_id)
    } else {
        None
    };
    let mut primitives = Vec::with_capacity(ranges.len());
    for (subset_index, (first, count, slot)) in ranges.into_iter().enumerate() {
        let end = first
            .checked_add(count)
            .ok_or(ConversionError::InvalidSubsetRange {
                mesh_id,
                subset: subset_index,
            })?;
        let primitive_indices =
            indices
                .get(first..end)
                .ok_or(ConversionError::InvalidSubsetRange {
                    mesh_id,
                    subset: subset_index,
                })?;
        if !primitive_indices.len().is_multiple_of(3) {
            return Err(ConversionError::InvalidTriangleCount {
                mesh_id,
                subset: subset_index,
                index_count: primitive_indices.len(),
            });
        }
        let material = material_key
            .map(|material_id| {
                let slot =
                    u32::try_from(slot).map_err(|_| ConversionError::MissingMaterialSlot {
                        node_id,
                        material_id,
                        slot,
                    })?;
                material_indices
                    .get(&(material_id, slot))
                    .copied()
                    .ok_or_else(|| ConversionError::MissingMaterialSlot {
                        node_id,
                        material_id,
                        slot: i32::try_from(slot).unwrap_or(i32::MAX),
                    })
            })
            .transpose()?;
        primitives.push(ScenePrimitive {
            indices: primitive_indices.to_vec(),
            material,
        });
    }
    Ok(primitives)
}

fn decode_bone_mapping(
    mesh_id: u32,
    mesh: &MeshChunk,
    subsets: Option<&MeshSubsetsChunk>,
    indices: &[u32],
    stream: StreamRef<'_>,
) -> Result<SceneBoneInfluences, ConversionError> {
    let double = mesh
        .vertex_count
        .checked_mul(2)
        .ok_or(ConversionError::SizeOverflow {
            context: "bone mapping count",
        })?;
    let groups = match stream.element_count {
        count if count == mesh.vertex_count => 1,
        count if count == double => 2,
        actual => {
            return Err(ConversionError::BoneMappingCount {
                mesh_id,
                vertex_count: mesh.vertex_count,
                double_vertex_count: double,
                actual,
            });
        }
    };
    let vertex_count = usize_from_u32(mesh.vertex_count, "bone mapping vertices")?;
    let mut joints = vec![[0; 4]; vertex_count * groups];
    let mut weights = vec![[0; 4]; vertex_count * groups];
    match stream.element_size {
        12 => {
            for (index, element) in stream.data.chunks_exact(12).enumerate() {
                for lane in 0..4 {
                    joints[index][lane] =
                        integer_u16(&element[lane * 2..lane * 2 + 2], stream.data_is_big_endian);
                    weights[index][lane] = element[8 + lane];
                }
            }
        }
        8 => decode_local_bones(
            mesh_id,
            subsets,
            indices,
            stream,
            groups,
            &mut joints,
            &mut weights,
        )?,
        element_size => return Err(unsupported_size(stream, element_size, &[8, 12])),
    }
    let secondary = (groups == 2).then(|| JointWeightSet {
        joints: joints.split_off(vertex_count),
        weights: weights.split_off(vertex_count),
    });
    Ok(SceneBoneInfluences {
        primary: JointWeightSet { joints, weights },
        secondary,
    })
}

fn decode_local_bones(
    mesh_id: u32,
    subsets: Option<&MeshSubsetsChunk>,
    indices: &[u32],
    stream: StreamRef<'_>,
    groups: usize,
    joints: &mut [[u16; 4]],
    weights: &mut [[u8; 4]],
) -> Result<(), ConversionError> {
    let subsets = subsets.ok_or(ConversionError::MissingSubsetBoneTable { mesh_id })?;
    let mut assigned = vec![false; joints.len()];
    let vertex_count = joints.len() / groups;
    for (subset_index, subset) in subsets.subsets.iter().enumerate() {
        let bone_ids = subset
            .bone_ids
            .as_deref()
            .ok_or(ConversionError::MissingSubsetBoneTable { mesh_id })?;
        let first = usize_from_u32(subset.first_index, "subset first index")?;
        let count = usize_from_u32(subset.index_count, "subset index count")?;
        let end = first
            .checked_add(count)
            .ok_or(ConversionError::InvalidSubsetRange {
                mesh_id,
                subset: subset_index,
            })?;
        let subset_indices =
            indices
                .get(first..end)
                .ok_or(ConversionError::InvalidSubsetRange {
                    mesh_id,
                    subset: subset_index,
                })?;
        let first_vertex = subset.first_vertex;
        let vertex_end = first_vertex.checked_add(subset.vertex_count).ok_or(
            ConversionError::InvalidSubsetRange {
                mesh_id,
                subset: subset_index,
            },
        )?;
        for vertex in subset_indices.iter().copied() {
            if vertex < first_vertex || vertex >= vertex_end {
                return Err(ConversionError::VertexOutsideSubset {
                    mesh_id,
                    subset: subset_index,
                    vertex,
                    first_vertex,
                    vertex_end,
                });
            }
            let vertex = usize_from_u32(vertex, "bone mapping vertex")?;
            for group in 0..groups {
                let output_index = group * vertex_count + vertex;
                let input = &stream.data[output_index * 8..output_index * 8 + 8];
                let mut mapped_joints = [0; 4];
                let mut mapped_weights = [0; 4];
                for lane in 0..4 {
                    let weight = input[4 + lane];
                    mapped_weights[lane] = weight;
                    if weight != 0 {
                        let local = input[lane];
                        mapped_joints[lane] = *bone_ids.get(usize::from(local)).ok_or(
                            ConversionError::InvalidLocalBone {
                                mesh_id,
                                subset: subset_index,
                                local_bone: local,
                            },
                        )?;
                    }
                }
                if assigned[output_index]
                    && (joints[output_index] != mapped_joints
                        || weights[output_index] != mapped_weights)
                {
                    return Err(ConversionError::ConflictingBoneMapping {
                        mesh_id,
                        vertex: u32_from_usize(vertex, "bone mapping vertex")?,
                    });
                }
                assigned[output_index] = true;
                joints[output_index] = mapped_joints;
                weights[output_index] = mapped_weights;
            }
        }
    }
    Ok(())
}

fn build_skin(
    model: &CryModel<'_>,
    nodes: &mut Vec<SceneNode>,
    first_skinned_mesh_id: u32,
) -> Result<SceneSkin, ConversionError> {
    if model.compiled_bones.is_empty() {
        return Err(ConversionError::MissingCompiledBones {
            mesh_id: first_skinned_mesh_id,
        });
    }
    if model.compiled_bones.len() != 1 {
        return Err(ConversionError::AmbiguousCompiledBones {
            count: model.compiled_bones.len(),
        });
    }
    let Some(compiled_bones) = model.compiled_bones.values().next() else {
        return Err(ConversionError::MissingCompiledBones {
            mesh_id: first_skinned_mesh_id,
        });
    };
    let bones = &compiled_bones.bones;
    let base_node = nodes.len();
    let mut joint_nodes = Vec::with_capacity(bones.len());
    let mut root_joint_nodes = Vec::new();
    let mut inverse_bind_matrices = Vec::with_capacity(bones.len());
    for (bone_index, bone) in bones.iter().enumerate() {
        let parent = bone_parent(bone_index, bone, bones.len())?;
        let local = parent.map_or_else(
            || matrix34(bone.bone_to_world),
            |parent_index| {
                matrix34(bones[parent_index].world_to_bone).multiply(matrix34(bone.bone_to_world))
            },
        );
        let node_index = base_node + bone_index;
        joint_nodes.push(node_index);
        if parent.is_none() {
            root_joint_nodes.push(node_index);
        }
        nodes.push(SceneNode {
            name: bone.name.clone(),
            matrix: local.converted(),
            children: Vec::new(),
            mesh: None,
            skin: None,
        });
        inverse_bind_matrices.push(matrix34(bone.world_to_bone).converted());
    }
    for (bone_index, bone) in bones.iter().enumerate() {
        if let Some(parent) = bone_parent(bone_index, bone, bones.len())? {
            nodes[base_node + parent]
                .children
                .push(base_node + bone_index);
        }
    }
    Ok(SceneSkin {
        joint_nodes,
        root_joint_nodes,
        inverse_bind_matrices,
    })
}

fn bone_parent(
    bone_index: usize,
    bone: &CompiledBone,
    bone_count: usize,
) -> Result<Option<usize>, ConversionError> {
    if bone.parent_offset == 0 {
        return Ok(None);
    }
    let index = i64::try_from(bone_index).map_err(|_| ConversionError::SizeOverflow {
        context: "bone index",
    })? + i64::from(bone.parent_offset);
    let parent = usize::try_from(index).map_err(|_| ConversionError::InvalidBoneParent {
        bone: bone_index,
        parent_offset: bone.parent_offset,
    })?;
    if parent >= bone_count || parent == bone_index {
        return Err(ConversionError::InvalidBoneParent {
            bone: bone_index,
            parent_offset: bone.parent_offset,
        });
    }
    Ok(Some(parent))
}

fn validate_joints(
    mesh: &SceneMesh,
    mesh_id: u32,
    joint_count: usize,
) -> Result<(), ConversionError> {
    let Some(influences) = &mesh.bone_influences else {
        return Ok(());
    };
    for joint in std::iter::once(&influences.primary)
        .chain(influences.secondary.iter())
        .flat_map(|set| set.joints.iter())
        .flatten()
        .copied()
    {
        if joint >= 0xfffe || usize::from(joint) >= joint_count {
            return Err(ConversionError::InvalidJoint {
                mesh_id,
                joint,
                joint_count,
            });
        }
    }
    Ok(())
}

fn node_matrix(raw: [[f32; 4]; 4]) -> Mat4 {
    let mut result = Mat4::IDENTITY.0;
    for (row, values) in result.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = raw[column][row];
        }
    }
    for row in result.iter_mut().take(3) {
        row[3] *= 0.01;
    }
    Mat4(result)
}

const fn matrix34(raw: [[f32; 4]; 3]) -> Mat4 {
    Mat4([raw[0], raw[1], raw[2], [0.0, 0.0, 0.0, 1.0]])
}

fn convert_point([x, y, z]: [f32; 3]) -> [f32; 3] {
    [-x, z, y]
}

fn convert_direction(value: [f32; 3]) -> [f32; 3] {
    convert_point(value)
}

fn float_value(bytes: &[u8], offset: usize, big_endian: bool) -> f32 {
    f32::from_bits(integer_u32(&bytes[offset..offset + 4], big_endian))
}

fn half_value(bytes: &[u8], offset: usize, big_endian: bool) -> f32 {
    f16::from_bits(integer_u16(&bytes[offset..offset + 2], big_endian)).to_f32()
}

fn integer_u16(bytes: &[u8], big_endian: bool) -> u16 {
    let value = [bytes[0], bytes[1]];
    if big_endian {
        u16::from_be_bytes(value)
    } else {
        u16::from_le_bytes(value)
    }
}

fn integer_u32(bytes: &[u8], big_endian: bool) -> u32 {
    let value = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if big_endian {
        u32::from_be_bytes(value)
    } else {
        u32::from_le_bytes(value)
    }
}

fn usize_from_u32(value: u32, context: &'static str) -> Result<usize, ConversionError> {
    usize::try_from(value).map_err(|_| ConversionError::SizeOverflow { context })
}

fn u32_from_usize(value: usize, context: &'static str) -> Result<u32, ConversionError> {
    u32::try_from(value).map_err(|_| ConversionError::SizeOverflow { context })
}
