//! `AssetDB` storage ownership and process-local status publication.
//!
//! Drizzle migrations are the only schema-version authority. The live process
//! owns one plain-WAL Turso database and one repository writer; derived runtime
//! handles share status publication state but never repeat migration.

use std::error::Error as StdError;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use drizzle::sqlite::pragma::{Pragma, Synchronous};
use thiserror::Error;

use crate::schema::AssetSchema;

pub const ASSET_DB_BUSY_TIMEOUT_MS: u64 = 10_000;
static PROCESS_MIGRATION_LOCK: Mutex<()> = Mutex::new(());

pub trait AssetDbFutureExt: Future + Sized {
    fn wait(self) -> Self::Output {
        futures_lite::future::block_on(self)
    }
}

impl<F: Future> AssetDbFutureExt for F {}

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("create parent directory for {path}: {source}")]
    ParentDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("asset database path is not valid UTF-8: {path}")]
    NonUtf8Path { path: String },
    #[error("{operation}: {source}")]
    Storage {
        operation: String,
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    #[error("apply AssetDB migrations: {0}")]
    Migration(String),
    #[error("close AssetDB while runtime or writer handles are still active")]
    ActiveHandles,
}

/// A live handle into the process-owned `AssetDB`.
pub struct AssetDb {
    local_database: az_turso::LocalDatabase,
    pub(crate) drizzle: drizzle::sqlite::turso::Drizzle<AssetSchema>,
    pub(crate) tables: AssetSchema,
    processing_changes: Arc<ProcessingChanges>,
    pub(crate) writer: Arc<OnceLock<crate::repo::AssetDbWriter>>,
    temporary_directory: Option<Arc<TemporaryDatabaseDirectory>>,
}

/// Keeps the file-backed test database alive across every process-local
/// connection. Turso's `:memory:` connections are isolated, so they cannot
/// exercise the production one-reader/one-writer composition.
#[derive(Debug)]
struct TemporaryDatabaseDirectory(std::path::PathBuf);

impl Drop for TemporaryDatabaseDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.0.display(),
                %error,
                "failed to remove temporary AssetDB test directory"
            );
        }
    }
}

impl std::fmt::Debug for AssetDb {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AssetDb").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct ProcessingChanges {
    sender: tokio::sync::watch::Sender<u64>,
}

impl Default for ProcessingChanges {
    fn default() -> Self {
        let (sender, _) = tokio::sync::watch::channel(0);
        Self { sender }
    }
}

impl ProcessingChanges {
    fn publish(&self) {
        let next = self.sender.borrow().saturating_add(1);
        self.sender.send_replace(next);
    }
}

pub struct AssetProcessingStatusSubscription {
    receiver: tokio::sync::watch::Receiver<u64>,
}

impl AssetProcessingStatusSubscription {
    #[must_use]
    pub fn revision(&self) -> u64 {
        *self.receiver.borrow()
    }

    pub async fn changed(&mut self) -> bool {
        self.receiver.changed().await.is_ok()
    }
}

impl AssetDb {
    /// # Errors
    ///
    /// Returns [`OpenError::ParentDir`] if the database's parent directory
    /// cannot be created, [`OpenError::NonUtf8Path`] if `path` is not valid
    /// UTF-8, and a storage error if the engine refuses the open, the deed is
    /// held by another process, or the startup pragmas cannot be applied.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        Self::open_path(path.as_ref(), false)
    }

    /// # Errors
    ///
    /// Returns the same failures as [`Self::open`]; with `enable_vacuum` it can
    /// also fail if a process-local owner for `path` was already opened without
    /// the vacuum feature.
    pub fn open_for_offline_maintenance(
        path: impl AsRef<Path>,
        enable_vacuum: bool,
    ) -> Result<Self, OpenError> {
        Self::open_path(path.as_ref(), enable_vacuum)
    }

    fn open_path(path: &Path, enable_vacuum: bool) -> Result<Self, OpenError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| OpenError::ParentDir {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let display = path.display().to_string();
        let path = path
            .to_str()
            .ok_or_else(|| OpenError::NonUtf8Path { path: display })?;
        Self::open_local(path, enable_vacuum, false)
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Errors
    ///
    /// Returns [`OpenError::ParentDir`] if the temporary directory cannot be
    /// created, [`OpenError::NonUtf8Path`] if its path is not valid UTF-8, and
    /// any error [`Self::open`] returns for the database inside it.
    pub fn open_in_memory() -> Result<Self, OpenError> {
        let directory = std::env::temp_dir().join(format!(
            "az-assetdb-test-{}",
            uuid::Uuid::now_v7().as_hyphenated()
        ));
        std::fs::create_dir_all(&directory).map_err(|source| OpenError::ParentDir {
            path: directory.display().to_string(),
            source,
        })?;
        let path = directory.join("assetdb.sqlite");
        let path_text = path.to_str().ok_or_else(|| OpenError::NonUtf8Path {
            path: path.display().to_string(),
        })?;
        let mut database = Self::open_local(path_text, false, true)?;
        database.temporary_directory = Some(Arc::new(TemporaryDatabaseDirectory(directory)));
        Ok(database)
    }

    fn open_local(
        path: &str,
        enable_vacuum: bool,
        create_current_schema: bool,
    ) -> Result<Self, OpenError> {
        let _migration_guard = PROCESS_MIGRATION_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let timeout = Duration::from_millis(ASSET_DB_BUSY_TIMEOUT_MS);
        let local_database = if enable_vacuum {
            az_turso::open_local_for_offline_maintenance(path, timeout, true).wait()
        } else {
            az_turso::open_local(path, timeout).wait()
        }
        .map_err(|source| storage("open Turso WAL AssetDB", source))?;
        let (mut drizzle, tables) = drizzle::sqlite::turso::Drizzle::new(
            local_database.connection().clone(),
            AssetSchema::new(),
        );
        apply_pragmas(&drizzle)?;
        if create_current_schema {
            drizzle
                .create()
                .wait()
                .map_err(|source| OpenError::Migration(format!("{source:?}")))?;
        } else {
            let migrations = drizzle::include_migrations!("./drizzle");
            drizzle
                .migrate(&migrations, drizzle::migrations::Tracking::SQLITE)
                .wait()
                .map_err(|source| OpenError::Migration(format!("{source:?}")))?;
        }
        Ok(Self {
            local_database,
            drizzle,
            tables,
            processing_changes: Arc::default(),
            writer: Arc::default(),
            temporary_directory: None,
        })
    }

    pub(crate) fn transaction_context(&self) -> drizzle::sqlite::turso::Drizzle<AssetSchema> {
        self.drizzle.clone()
    }

    /// # Errors
    ///
    /// Returns a storage error if the engine refuses another connection to the
    /// same database, or if the startup pragmas cannot be applied to it.
    pub fn new_runtime_handle(&self) -> Result<Self, OpenError> {
        self.new_handle(Arc::clone(&self.writer))
    }

    pub(crate) fn new_writer_handle(&self) -> Result<Self, OpenError> {
        self.new_handle(Arc::default())
    }

    fn new_handle(
        &self,
        writer: Arc<OnceLock<crate::repo::AssetDbWriter>>,
    ) -> Result<Self, OpenError> {
        let local_database = self
            .local_database
            .new_handle(Duration::from_millis(ASSET_DB_BUSY_TIMEOUT_MS))
            .map_err(|source| storage("open AssetDB runtime handle", source))?;
        let (drizzle, tables) = drizzle::sqlite::turso::Drizzle::new(
            local_database.connection().clone(),
            AssetSchema::new(),
        );
        apply_pragmas(&drizzle)?;
        Ok(Self {
            local_database,
            drizzle,
            tables,
            processing_changes: Arc::clone(&self.processing_changes),
            writer,
            temporary_directory: self.temporary_directory.clone(),
        })
    }

    pub(crate) fn publish_processing_change(&self) {
        self.processing_changes.publish();
    }

    pub fn subscribe_asset_processing_status(&self) -> AssetProcessingStatusSubscription {
        AssetProcessingStatusSubscription {
            receiver: self.processing_changes.sender.subscribe(),
        }
    }

    /// # Errors
    ///
    /// Returns a storage error if the engine refuses the WAL checkpoint,
    /// including when it reports the checkpoint as busy.
    pub fn checkpoint(&self) -> Result<(), OpenError> {
        self.local_database
            .checkpoint()
            .wait()
            .map_err(|source| storage("checkpoint AssetDB", source))
    }

    /// # Errors
    ///
    /// Returns a storage error if the engine refuses the vacuum, which includes
    /// a database opened without the vacuum feature.
    pub fn vacuum(&self) -> Result<(), OpenError> {
        self.local_database
            .vacuum()
            .wait()
            .map_err(|source| storage("vacuum AssetDB", source))
    }

    /// # Errors
    ///
    /// Returns a storage error if the `optimize` pragma cannot be executed.
    pub fn optimize(&self) -> Result<(), OpenError> {
        self.drizzle
            .execute(Pragma::Optimize(None))
            .wait()
            .map_err(|source| storage("optimize AssetDB", source))?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns a storage error if the `REINDEX` statement cannot be executed.
    pub fn reindex(&self) -> Result<(), OpenError> {
        self.drizzle
            .conn()
            .execute("REINDEX", ())
            .wait()
            .map_err(|source| storage("reindex AssetDB", source))?;
        Ok(())
    }

    /// Close the process-owned database before an offline file replacement.
    /// The operation fails closed while any derived runtime handle or writer
    /// client remains live.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::ActiveHandles`] if another runtime handle or a
    /// live writer client still references this database.
    pub fn close(self) -> Result<(), OpenError> {
        let Self {
            local_database,
            drizzle,
            tables: _,
            processing_changes: _,
            writer,
            temporary_directory: _,
        } = self;
        let writer = Arc::try_unwrap(writer).map_err(|_| OpenError::ActiveHandles)?;
        if let Some(writer) = writer.into_inner() {
            if writer.has_other_handles() {
                return Err(OpenError::ActiveHandles);
            }
            drop(writer);
        }
        drop(drizzle);
        drop(local_database);
        Ok(())
    }
}

fn apply_pragmas(db: &drizzle::sqlite::turso::Drizzle<AssetSchema>) -> Result<(), OpenError> {
    db.execute(Pragma::foreign_keys(true))
        .wait()
        .map_err(|source| storage("enable AssetDB foreign keys", source))?;
    db.execute(Pragma::CacheSize(-16_384))
        .wait()
        .map_err(|source| storage("configure AssetDB cache", source))?;
    db.execute(Pragma::Synchronous(Synchronous::Normal))
        .wait()
        .map_err(|source| storage("configure AssetDB durability", source))?;
    Ok(())
}

fn storage(
    operation: impl Into<String>,
    source: impl StdError + Send + Sync + 'static,
) -> OpenError {
    OpenError::Storage {
        operation: operation.into(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetDb, AssetDbFutureExt};
    use crate::{RegisterWorkspace, WorkspaceKey};

    #[test]
    fn file_backed_open_publishes_migrated_schema_to_the_writer_connection() {
        let temp = tempfile::tempdir().unwrap();
        let db = AssetDb::open(temp.path().join("assetdb.sqlite")).unwrap();
        let migrations = drizzle::include_migrations!("./drizzle");
        assert_eq!(
            migrations.len(),
            1,
            "the generated empty-database baseline is embedded as one fail-loud migration"
        );
        let mut schema_rows = db
            .drizzle
            .conn()
            .query(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspaces'",
                (),
            )
            .wait()
            .unwrap();
        assert_eq!(
            schema_rows
                .next()
                .wait()
                .unwrap()
                .unwrap()
                .get::<i64>(0)
                .unwrap(),
            1,
            "the bootstrap connection observes the migrated schema"
        );
        let writer = db.writer().unwrap();
        drop(db);

        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: "schema-visibility".to_owned(),
                    root: temp.path().to_string_lossy().into_owned(),
                    branch: "test".to_owned(),
                },
                now: 1,
            })
            .wait_blocking()
            .unwrap();

        assert!(workspace.workspace_id > 0);
    }
}
