use std::path::{Path, PathBuf};

use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_runtime::{RuntimeLaunchSnapshot, encode_runtime_launch_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeLaunchSnapshotError {
    #[error("failed to encode runtime launch snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write runtime launch snapshot side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct RuntimeLaunchSnapshotSideChannel {
    root: PathBuf,
}

impl RuntimeLaunchSnapshotSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn default_root() -> PathBuf {
        std::env::temp_dir().join("azoth").join("runtime-launch")
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Publish `snapshot` as a content-addressed side-channel launch file.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeLaunchSnapshotError::Encode`] when the snapshot cannot
    /// be serialized, or [`RuntimeLaunchSnapshotError::Write`] when the staging
    /// file cannot be written under [`Self::root`].
    pub fn write_snapshot(
        &self,
        snapshot: &RuntimeLaunchSnapshot,
    ) -> Result<RuntimeLaunchSnapshotSideChannelFile, RuntimeLaunchSnapshotError> {
        let bytes = encode_runtime_launch_snapshot(snapshot)?;
        let written = write_content_addressed_staging_file(&self.root, "runtime-launch", &bytes)
            .map_err(|error| RuntimeLaunchSnapshotError::Write {
                path: error.path,
                source: error.source,
            })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        Ok(RuntimeLaunchSnapshotSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for RuntimeLaunchSnapshotSideChannel {
    fn default() -> Self {
        Self::new(Self::default_root())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunchSnapshotSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}
