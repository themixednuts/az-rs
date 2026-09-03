//! Editor-side asset-processor client.
//!
//! The editor only uses the read surface here. Worker lease and completion
//! calls stay behind dedicated asset-worker services.

use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserJobData, AssetBrowserJobStatus,
    AssetBuilderData, AssetBuilderPatternData, AssetBuilderPatternKindData,
    AssetSourceDependentJobData, AssetSourceDependentSourceData, AssetSourceFileTemplateData,
    AssetSourceFileWorkflowData, AssetSourceSchemaAuthoringData, AssetSourceSchemaData,
    CatalogProductData, ConsoleState, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetProcessorActivity, EditorAssetSourceDependentsPreview, EditorCatalogProductsStatus,
    EditorJobInspection, JobDependencyData, JobProductData, LogLevel, WorkspaceRootData,
    source_path_matches_extensions, validate_asset_db_relative_path,
};
use az_editor_ui::status::ServiceHealthStateData;
use az_proto_asset::{
    ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME,
    ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION, AssetBuilderCatalogRequest,
    AssetBuilderCatalogResult, AssetBuilderDescriptor, AssetBuilderPatternDescriptor,
    AssetBuilderPatternKind, AssetProcessorEvent, AssetProcessorEventKind,
    AssetProcessorEventSubscriptionRequest, AssetProcessorEventSubscriptionResult, AssetRootScope,
    AttemptStatus, CatalogProductEntry, CatalogProductsRequest, CatalogProductsResult,
    ForceReprocessAssetRequest, ForceReprocessAssetResult, InspectJobRequest, InspectJobResult,
    InspectJobSelector, JobActivity, JobAttemptRecord, JobDependencyKind, JobDependencyRecord,
    JobDependencyTarget, JobInspection, JobOwner, JobProductRecord, JobRecord, JobStatus,
    ReconcileAssetSourcesRequest, ReconcileAssetSourcesResult, SourceDependentJob,
    SourceDependentSource, SourceDependentsRequest, SourceDependentsResult,
    SourceFileCreateContent, SourceFileCreateRequest, SourceFileCreateResult,
    SourceFileDeleteRequest, SourceFileDeleteResult, SourceFileMoveRequest, SourceFileMoveResult,
    SourceSchemaAuthoring, SourceSchemaDescriptor, WorkspaceEntry, WorkspaceEntryPageRequest,
    WorkspaceEntryPageResult, WorkspaceRoot, WorkspaceSnapshot, WorkspaceSnapshotRequest,
    WorkspaceSnapshotResult, asset_capnp,
};
use az_proto_core::{
    Capability, ProtocolVersion, ServiceDescriptor, ServiceHealth, ServiceHealthState, ServiceId,
    ServiceRole,
};
use az_proto_session::{
    SessionManifest as ProtoSessionManifest, SessionWorkspaceStatus as ProtoSessionWorkspaceStatus,
};
use gpui::{App, Global};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Builder;
use tokio::sync::{
    mpsc::{Receiver, Sender, UnboundedSender, channel, unbounded_channel},
    oneshot, watch,
};
use tokio::task::LocalSet;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::attach::{AttachedWorkspace, EditorAttachSession};
use crate::error::{EditorError, EditorResult};
use crate::project_host::{EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE};
use crate::service_descriptor::validate_descriptor_capability_templates;
use crate::session_supervisor::SessionSupervisorClient;

pub const WORKSPACE_ENTRY_PAGE_SIZE: u32 = 512;
pub const ASSET_BROWSER_PAGE_SIZE: u32 = WORKSPACE_ENTRY_PAGE_SIZE;
pub const DEFAULT_ASSET_PRODUCT_PLATFORM: &str = "pc";
const ASSET_BROWSER_SNAPSHOT_REFRESH_TIMEOUT: Duration = Duration::from_mins(30);

/// Coalesced input edge for the animation catalog derived from editor state.
/// Asset-browser publication and preview-character selection are both inputs;
/// neither producer claims that the other one's state changed.
pub(crate) struct EditorAnimationCatalogInputInvalidation {
    revision: u64,
    changes: watch::Sender<u64>,
}

impl Default for EditorAnimationCatalogInputInvalidation {
    fn default() -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            revision: 0,
            changes,
        }
    }
}

impl Global for EditorAnimationCatalogInputInvalidation {}

pub(crate) fn publish_asset_browser_status(cx: &mut App, status: EditorAssetBrowserStatus) {
    cx.set_global(status);
    invalidate_animation_catalog_input(cx);
}

pub(crate) fn invalidate_animation_catalog_input(cx: &mut App) {
    let invalidation = cx.default_global::<EditorAnimationCatalogInputInvalidation>();
    invalidation.revision = invalidation.revision.saturating_add(1).max(1);
    invalidation.changes.send_replace(invalidation.revision);
}

pub(crate) fn subscribe_animation_catalog_input_invalidation(cx: &mut App) -> watch::Receiver<u64> {
    cx.default_global::<EditorAnimationCatalogInputInvalidation>()
        .changes
        .subscribe()
}

pub mod browser_controller;
pub mod client;
pub mod event_sink;
pub mod install;
pub(crate) mod jobs;
pub mod projection;
pub(crate) mod response_validation;
pub(crate) mod source_validation;
#[cfg(test)]
mod tests;

pub use browser_controller::*;
pub use client::*;
pub use event_sink::*;
pub use install::*;
pub(crate) use jobs::*;
pub use projection::*;
pub(crate) use response_validation::*;
pub(crate) use source_validation::*;
