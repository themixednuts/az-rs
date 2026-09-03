//! glTF/GLB import into the renderer-neutral processed mesh shape.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use az_core::AssetPathBuf;
use base64::Engine as _;
use glam::{Mat4, Vec3, Vec4};
use gltf::buffer::Source;
use gltf::mesh::Mode;
use thiserror::Error;

use crate::{MeshAsset, MeshMaterialSlot, MeshPrimitive};

#[derive(Debug)]
pub struct ImportedMesh {
    pub asset: MeshAsset,
    pub material_sources: Vec<String>,
}

#[derive(Debug, Error)]
pub enum MeshImportError {
    #[error("parse glTF source: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("mesh source has no scene")]
    MissingScene,
    #[error("mesh primitive {primitive} has no positions")]
    MissingPositions { primitive: String },
    #[error("mesh primitive {primitive} uses unsupported topology {mode:?}")]
    UnsupportedTopology { primitive: String, mode: Mode },
    #[error("mesh source has no triangle primitives")]
    MissingPrimitives,
    #[error("mesh buffer URI `{uri}` is unsupported")]
    UnsupportedBufferUri { uri: String },
    #[error("mesh buffer path `{path}` is invalid: {reason}")]
    InvalidBufferPath { path: String, reason: String },
    #[error("read mesh buffer {path}: {source}")]
    ReadBuffer {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("binary glTF buffer is missing")]
    MissingBinaryBuffer,
    #[error("mesh bounds contain non-finite values")]
    InvalidBounds,
}

/// Discover external glTF buffer dependencies without loading them.
///
/// # Errors
///
/// Returns [`MeshImportError::Gltf`] if `source_bytes` is not a parseable
/// glTF/GLB document, [`MeshImportError::UnsupportedBufferUri`] if a buffer URI
/// is absolute or otherwise not a relative asset path, or
/// [`MeshImportError::InvalidBufferPath`] if a relative URI escapes the asset
/// root.
pub fn gltf_source_dependencies(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<Vec<String>, MeshImportError> {
    let gltf = gltf::Gltf::from_slice(source_bytes)?;
    let mut dependencies = Vec::new();
    for buffer in gltf.document.buffers() {
        if let Source::Uri(uri) = buffer.source()
            && !uri.starts_with("data:")
        {
            dependencies.push(resolve_relative_asset_path(source_path, uri)?);
        }
    }
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

/// Import one glTF/GLB source. Node transforms are baked into each primitive,
/// making the product deterministic and directly renderable by Bevy.
///
/// # Errors
///
/// Returns [`MeshImportError::Gltf`] if `source_bytes` does not parse,
/// [`MeshImportError::MissingBinaryBuffer`], [`MeshImportError::ReadBuffer`],
/// [`MeshImportError::UnsupportedBufferUri`] or
/// [`MeshImportError::InvalidBufferPath`] if a referenced buffer cannot be
/// resolved and loaded, [`MeshImportError::MissingScene`] if the document has no
/// scene, [`MeshImportError::UnsupportedTopology`] for any primitive that is not
/// [`Mode::Triangles`], [`MeshImportError::MissingPositions`] for a primitive
/// without a `POSITION` accessor, [`MeshImportError::MissingPrimitives`] if the
/// scene contributed no primitives, or [`MeshImportError::InvalidBounds`] if the
/// accumulated bounds are non-finite.
pub fn import_gltf(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<ImportedMesh, MeshImportError> {
    let gltf = gltf::Gltf::from_slice(source_bytes)?;
    let buffers = load_buffers(&gltf, source_root, source_path)?;
    let (slots, material_sources) = material_slots(&gltf.document);
    let slot_by_material = gltf
        .document
        .materials()
        .map(|material| {
            let index = material.index().unwrap_or(usize::MAX);
            let slot = slots
                .iter()
                .find(|slot| slot.label == material_label(&material))
                .map_or(0, |slot| slot.id);
            (index, slot)
        })
        .collect::<BTreeMap<_, _>>();

    let scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next())
        .ok_or(MeshImportError::MissingScene)?;
    let mut primitives = Vec::new();
    let mut bounds_min = Vec3::splat(f32::INFINITY);
    let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);
    for node in scene.nodes() {
        visit_node(
            &node,
            Mat4::IDENTITY,
            &buffers,
            &slot_by_material,
            &mut primitives,
            &mut bounds_min,
            &mut bounds_max,
        )?;
    }
    if primitives.is_empty() {
        return Err(MeshImportError::MissingPrimitives);
    }
    if !bounds_min.is_finite() || !bounds_max.is_finite() {
        return Err(MeshImportError::InvalidBounds);
    }

    let name = Path::new(source_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mesh")
        .to_owned();
    Ok(ImportedMesh {
        asset: MeshAsset {
            name,
            bounds_min: bounds_min.to_array(),
            bounds_max: bounds_max.to_array(),
            material_slots: slots,
            primitives,
        },
        material_sources,
    })
}

fn visit_node(
    node: &gltf::Node<'_>,
    parent_transform: Mat4,
    buffers: &[Vec<u8>],
    slot_by_material: &BTreeMap<usize, u32>,
    primitives: &mut Vec<MeshPrimitive>,
    bounds_min: &mut Vec3,
    bounds_max: &mut Vec3,
) -> Result<(), MeshImportError> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let transform = parent_transform * local;
    let normal_transform = transform.inverse().transpose();
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let primitive_index = primitive.index();
            let label = format!(
                "{}:{}",
                node.name().or_else(|| mesh.name()).unwrap_or("mesh"),
                primitive_index
            );
            if primitive.mode() != Mode::Triangles {
                return Err(MeshImportError::UnsupportedTopology {
                    primitive: label,
                    mode: primitive.mode(),
                });
            }
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
            let positions = reader
                .read_positions()
                .ok_or_else(|| MeshImportError::MissingPositions {
                    primitive: label.clone(),
                })?
                .map(|position| {
                    let position = transform.transform_point3(Vec3::from_array(position));
                    *bounds_min = bounds_min.min(position);
                    *bounds_max = bounds_max.max(position);
                    position.to_array()
                })
                .collect::<Vec<_>>();
            let normals = reader
                .read_normals()
                .map(|values| {
                    values
                        .map(|normal| {
                            normal_transform
                                .transform_vector3(Vec3::from_array(normal))
                                .normalize_or_zero()
                                .to_array()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let tangents = reader
                .read_tangents()
                .map(|values| {
                    values
                        .map(|tangent| {
                            let direction = normal_transform
                                .transform_vector3(Vec3::new(tangent[0], tangent[1], tangent[2]))
                                .normalize_or_zero();
                            Vec4::new(direction.x, direction.y, direction.z, tangent[3]).to_array()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let uv0 = reader
                .read_tex_coords(0)
                .map(|values| values.into_f32().collect())
                .unwrap_or_default();
            let indices = reader.read_indices().map_or_else(
                || (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect(),
                |values| values.into_u32().collect(),
            );
            let material_slot = primitive
                .material()
                .index()
                .and_then(|index| slot_by_material.get(&index).copied())
                .unwrap_or(0);
            primitives.push(MeshPrimitive {
                label,
                material_slot,
                positions,
                normals,
                tangents,
                uv0,
                indices,
            });
        }
    }
    for child in node.children() {
        visit_node(
            &child,
            transform,
            buffers,
            slot_by_material,
            primitives,
            bounds_min,
            bounds_max,
        )?;
    }
    Ok(())
}

fn load_buffers(
    gltf: &gltf::Gltf,
    source_root: &Path,
    source_path: &str,
) -> Result<Vec<Vec<u8>>, MeshImportError> {
    let mut buffers = Vec::new();
    for buffer in gltf.document.buffers() {
        let bytes = match buffer.source() {
            Source::Bin => gltf
                .blob
                .clone()
                .ok_or(MeshImportError::MissingBinaryBuffer)?,
            Source::Uri(uri) => {
                if let Some((metadata, payload)) = uri.split_once(',')
                    && metadata.starts_with("data:")
                    && metadata.ends_with(";base64")
                {
                    base64::engine::general_purpose::STANDARD
                        .decode(payload)
                        .map_err(|error| MeshImportError::InvalidBufferPath {
                            path: "embedded glTF buffer".to_owned(),
                            reason: error.to_string(),
                        })?
                } else {
                    let relative = resolve_relative_asset_path(source_path, uri)?;
                    let path =
                        source_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
                    std::fs::read(&path)
                        .map_err(|source| MeshImportError::ReadBuffer { path, source })?
                }
            }
        };
        buffers.push(bytes);
    }
    Ok(buffers)
}

fn material_slots(document: &gltf::Document) -> (Vec<MeshMaterialSlot>, Vec<String>) {
    let mut slots = vec![MeshMaterialSlot {
        id: 0,
        label: "default".to_owned(),
        default_material: None,
    }];
    let mut material_sources = Vec::new();
    for material in document.materials() {
        let label = material_label(&material);
        let index = material.index().unwrap_or(0);
        let id = az_core::crc::crc32_lower(format!("{index}:{label}").as_bytes());
        let material_source = label
            .ends_with(".azmaterial.ron")
            .then(|| label.replace('\\', "/"));
        let default_material = material_source
            .as_deref()
            .map(az_material_builder::material_product_path);
        if let Some(source) = material_source {
            material_sources.push(source);
        }
        slots.push(MeshMaterialSlot {
            id,
            label,
            default_material,
        });
    }
    material_sources.sort();
    material_sources.dedup();
    (slots, material_sources)
}

fn material_label(material: &gltf::Material<'_>) -> String {
    material.name().map_or_else(
        || format!("material-{}", material.index().unwrap_or(0)),
        str::to_owned,
    )
}

fn resolve_relative_asset_path(source_path: &str, uri: &str) -> Result<String, MeshImportError> {
    if uri.contains("://") || uri.contains('%') {
        return Err(MeshImportError::UnsupportedBufferUri {
            uri: uri.to_owned(),
        });
    }
    let source_parent = Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let combined = source_parent.join(uri);
    let normalized = combined.to_string_lossy().replace('\\', "/");
    AssetPathBuf::new(normalized.clone()).map_err(|error| MeshImportError::InvalidBufferPath {
        path: normalized.clone(),
        reason: error.to_string(),
    })?;
    Ok(normalized)
}
