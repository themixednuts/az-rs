use std::path::{Path, PathBuf};

use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_project::{GraphDocumentSnapshot, encode_graph_document_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphDocumentSnapshotPublishError {
    #[error("failed to encode graph document snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write graph document snapshot side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct GraphDocumentSideChannel {
    root: PathBuf,
}

impl GraphDocumentSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Publish `snapshot` as a content-addressed side-channel document file.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDocumentSnapshotPublishError::Encode`] when the snapshot
    /// cannot be serialized, or [`GraphDocumentSnapshotPublishError::Write`]
    /// when the staging file cannot be written under [`Self::root`].
    pub fn write_snapshot(
        &self,
        snapshot: &GraphDocumentSnapshot,
    ) -> Result<GraphDocumentSnapshotSideChannelFile, GraphDocumentSnapshotPublishError> {
        let bytes = encode_graph_document_snapshot(snapshot)?;
        let written =
            write_content_addressed_staging_file(&self.root, "graph-document-snapshot", &bytes)
                .map_err(|error| GraphDocumentSnapshotPublishError::Write {
                    path: error.path,
                    source: error.source,
                })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        Ok(GraphDocumentSnapshotSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphDocumentSnapshotSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}
