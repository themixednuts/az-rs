//! Offline-only retention and workspace reclamation operations.
//!
//! Live mutations use [`crate::AssetDbWriter`]. These operations are direct,
//! isolated transactions because the maintenance host owns the database deed
//! while the asset-processor service is stopped.

use drizzle::core::expr::{and, eq, is_not_null, lt, or};
use drizzle::sqlite::connection::SQLiteTransactionType;
use drizzle::sqlite::prelude::*;
use futures_lite::future::block_on;
use thiserror::Error;

use crate::connection::AssetDbFutureExt;
use crate::{AssetDb, SelectWorkspaces, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub finished_attempt_cutoff_unix_ms: i64,
    pub closed_path_cutoff_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionResult {
    pub deleted_attempts: u64,
    pub deleted_path_history_rows: u64,
}

#[derive(Debug, Error)]
pub enum MaintenanceError {
    #[error("AssetDB offline maintenance failed: {0}")]
    Storage(String),
}

// Passed directly to `Result::map_err`, which hands the error over by value, so this
// cannot take it by reference without wrapping every call site in a closure.
#[allow(clippy::needless_pass_by_value)]
fn storage(error: drizzle::error::DrizzleError) -> MaintenanceError {
    MaintenanceError::Storage(format!("{error:?}"))
}

impl AssetDb {
    /// Delete bounded scheduler and path history in one offline transaction.
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError::Storage`] if the engine refuses the
    /// transaction or any of its deletes.
    pub fn compact_operational_history(
        &self,
        policy: RetentionPolicy,
    ) -> Result<CompactionResult, MaintenanceError> {
        let tables = self.tables;
        let mut context = self.drizzle.clone();
        block_on(
            context.transaction(SQLiteTransactionType::Immediate, async |transaction| {
                let deleted_attempts = transaction
                    .delete(tables.attempts)
                    .r#where(and(
                        is_not_null(tables.attempts.finished),
                        and(
                            lt(
                                tables.attempts.finished,
                                Some(policy.finished_attempt_cutoff_unix_ms),
                            ),
                            or(
                                eq(tables.attempts.status, Status::Succeeded),
                                or(
                                    eq(tables.attempts.status, Status::Failed),
                                    eq(tables.attempts.status, Status::Abandoned),
                                ),
                            ),
                        ),
                    ))
                    .execute()
                    .await? as u64;
                let deleted_path_history_rows = transaction
                    .delete(tables.paths)
                    .r#where(and(
                        is_not_null(tables.paths.to),
                        lt(tables.paths.to, Some(policy.closed_path_cutoff_unix_ms)),
                    ))
                    .execute()
                    .await? as u64;
                Ok(CompactionResult {
                    deleted_attempts,
                    deleted_path_history_rows,
                })
            }),
        )
        .map_err(storage)
    }

    /// Return the complete, small workspace identity set for offline stale
    /// workspace inspection.
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError::Storage`] if the workspace query fails.
    pub fn workspaces_for_maintenance(&self) -> Result<Vec<SelectWorkspaces>, MaintenanceError> {
        self.drizzle
            .select(())
            .from(self.tables.workspaces)
            .order_by([asc(self.tables.workspaces.workspace_id)])
            .all()
            .wait()
            .map_err(storage)
    }

    /// Reclaim one stale workspace and every workspace-owned row through the
    /// schema's cascade graph. Product files remain untouched.
    ///
    /// # Errors
    ///
    /// Returns [`MaintenanceError::Storage`] if the engine refuses the
    /// transaction or the cascading delete.
    pub fn delete_workspace_for_maintenance(
        &self,
        workspace_id: i64,
    ) -> Result<bool, MaintenanceError> {
        let tables = self.tables;
        let mut context = self.drizzle.clone();
        block_on(
            context.transaction(SQLiteTransactionType::Immediate, async |transaction| {
                let deleted = transaction
                    .delete(tables.workspaces)
                    .r#where(eq(tables.workspaces.workspace_id, workspace_id))
                    .execute()
                    .await?;
                Ok(deleted == 1)
            }),
        )
        .map_err(storage)
    }
}
