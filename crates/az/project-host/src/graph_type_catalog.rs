use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use az_gem_contract::Registries;
use az_node_graph::{GraphTypeCatalog, GraphTypeCatalogError};
use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_project::{GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION, encode_graph_type_catalog_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphTypeCatalogPublishError {
    #[error("invalid graph type catalog: {0}")]
    Validation(#[from] GraphTypeCatalogError),

    #[error("failed to encode graph type catalog snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write graph type catalog side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("system clock is before Unix epoch")]
    ClockBeforeUnixEpoch,
}

#[derive(Debug, Clone)]
pub struct GraphTypeCatalogSideChannel {
    root: PathBuf,
}

impl GraphTypeCatalogSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compose the registered graph types and publish them as a snapshot file.
    ///
    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogPublishError::ClockBeforeUnixEpoch`] when the
    /// system clock predates the Unix epoch,
    /// [`GraphTypeCatalogPublishError::Validation`] when the composed catalog
    /// is rejected, and any error [`Self::write_catalog_snapshot`] returns.
    pub fn write_registered_snapshot(
        &self,
        registries: &Registries,
    ) -> Result<GraphTypeCatalogSideChannelFile, GraphTypeCatalogPublishError> {
        let catalog = GraphTypeCatalog::compose(
            GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION,
            now_unix_ms()?,
            registries,
        )?;
        self.write_catalog_snapshot(&catalog)
    }

    /// Publish `catalog` as a content-addressed side-channel snapshot file.
    ///
    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogPublishError::ClockBeforeUnixEpoch`] when the
    /// catalog carries no generation timestamp and the system clock predates
    /// the Unix epoch, [`GraphTypeCatalogPublishError::Validation`] when
    /// re-validating the catalog fails,
    /// [`GraphTypeCatalogPublishError::Encode`] when the snapshot cannot be
    /// serialized, and [`GraphTypeCatalogPublishError::Write`] when the staging
    /// file cannot be written.
    pub fn write_catalog_snapshot(
        &self,
        catalog: &GraphTypeCatalog,
    ) -> Result<GraphTypeCatalogSideChannelFile, GraphTypeCatalogPublishError> {
        let generated_unix_ms = if catalog.generated_unix_ms == 0 {
            now_unix_ms()?
        } else {
            catalog.generated_unix_ms
        };
        let catalog = GraphTypeCatalog::try_new(
            catalog.catalog_version,
            generated_unix_ms,
            catalog.graph_types.clone(),
        )?;
        let bytes = encode_graph_type_catalog_snapshot(&catalog)?;
        self.write_snapshot_bytes(&bytes)
    }

    fn write_snapshot_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<GraphTypeCatalogSideChannelFile, GraphTypeCatalogPublishError> {
        let written = write_content_addressed_staging_file(&self.root, "graph-type-catalog", bytes)
            .map_err(|error| GraphTypeCatalogPublishError::Write {
                path: error.path,
                source: error.source,
            })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        Ok(GraphTypeCatalogSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphTypeCatalogSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}

fn now_unix_ms() -> Result<u64, GraphTypeCatalogPublishError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GraphTypeCatalogPublishError::ClockBeforeUnixEpoch)?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}
