use std::path::{Path, PathBuf};

use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_project::{GraphCommandStatusSnapshot, encode_graph_command_status_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphCommandStatusPublishError {
    #[error("failed to encode graph command status snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write graph command status side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct GraphCommandSideChannel {
    root: PathBuf,
}

impl GraphCommandSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Publish `status` as a content-addressed side-channel status file.
    ///
    /// # Errors
    ///
    /// Returns [`GraphCommandStatusPublishError::Encode`] when the snapshot
    /// cannot be serialized, or [`GraphCommandStatusPublishError::Write`] when
    /// the staging file cannot be written under [`Self::root`].
    pub fn write_status(
        &self,
        status: &GraphCommandStatusSnapshot,
    ) -> Result<GraphCommandStatusSideChannelFile, GraphCommandStatusPublishError> {
        let bytes = encode_graph_command_status_snapshot(status)?;
        let written =
            write_content_addressed_staging_file(&self.root, "graph-command-status", &bytes)
                .map_err(|error| GraphCommandStatusPublishError::Write {
                    path: error.path,
                    source: error.source,
                })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        Ok(GraphCommandStatusSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommandStatusSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}
