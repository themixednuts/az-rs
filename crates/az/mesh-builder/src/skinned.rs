//! Self-contained glTF skinned-model products.

use az_asset_builder::{
    AssetBuilderPattern, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductFormat,
    ProductFormatId, SourceFormat, SourceSchemaType, TypedBuildProduct, product_format_id,
    source_schema_type,
};
pub use az_mesh::{SKINNED_MESH_ASSET_TYPE, SKINNED_MESH_ASSET_TYPE_NAME, SkinnedMeshAssetData};
use gltf::mesh::Mode;
use thiserror::Error;
use uuid::uuid;

use crate::rigged::{RiggedGltfError, validate_rigged_glb};

pub const SKINNED_MESH_SOURCE_SCHEMA_NAME: &str = "azoth.mesh.SkinnedSource";
pub const SKINNED_MESH_PRODUCT_EXTENSION: &str = "skinned.glb";
pub const SKINNED_MESH_PRODUCT_SUB_ID: u32 = 1;
pub const SKINNED_MESH_BUILDER_NAME: &str = "azoth.skinned-mesh";
pub const SKINNED_MESH_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("75d9137e-708f-4b47-a70e-44547f3fc087"));

pub struct SkinnedMeshSourceFormat;

impl SourceFormat for SkinnedMeshSourceFormat {
    const SCHEMA: Option<SourceSchemaType> = Some(SourceSchemaType::__from_static(
        SKINNED_MESH_SOURCE_SCHEMA_NAME,
    ));
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[SourceSchemaType::__from_static(
        SKINNED_MESH_SOURCE_SCHEMA_NAME,
    )];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.skinnedmesh.glb")];
}

pub struct SkinnedMeshProductFormat;

impl ProductFormat for SkinnedMeshProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static(SKINNED_MESH_ASSET_TYPE_NAME);
    const VERSION: u32 = 1;
    type Asset = SkinnedMeshAssetData;
}

pub const SKINNED_MESH_FORMAT_ID: ProductFormatId = product_format_id::<SkinnedMeshProductFormat>();
pub const SKINNED_MESH_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<SkinnedMeshSourceFormat>();

#[must_use]
pub fn skinned_mesh_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<SkinnedMeshSourceFormat>()
        .named(SKINNED_MESH_BUILDER_NAME)
        .id(SKINNED_MESH_BUILDER_ID)
        .version(1)
        .analysis_fingerprint(crate::fingerprint::passthrough_analysis_fingerprint())
        .produces::<SkinnedMeshProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        ..CreateJobsResponse::default()
    }
}

fn process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if let Err(error) = validate_skinned_mesh_glb(&request.source_path, request.source_bytes) {
        tracing::warn!(source = %request.source_path, %error, "skinned mesh product failed");
        return ProcessJobResponse {
            result: ProcessJobResult::Failed,
            ..ProcessJobResponse::default()
        };
    }

    ProcessJobResponse {
        products: vec![
            TypedBuildProduct::<SkinnedMeshProductFormat>::from_trusted_path(
                skinned_mesh_product_path(&request.source_path),
                SKINNED_MESH_PRODUCT_SUB_ID,
                request.source_bytes.to_vec(),
            )
            .erase(),
        ],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

#[must_use]
pub fn skinned_mesh_product_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = normalized
        .strip_suffix(".skinnedmesh.glb")
        .unwrap_or(normalized.as_str());
    format!("{stem}.{SKINNED_MESH_PRODUCT_EXTENSION}")
}

#[derive(Debug, Error)]
pub enum SkinnedMeshProductError {
    #[error(transparent)]
    RiggedGltf(#[from] RiggedGltfError),
    #[error("skinned mesh source {source_path} has no triangle mesh primitives")]
    MissingPrimitives { source_path: String },
    #[error("skinned mesh source {source_path} primitive {primitive} has no positions")]
    MissingPositions {
        source_path: String,
        primitive: String,
    },
    #[error(
        "skinned mesh source {source_path} primitive {primitive} uses unsupported topology {mode:?}"
    )]
    UnsupportedTopology {
        source_path: String,
        primitive: String,
        mode: Mode,
    },
    #[error(
        "skinned mesh source {source_path} primitive {primitive} must provide JOINTS_0 and WEIGHTS_0 together"
    )]
    PartialSkinAttributes {
        source_path: String,
        primitive: String,
    },
    #[error(
        "skinned mesh source {source_path} primitive {primitive} has {actual} {attribute} values for {vertices} vertices"
    )]
    AttributeCount {
        source_path: String,
        primitive: String,
        attribute: &'static str,
        actual: usize,
        vertices: usize,
    },
    #[error("skinned mesh source {source_path} mesh {mesh} has weights but no node skin binding")]
    MissingSkinBinding { source_path: String, mesh: usize },
    #[error(
        "skinned mesh source {source_path} primitive {primitive} vertex {vertex} has invalid weights"
    )]
    InvalidWeights {
        source_path: String,
        primitive: String,
        vertex: usize,
    },
    #[error(
        "skinned mesh source {source_path} primitive {primitive} vertex {vertex} references joint {joint}, but its skin has {joint_count} joints"
    )]
    JointOutOfRange {
        source_path: String,
        primitive: String,
        vertex: usize,
        joint: u16,
        joint_count: usize,
    },
    #[error("skinned mesh source {source_path} has no weighted mesh primitive")]
    MissingWeightedPrimitive { source_path: String },
}

/// Accept a `.skinnedmesh.glb` source only if every primitive is a triangle list
/// and every skinned vertex carries consistent, normalized weights.
///
/// # Errors
///
/// Returns [`SkinnedMeshProductError::RiggedGltf`] for any rig-level failure
/// [`validate_rigged_glb`] reports (bytes that do not parse, an external buffer,
/// a missing binary chunk, no skins, or a malformed inverse bind matrix). Beyond
/// that: [`SkinnedMeshProductError::UnsupportedTopology`] for a primitive whose
/// mode is not [`Mode::Triangles`],
/// [`SkinnedMeshProductError::MissingPositions`] for a primitive with no
/// `POSITION` accessor, [`SkinnedMeshProductError::PartialSkinAttributes`] if
/// exactly one of `JOINTS_0`/`WEIGHTS_0` is present,
/// [`SkinnedMeshProductError::AttributeCount`] if either skin attribute has a
/// different element count than `POSITION`,
/// [`SkinnedMeshProductError::MissingSkinBinding`] if a weighted mesh is never
/// referenced by a skinned node, [`SkinnedMeshProductError::InvalidWeights`] for
/// a vertex whose weights are non-finite, negative, or do not sum to `1.0`
/// within `0.001`, [`SkinnedMeshProductError::JointOutOfRange`] for a
/// positively-weighted joint index at or past the skin's joint count,
/// [`SkinnedMeshProductError::MissingPrimitives`] if the document declares no
/// primitives, or [`SkinnedMeshProductError::MissingWeightedPrimitive`] if none
/// of them is skinned.
///
/// # Panics
///
/// Panics if a `JOINTS_0` or `WEIGHTS_0` accessor that
/// [`gltf::Primitive::get`] reported cannot then be read through
/// [`gltf::Primitive::reader`], which would mean the document contradicts
/// itself.
pub fn validate_skinned_mesh_glb(
    source_path: &str,
    bytes: &[u8],
) -> Result<(), SkinnedMeshProductError> {
    let validated = validate_rigged_glb(source_path, bytes)?;
    let gltf = validated.gltf();
    let blob = validated.blob();
    let joint_limits = mesh_joint_limits(gltf, validated.skin_joint_counts());

    let mut primitive_count = 0;
    let mut weighted_primitive_count = 0;
    for mesh in gltf.meshes() {
        for primitive in mesh.primitives() {
            primitive_count += 1;
            if validate_skinned_primitive(
                source_path,
                blob,
                mesh.index(),
                joint_limits[mesh.index()],
                &primitive,
            )? {
                weighted_primitive_count += 1;
            }
        }
    }
    if primitive_count == 0 {
        return Err(SkinnedMeshProductError::MissingPrimitives {
            source_path: source_path.to_owned(),
        });
    }
    if weighted_primitive_count == 0 {
        return Err(SkinnedMeshProductError::MissingWeightedPrimitive {
            source_path: source_path.to_owned(),
        });
    }
    Ok(())
}

/// Tightest joint count each mesh is bound under, across every node that skins
/// it. `None` marks a mesh no skinned node references.
fn mesh_joint_limits(gltf: &gltf::Gltf, skin_joint_counts: &[usize]) -> Vec<Option<usize>> {
    let mut limits = vec![None; gltf.meshes().len()];
    for node in gltf.nodes() {
        if let (Some(mesh), Some(skin)) = (node.mesh(), node.skin()) {
            let count = skin_joint_counts[skin.index()];
            let limit = &mut limits[mesh.index()];
            *limit = Some(limit.map_or(count, |current: usize| current.min(count)));
        }
    }
    limits
}

/// Validate one primitive. Returns whether it carried skin attributes.
fn validate_skinned_primitive(
    source_path: &str,
    blob: &[u8],
    mesh_index: usize,
    joint_limit: Option<usize>,
    primitive: &gltf::Primitive<'_>,
) -> Result<bool, SkinnedMeshProductError> {
    let label = format!("{}:{}", mesh_index, primitive.index());
    if primitive.mode() != Mode::Triangles {
        return Err(SkinnedMeshProductError::UnsupportedTopology {
            source_path: source_path.to_owned(),
            primitive: label,
            mode: primitive.mode(),
        });
    }
    let positions = primitive.get(&gltf::Semantic::Positions).ok_or_else(|| {
        SkinnedMeshProductError::MissingPositions {
            source_path: source_path.to_owned(),
            primitive: label.clone(),
        }
    })?;
    let joints = primitive.get(&gltf::Semantic::Joints(0));
    let weights = primitive.get(&gltf::Semantic::Weights(0));
    if joints.is_some() != weights.is_some() {
        return Err(SkinnedMeshProductError::PartialSkinAttributes {
            source_path: source_path.to_owned(),
            primitive: label,
        });
    }
    let (Some(joints), Some(weights)) = (joints, weights) else {
        return Ok(false);
    };
    validate_attribute_count(
        source_path,
        &label,
        "JOINTS_0",
        joints.count(),
        positions.count(),
    )?;
    validate_attribute_count(
        source_path,
        &label,
        "WEIGHTS_0",
        weights.count(),
        positions.count(),
    )?;
    let joint_count = joint_limit.ok_or_else(|| SkinnedMeshProductError::MissingSkinBinding {
        source_path: source_path.to_owned(),
        mesh: mesh_index,
    })?;
    validate_skin_weights(source_path, blob, &label, joint_count, primitive)?;
    Ok(true)
}

/// Walk every skinned vertex, checking weight normalization and joint range.
fn validate_skin_weights(
    source_path: &str,
    blob: &[u8],
    label: &str,
    joint_count: usize,
    primitive: &gltf::Primitive<'_>,
) -> Result<(), SkinnedMeshProductError> {
    let reader = primitive.reader(|_| Some(blob));
    let joints = reader
        .read_joints(0)
        .expect("validated JOINTS_0 accessor")
        .into_u16();
    let weights = reader
        .read_weights(0)
        .expect("validated WEIGHTS_0 accessor")
        .into_f32();
    for (vertex, (joints, weights)) in joints.zip(weights).enumerate() {
        let weight_sum = weights.iter().copied().sum::<f32>();
        if weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight < 0.0)
            || (weight_sum - 1.0).abs() > 0.001
        {
            return Err(SkinnedMeshProductError::InvalidWeights {
                source_path: source_path.to_owned(),
                primitive: label.to_owned(),
                vertex,
            });
        }
        for (joint, weight) in joints.into_iter().zip(weights) {
            if weight > 0.0 && usize::from(joint) >= joint_count {
                return Err(SkinnedMeshProductError::JointOutOfRange {
                    source_path: source_path.to_owned(),
                    primitive: label.to_owned(),
                    vertex,
                    joint,
                    joint_count,
                });
            }
        }
    }
    Ok(())
}

fn validate_attribute_count(
    source_path: &str,
    primitive: &str,
    attribute: &'static str,
    actual: usize,
    vertices: usize,
) -> Result<(), SkinnedMeshProductError> {
    if actual == vertices {
        Ok(())
    } else {
        Err(SkinnedMeshProductError::AttributeCount {
            source_path: source_path.to_owned(),
            primitive: primitive.to_owned(),
            attribute,
            actual,
            vertices,
        })
    }
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{ProductFormat, SourceFormat};

    use super::*;

    #[test]
    fn skinned_mesh_rule_is_schema_and_suffix_specific() {
        let registries = az_gem_contract::Registries::new();
        let rule = skinned_mesh_build_rule(&JobContext::new(&registries));
        assert!(rule.matches_source(
            "characters/player/body.skinnedmesh.glb",
            Some(SKINNED_MESH_SOURCE_SCHEMA_NAME)
        ));
        assert!(!rule.matches_source(
            "meshes/props/crate.glb",
            Some(SKINNED_MESH_SOURCE_SCHEMA_NAME)
        ));
        assert_eq!(SkinnedMeshProductFormat::ID, SKINNED_MESH_FORMAT_ID);
        assert_eq!(
            SkinnedMeshSourceFormat::SCHEMA,
            Some(SKINNED_MESH_SOURCE_SCHEMA)
        );
    }

    #[test]
    fn valid_skinned_glb_becomes_the_runtime_product() {
        let bytes = skinned_triangle_glb();
        validate_skinned_mesh_glb("characters/test.skinnedmesh.glb", &bytes).unwrap();
        let request_path = "characters/test.skinnedmesh.glb";
        assert_eq!(
            skinned_mesh_product_path(request_path),
            "characters/test.skinned.glb"
        );
    }

    #[test]
    fn static_glb_is_not_accepted_as_a_skinned_model() {
        let bytes = glb_with_json(
            r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":0}],"scenes":[{"nodes":[]}],"scene":0}"#,
            &[],
        );
        assert!(matches!(
            validate_skinned_mesh_glb("characters/test.skinnedmesh.glb", &bytes),
            Err(SkinnedMeshProductError::RiggedGltf(
                RiggedGltfError::MissingBinaryBuffer { .. } | RiggedGltfError::MissingSkins { .. }
            ))
        ));
    }

    fn skinned_triangle_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        for _ in 0..3 {
            for value in [0_u16; 4] {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for value in [1.0_f32, 0.0, 0.0, 0.0] {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        for value in glam::Mat4::IDENTITY.to_cols_array() {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        glb_with_json(
            r#"{
                "asset":{"version":"2.0"},
                "buffers":[{"byteLength":172}],
                "bufferViews":[
                    {"buffer":0,"byteOffset":0,"byteLength":36,"target":34962},
                    {"buffer":0,"byteOffset":36,"byteLength":24,"target":34962},
                    {"buffer":0,"byteOffset":60,"byteLength":48,"target":34962},
                    {"buffer":0,"byteOffset":108,"byteLength":64}
                ],
                "accessors":[
                    {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
                    {"bufferView":1,"componentType":5123,"count":3,"type":"VEC4"},
                    {"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"},
                    {"bufferView":3,"componentType":5126,"count":1,"type":"MAT4"}
                ],
                "meshes":[{"primitives":[{"attributes":{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}}]}],
                "nodes":[{"name":"root"},{"name":"body","mesh":0,"skin":0}],
                "skins":[{"joints":[0],"skeleton":0,"inverseBindMatrices":3}],
                "scenes":[{"nodes":[0,1]}],
                "scene":0
            }"#,
            &bin,
        )
    }

    fn glb_with_json(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let bin_chunk_len = if bin.is_empty() { 0 } else { 8 + bin.len() };
        let total_len = 12 + 8 + json.len() + bin_chunk_len;
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(
            &u32::try_from(total_len)
                .expect("test GLB total length fits in u32")
                .to_le_bytes(),
        );
        glb.extend_from_slice(
            &u32::try_from(json.len())
                .expect("test GLB JSON chunk fits in u32")
                .to_le_bytes(),
        );
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        if !bin.is_empty() {
            glb.extend_from_slice(
                &u32::try_from(bin.len())
                    .expect("test GLB BIN chunk fits in u32")
                    .to_le_bytes(),
            );
            glb.extend_from_slice(b"BIN\0");
            glb.extend_from_slice(&bin);
        }
        glb
    }
}
