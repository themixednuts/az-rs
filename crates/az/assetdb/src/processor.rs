//! Scheduler-facing facade over the `AssetDB` read model and single writer.
//!
//! Dispatch owns waiting, lease duration, renewal, and connection lifetime.
//! This module only exposes exact durable transitions with explicit fences.

use crate::AssetDb;
use crate::repo::{
    AbandonAttempts, AbandonAttemptsResult, AssetDbWriter, ClaimReadyJob, ClaimReadyJobResult,
    CompleteAttempt, CompleteAttemptResult, ProcessingStatus, RepoResult,
};
use crate::schema::SelectJobs;
use crate::value::Work;
use std::rc::Rc;

#[derive(Debug)]
pub struct AssetProcessorQueue {
    db: Rc<AssetDb>,
    writer: AssetDbWriter,
}

impl AssetProcessorQueue {
    pub const fn new(db: Rc<AssetDb>, writer: AssetDbWriter) -> Self {
        Self { db, writer }
    }

    /// # Errors
    ///
    /// Returns any error [`AssetDb::ready_jobs`] returns for this page query.
    pub fn ready_page(
        &self,
        workspace_pk: i64,
        kind: Work,
        after_job_id: i64,
        limit: u32,
    ) -> RepoResult<Vec<SelectJobs>> {
        self.db.ready_jobs(workspace_pk, kind, after_job_id, limit)
    }

    /// # Errors
    ///
    /// Returns any error the writer returns while claiming a ready job,
    /// including [`RepoError::WriterStopped`] if the writer has shut down.
    // `AssetProcessorQueue` holds an `Rc<AssetDb>`, so it is `!Sync` by construction and
    // this future cannot be `Send` without moving the queue from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub async fn claim(&self, input: ClaimReadyJob) -> RepoResult<ClaimReadyJobResult> {
        self.writer.claim_ready_job(input).await
    }

    /// # Errors
    ///
    /// Returns any error the writer returns while abandoning attempts,
    /// including [`RepoError::WriterStopped`] if the writer has shut down.
    // `AssetProcessorQueue` holds an `Rc<AssetDb>`, so it is `!Sync` by construction and
    // this future cannot be `Send` without moving the queue from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub async fn abandon(&self, input: AbandonAttempts) -> RepoResult<AbandonAttemptsResult> {
        self.writer.abandon_attempts(input).await
    }

    /// # Errors
    ///
    /// Returns any error the writer returns while completing the attempt,
    /// including [`RepoError::WriterStopped`] if the writer has shut down.
    // `AssetProcessorQueue` holds an `Rc<AssetDb>`, so it is `!Sync` by construction and
    // this future cannot be `Send` without moving the queue from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub async fn complete(&self, input: CompleteAttempt) -> RepoResult<CompleteAttemptResult> {
        self.writer.complete_attempt(input).await
    }

    /// # Errors
    ///
    /// Returns any error [`AssetDb::processing_status`] returns for this
    /// workspace and platform.
    pub fn status(
        &self,
        workspace_pk: i64,
        platform: Option<&str>,
    ) -> RepoResult<ProcessingStatus> {
        self.db.processing_status(workspace_pk, platform)
    }

    #[must_use]
    pub fn database(&self) -> &AssetDb {
        &self.db
    }
}
