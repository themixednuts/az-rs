//! `az-assetdb` — the typed, process-owned `AssetDB` repository.
//!
//! The public surface is a domain repository. Drizzle schema and connection
//! details remain private; all live mutations cross the single writer.

mod connection;
mod maintenance;
mod processor;
mod recovery;
mod repo;
mod schema;
mod value;

pub use connection::{AssetDb, AssetProcessingStatusSubscription, OpenError};
pub use maintenance::{CompactionResult, MaintenanceError, RetentionPolicy};
pub use processor::AssetProcessorQueue;
pub use recovery::{
    RecoveryExportError, export_unsaved_payloads_for_reset, validate_recovery_payloads,
};
pub use repo::{
    ATTEMPT_LIMIT_EXHAUSTED, AbandonAttempts, AbandonAttemptsResult, ApplyPlanDelta,
    ApplySweepDelta, AssetDbWriter, AttemptFence, BuilderCatalogReplaceOutcome, BuilderDescriptor,
    CatalogCursor, CatalogPage, CatalogProductEdge, CatalogTarget, CheckpointWrite, ClaimReadyJob,
    ClaimReadyJobResult, ClaimedJobContext, CompleteAttempt, CompleteAttemptResult, DeleteSource,
    DeleteSourceResult, DeletedSource, ExhaustedAttempt, ExpectedPayload, IdleFailedJob,
    ImportRecoveredPayloadResult, ImportUnsavedPayload, JobActivitySnapshot, JobEdgeInput,
    JobInspection, JobInspectionSelector, MAX_ASSET_JOB_ATTEMPTS, MoveSource, MoveSourceResult,
    MovedSource, PlanDelta, PlannedJob, PostCommitEffect, PostCommitEffectDrain,
    PostCommitEffectSubscription, ProcessingStatus, ProductEdgeInput, ProductInput,
    PublishAuthoredSource, PublishAuthoredSourceResult, PublishedAuthoredSource, RecoveredRoot,
    RecoveredWorkspace, RegisterWorkspace, RegisterWorkspaceRoot, ReplaceBuilderCatalog,
    ReplaceWorkspaceRoots, RepoError, RepoResult, ResolveIdleBlocked, ResolveIdleBlockedResult,
    SourceDependentJob, SourceDependentSource, SourceDependents, SourceDependentsInput,
    SourceEdgeInput, SourceStateToken, SweepDeltaResult, SweepEntry, SweepPlannerJob, SweepRecord,
    SweepRemoval, UNSATISFIABLE_DEPENDENCY, UnsavedPayload, WorkspaceEntrySnapshot, WorkspaceKey,
    WorkspaceRootBinding, WorkspaceRootRegistration, WriteSourcePayload, WriteSourcePayloadResult,
    WriterReply,
};
pub use schema::{
    SelectAssets, SelectAttempts, SelectBuilders, SelectCatalog, SelectEntries, SelectJobEdges,
    SelectJobs, SelectPayloads, SelectProductEdges, SelectProducts, SelectRoots, SelectSourceEdges,
    SelectWorkspaceRoots, SelectWorkspaces,
};
pub use value::{
    Aliases, Coupling, Diff, Digest, Encoding, Exclusions, InvalidDigest, InvalidTargetPath,
    Registration, Relation, Status, Target, TargetPath, Work,
};

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn wave_five_runtime_has_no_legacy_scheduler_or_scan_vocabulary() {
        let sources = [
            include_str!("repo.rs"),
            include_str!("processor.rs"),
            include_str!("connection.rs"),
        ]
        .join("\n");
        for forbidden in [
            "Started",
            "mark_started",
            "renew_lease",
            "poll_for",
            "scan_observation",
            "current_pointer",
            "healer",
            "cascade",
        ] {
            assert!(
                !sources.contains(forbidden),
                "Wave 5 must not reintroduce legacy `{forbidden}` APIs"
            );
        }
    }

    #[test]
    fn every_repository_transaction_reserves_the_writer_immediately() {
        let source = include_str!("repo.rs");
        assert_eq!(
            source.matches(".transaction(").count(),
            source
                .matches(".transaction(SQLiteTransactionType::Immediate")
                .count(),
            "every live repository transaction must be an isolated immediate writer barrier"
        );
    }

    #[test]
    fn optional_selects_decode_not_found_explicitly() {
        fn is_drizzle_select_get(binding: &str) -> bool {
            binding
                .find(".select(")
                .is_some_and(|select| binding[select..].contains(".get()"))
        }

        let source = include_str!("repo.rs");
        let mut remainder = source;
        while let Some(offset) = remainder.find("Option<Select") {
            let candidate = &remainder[offset..];
            let end = candidate
                .find(';')
                .expect("optional Select binding must end in the source file");
            let binding = &candidate[..=end];
            if binding.contains(".get()") {
                assert!(
                    binding.contains(".optional()"),
                    "Drizzle get() reports an absent row as NotFound; every Option<Select*> binding must use DrizzleOptionalExt: {binding}"
                );
            }
            remainder = &candidate[end + 1..];
        }

        let mut remainder = source;
        while let Some(offset) = remainder.find("if let Some") {
            let candidate = &remainder[offset..];
            let end = candidate
                .find('{')
                .expect("if-let binding must have a body in the source file");
            let binding = &candidate[..end];
            if is_drizzle_select_get(binding) {
                assert!(
                    binding.contains(".optional()"),
                    "inferred optional get binding must decode NotFound explicitly: {binding}"
                );
            }
            remainder = &candidate[end + 1..];
        }
    }
}
