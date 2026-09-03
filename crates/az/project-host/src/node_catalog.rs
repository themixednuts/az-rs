use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use az_gem_contract::Registries;
use az_node_graph::{NodeTypeCatalog, NodeTypeCatalogError};
use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_project::{NODE_TYPE_CATALOG_SNAPSHOT_VERSION, encode_node_type_catalog_snapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeTypeCatalogPublishError {
    #[error("invalid node type catalog: {0}")]
    Validation(#[from] NodeTypeCatalogError),

    #[error("failed to encode node type catalog snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write node type catalog side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("system clock is before Unix epoch")]
    ClockBeforeUnixEpoch,
}

#[derive(Debug, Clone)]
pub struct NodeTypeCatalogSideChannel {
    root: PathBuf,
}

impl NodeTypeCatalogSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compose the registered node types and publish them as a snapshot file.
    ///
    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogPublishError::ClockBeforeUnixEpoch`] when the
    /// system clock predates the Unix epoch,
    /// [`NodeTypeCatalogPublishError::Validation`] when the composed catalog is
    /// rejected, and any error [`Self::write_catalog_snapshot`] returns.
    pub fn write_registered_snapshot(
        &self,
        registries: &Registries,
    ) -> Result<NodeTypeCatalogSideChannelFile, NodeTypeCatalogPublishError> {
        let catalog = NodeTypeCatalog::compose(
            NODE_TYPE_CATALOG_SNAPSHOT_VERSION,
            now_unix_ms()?,
            registries,
        )?;
        self.write_catalog_snapshot(&catalog)
    }

    /// Publish `catalog` as a content-addressed side-channel snapshot file.
    ///
    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogPublishError::ClockBeforeUnixEpoch`] when the
    /// catalog carries no generation timestamp and the system clock predates
    /// the Unix epoch, [`NodeTypeCatalogPublishError::Validation`] when
    /// re-validating the catalog fails,
    /// [`NodeTypeCatalogPublishError::Encode`] when the snapshot cannot be
    /// serialized, and [`NodeTypeCatalogPublishError::Write`] when the staging
    /// file cannot be written.
    pub fn write_catalog_snapshot(
        &self,
        catalog: &NodeTypeCatalog,
    ) -> Result<NodeTypeCatalogSideChannelFile, NodeTypeCatalogPublishError> {
        let generated_unix_ms = if catalog.generated_unix_ms == 0 {
            now_unix_ms()?
        } else {
            catalog.generated_unix_ms
        };
        let catalog = NodeTypeCatalog::try_new(
            catalog.catalog_version,
            generated_unix_ms,
            catalog.node_types.clone(),
        )?;
        let bytes = encode_node_type_catalog_snapshot(&catalog)?;
        self.write_snapshot_bytes(&bytes)
    }

    fn write_snapshot_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<NodeTypeCatalogSideChannelFile, NodeTypeCatalogPublishError> {
        let written = write_content_addressed_staging_file(&self.root, "node-type-catalog", bytes)
            .map_err(|error| NodeTypeCatalogPublishError::Write {
                path: error.path,
                source: error.source,
            })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        Ok(NodeTypeCatalogSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTypeCatalogSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}

fn now_unix_ms() -> Result<u64, NodeTypeCatalogPublishError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NodeTypeCatalogPublishError::ClockBeforeUnixEpoch)?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}
