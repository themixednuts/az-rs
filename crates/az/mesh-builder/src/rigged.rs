//! Shared validation for self-contained rigged GLB authoring sources.

use gltf::buffer::Source;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RiggedGltfError {
    #[error("rigged glTF source {source_path} must be a binary GLB")]
    NotBinaryGlb { source_path: String },
    #[error("parse rigged glTF source {source_path}: {source}")]
    Gltf {
        source_path: String,
        source: gltf::Error,
    },
    #[error("rigged glTF source {source_path} has no scene")]
    MissingScene { source_path: String },
    #[error("rigged glTF source {source_path} references external buffer {buffer}")]
    ExternalBuffer { source_path: String, buffer: usize },
    #[error("rigged glTF source {source_path} has no binary buffer")]
    MissingBinaryBuffer { source_path: String },
    #[error("rigged glTF source {source_path} has no skins")]
    MissingSkins { source_path: String },
    #[error("rigged glTF source {source_path} skin {skin} has no joints")]
    EmptySkin { source_path: String, skin: usize },
    #[error("rigged glTF source {source_path} skin {skin} has no inverse bind matrices")]
    MissingInverseBindMatrices { source_path: String, skin: usize },
    #[error(
        "rigged glTF source {source_path} skin {skin} has {inverse_bind_count} inverse bind matrices for {joint_count} joints"
    )]
    InverseBindCountMismatch {
        source_path: String,
        skin: usize,
        joint_count: usize,
        inverse_bind_count: usize,
    },
    #[error("rigged glTF source {source_path} skin {skin} has a non-finite inverse bind matrix")]
    InvalidInverseBindMatrix { source_path: String, skin: usize },
}

pub struct ValidatedRiggedGltf {
    gltf: gltf::Gltf,
    skin_joint_counts: Vec<usize>,
}

impl ValidatedRiggedGltf {
    pub const fn gltf(&self) -> &gltf::Gltf {
        &self.gltf
    }

    pub fn blob(&self) -> &[u8] {
        self.gltf
            .blob
            .as_deref()
            .expect("validated rigged GLB has a binary buffer")
    }

    pub fn skin_joint_counts(&self) -> &[usize] {
        &self.skin_joint_counts
    }
}

pub fn validate_rigged_glb(
    source_path: &str,
    bytes: &[u8],
) -> Result<ValidatedRiggedGltf, RiggedGltfError> {
    if !bytes.starts_with(b"glTF") {
        return Err(RiggedGltfError::NotBinaryGlb {
            source_path: source_path.to_owned(),
        });
    }
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|source| RiggedGltfError::Gltf {
        source_path: source_path.to_owned(),
        source,
    })?;
    gltf.default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or_else(|| RiggedGltfError::MissingScene {
            source_path: source_path.to_owned(),
        })?;
    for buffer in gltf.buffers() {
        if !matches!(buffer.source(), Source::Bin) {
            return Err(RiggedGltfError::ExternalBuffer {
                source_path: source_path.to_owned(),
                buffer: buffer.index(),
            });
        }
    }
    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| RiggedGltfError::MissingBinaryBuffer {
            source_path: source_path.to_owned(),
        })?;
    let mut skin_joint_counts = Vec::with_capacity(gltf.skins().len());
    for skin in gltf.skins() {
        let skin_index = skin.index();
        let joint_count = skin.joints().count();
        if joint_count == 0 {
            return Err(RiggedGltfError::EmptySkin {
                source_path: source_path.to_owned(),
                skin: skin_index,
            });
        }
        let accessor = skin.inverse_bind_matrices().ok_or_else(|| {
            RiggedGltfError::MissingInverseBindMatrices {
                source_path: source_path.to_owned(),
                skin: skin_index,
            }
        })?;
        if accessor.count() != joint_count {
            return Err(RiggedGltfError::InverseBindCountMismatch {
                source_path: source_path.to_owned(),
                skin: skin_index,
                joint_count,
                inverse_bind_count: accessor.count(),
            });
        }
        let matrices = skin
            .reader(|_| Some(blob))
            .read_inverse_bind_matrices()
            .expect("validated inverse-bind accessor");
        if matrices
            .flatten()
            .flatten()
            .any(|component| !component.is_finite())
        {
            return Err(RiggedGltfError::InvalidInverseBindMatrix {
                source_path: source_path.to_owned(),
                skin: skin_index,
            });
        }
        skin_joint_counts.push(joint_count);
    }
    if skin_joint_counts.is_empty() {
        return Err(RiggedGltfError::MissingSkins {
            source_path: source_path.to_owned(),
        });
    }
    Ok(ValidatedRiggedGltf {
        gltf,
        skin_joint_counts,
    })
}
