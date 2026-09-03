//! Project-host service adapter.
//!
//! The host exposes reflected Prefab authoring through the vNext RPCs and keeps
//! the independent graph, `GameData`, runtime-launch, inventory, and health
//! services available on their existing ordinals.

pub mod source_authoring;
pub mod source_session;
pub mod type_registry;

#[cfg(any(test, feature = "test-support"))]
pub use source_authoring::UnavailableSourceAuthoringClient;
pub use source_authoring::{
    SourceAuthoringClient, SourceAuthoringClientError, SourceAuthoringRpcClient,
    SourceAuthoringSessionError, SourceAuthoringSessionResult, SourceAuthoringSessionService,
    SourceAuthoringSessionStatus,
};
pub use source_session::{
    FileSourcePersistence, NoSourceRecoveryStore, SourceCommitReceipt, SourceDiagnostic,
    SourceDiagnosticPhase, SourceSessionHost, SourceSessionHostError, TypedSourcePersistence,
    TypedSourcePolicy, TypedSourceRecoveryStore,
};
pub use type_registry::{
    ComposedTypeRegistry, PrefabManifestEntry, RegistryConsumer, RegistryManifest,
    RegistryValidationError, compose_type_registry, project_type_registry, registry_manifest,
    registry_manifest_for_consumer, validate_type_registry,
};

#[cfg(any(test, feature = "test-support"))]
use std::cell::Ref;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use az_asset_builder::BuildRuleRegistration;
use az_gem_contract::{
    ComposeError, ComposeReport, Composer, ProcessComposition, ProcessCompositionCleanupError,
    Registries, Registry, RegistryEntry, RegistryLease, RegistryLeaseError,
};
use az_node_graph::{
    GraphTypeCatalog, GraphTypeCatalogError, GraphTypeId, GraphTypeRegistration, NodeSourceLink,
    NodeTypeCatalog, NodeTypeCatalogError, NodeTypeRegistration, VisualGraphDocument,
    VisualGraphDocumentIoError, decode_visual_graph_document_ron, encode_visual_graph_document_ron,
};
use az_prefab::PrefabType;
use az_project::{
    GemTargetRole, LockedPackage, LockedPackageKind, PortableKey, ProjectLockStatus,
    ProjectManifestError, load_resolved_project_graph,
    load_resolved_project_graph_with_lock_status, project_lock_path,
    selected_gem_contributions_apply_to,
};
use az_proto_core::{Capability, CapabilityGrantSet, ServiceHealth, ServiceId, ServiceRole};
use az_proto_project::project_capnp;
pub use az_proto_project::{
    CreateGraphDocumentRequest, DocumentId, DocumentRevision, GAMEDATA_CATALOG_SNAPSHOT_VERSION,
    GameDataCatalogSnapshot, GraphCommandBatchRequest, GraphCommandBatchSnapshot,
    GraphCommandDiagnostic, GraphCommandStatusOutcome, GraphCommandStatusSnapshot,
    GraphDiagnosticSeverity, GraphDocumentSnapshot, NodeSourceLinkPathKind, NodeSourceLinkRequest,
    NodeSourceLinkTarget, PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION,
    PROJECT_EDIT_PERMISSION, PROJECT_GAMEDATA_PERMISSION, PROJECT_GRAPH_CATALOG_PERMISSION,
    PROJECT_HOST_AUDIENCE, PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
    PROJECT_INVENTORY_PERMISSION, PROJECT_NODE_CATALOG_PERMISSION,
    PROJECT_RUNTIME_LAUNCH_PERMISSION, PROJECT_SCHEMA_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION, ProjectDocumentRequest, ProjectHostCapabilityRequest,
    ProjectInventoryGem, ProjectInventoryGemKind, ProjectInventoryLockState,
    ProjectInventoryLockStatus, ProjectInventoryRegistryCounts, ProjectInventoryReport,
    RuntimeLaunchSnapshotRequest, SaveDocumentResult, SavedDocument,
    load_graph_command_batch_side_channel,
};
use az_proto_project::{FromCapnp as _, ToCapnp};
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RuntimeLaunchSnapshot, RuntimeResolvedGem,
};
use capnp::Error;
use thiserror::Error;
use tracing::{info, instrument};
use uuid::Uuid;

mod gamedata_catalog;
mod graph_commands;
mod graph_documents;
mod graph_type_catalog;
mod node_catalog;
mod runtime_launch;
mod transport;
mod vnext_host;

pub use gamedata_catalog::*;
pub use graph_commands::*;
pub use graph_documents::*;
pub use graph_type_catalog::*;
pub use node_catalog::*;
pub use runtime_launch::*;
pub use transport::*;
use vnext_host::VNextProjectHost;

/// One finalized ProjectHost-role composition owned for the RPC lifetime.
///
/// Construction consumes the mutable composer, validates it exactly once, and
/// rejects every other process role. Dropping this owner finishes and cleans
/// up every contribution through [`ProcessComposition`].
pub struct Composition(ProcessComposition);

impl Composition {
    /// Finalize and admit one `ProjectHost` composition.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectHostError::Compose`] when finalization fails,
    /// [`ProjectHostError::CompositionRole`] for a composer targeting another
    /// process role, or [`ProjectHostError::CompositionNotReady`] when a
    /// contribution has not reached readiness.
    pub fn new(composer: Composer) -> Result<Self, ProjectHostError> {
        let actual = composer.host().role();
        if actual != GemTargetRole::ProjectHost {
            composer.finish();
            composer.cleanup();
            return Err(ProjectHostError::CompositionRole { actual });
        }
        let composition = ProcessComposition::new(composer)?;
        if !composition.is_ready() {
            return Err(ProjectHostError::CompositionNotReady);
        }
        Ok(Self(composition))
    }

    /// The composed registries.
    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.0.registries()
    }

    /// The operator-visible compose report for this host.
    ///
    /// This is what replaced the link-time process inventory: the gems that
    /// reached this host are the ones that composed a contribution into it,
    /// named by the report, not by an anchor a linker happened to keep.
    ///
    #[must_use]
    pub const fn report(&self) -> &ComposeReport {
        self.0.report()
    }

    /// Composed Prefab types, when a contribution registered any.
    #[must_use]
    pub fn prefabs(&self) -> Option<&Registry<PrefabType>> {
        self.registries().get::<PrefabType>()
    }

    fn rpc_view(&self) -> Result<CompositionView, RegistryLeaseError> {
        Ok(CompositionView {
            registries: self.0.registry_lease()?,
            active_gems: self
                .0
                .report()
                .composed
                .iter()
                .map(|instance| instance.gem.to_string())
                .collect(),
        })
    }

    fn begin_shutdown(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.0.begin_shutdown()
    }

    fn cleanup(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.0.cleanup()
    }
}

impl std::fmt::Debug for Composition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Composition")
            .field("role", &self.0.role())
            .field("contributions", &self.0.report().composed.len())
            .finish()
    }
}

#[derive(Clone)]
struct CompositionView {
    registries: RegistryLease,
    active_gems: BTreeSet<String>,
}

impl CompositionView {
    #[must_use]
    fn registries(&self) -> &Registries {
        self.registries.registries()
    }

    #[must_use]
    fn prefabs(&self) -> Option<&Registry<PrefabType>> {
        self.registries().get::<PrefabType>()
    }

    #[must_use]
    const fn active_gems(&self) -> &BTreeSet<String> {
        &self.active_gems
    }
}

#[cfg(any(test, feature = "test-support"))]
fn test_project_host_composition() -> Composition {
    Composition::new(Composer::new(GemTargetRole::ProjectHost))
        .expect("empty ProjectHost test composition is valid and ready")
}

#[derive(Debug, Default)]
pub struct ProjectHost {
    graph_documents: BTreeMap<DocumentId, VisualGraphDocument>,
    graph_revisions: BTreeMap<DocumentId, DocumentRevision>,
    source_root: Option<PathBuf>,
    project_id: Option<String>,
}

impl ProjectHost {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_source_root(source_root: impl Into<PathBuf>) -> Self {
        Self {
            source_root: Some(source_root.into()),
            ..Self::default()
        }
    }

    /// Open a host rooted at `source_root`, normalizing it first.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectHostError::SourceRootNotAbsolute`] when `source_root`
    /// is relative, [`ProjectHostError::SourceRootCanonicalize`] when it cannot
    /// be canonicalized, or [`ProjectHostError::SourceRootNotDirectory`] when
    /// it does not name a directory.
    #[instrument(skip_all, fields(source_root = %source_root.as_ref().display()))]
    pub fn open_source_root(source_root: impl AsRef<Path>) -> Result<Self, ProjectHostError> {
        let source_root = normalize_source_root(source_root.as_ref().to_path_buf())?;
        info!(source_root = %source_root.display(), "opened vNext project-host source root");
        Ok(Self::with_source_root(source_root))
    }

    /// Open a host rooted at `source_root` and bind it to `project_id`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::open_source_root`] returns,
    /// [`ProjectHostError::ProjectManifest`] when the resolved project graph
    /// does not load, or [`ProjectHostError::SourceRootProjectIdMismatch`] when
    /// the manifest at that root names a different project.
    ///
    /// # Panics
    ///
    /// Panics if the source root just opened by [`Self::open_source_root`] is
    /// absent; that constructor always sets it.
    #[instrument(skip_all, fields(source_root = %source_root.as_ref().display(), project_id))]
    pub fn open_project_source_root(
        source_root: impl AsRef<Path>,
        project_id: &str,
    ) -> Result<Self, ProjectHostError> {
        let mut host = Self::open_source_root(source_root)?;
        let root = host.source_root.as_deref().expect("opened source root");
        let graph = load_resolved_project_graph(root)?;
        if graph.manifest.project.id != project_id {
            return Err(ProjectHostError::SourceRootProjectIdMismatch {
                source_root: root.to_path_buf(),
                expected: project_id.to_string(),
                actual: graph.manifest.project.id,
            });
        }
        host.project_id = Some(project_id.to_string());
        Ok(host)
    }

    #[must_use]
    pub fn source_root(&self) -> Option<&Path> {
        self.source_root.as_deref()
    }

    /// Resolve a graph node's source link to a concrete path under this root.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectHostError::NodeSourceLinkMissingTarget`] when `link`
    /// names neither a file nor a docs URL,
    /// [`ProjectHostError::NodeSourceLinkPathEscapes`] when the file contains
    /// `.`/`..` or resolves outside the source root,
    /// [`ProjectHostError::SourceRootRequired`] when this host has no source
    /// root, and [`ProjectHostError::ProjectManifest`] when the resolved
    /// project graph does not load.
    ///
    /// # Panics
    ///
    /// Panics if the source-link file vanishes between the `is_none` check
    /// above and the read below; `link` is borrowed immutably throughout.
    pub fn resolve_node_source_link(
        &self,
        link: &NodeSourceLink,
    ) -> Result<NodeSourceLinkTarget, ProjectHostError> {
        if link.file.as_deref().is_none() {
            if link.docs_url.as_deref().is_some() {
                return Ok(NodeSourceLinkTarget {
                    source_link: link.clone(),
                    resolved_path: None,
                    package_id: None,
                    package_root: None,
                    path_kind: NodeSourceLinkPathKind::DocsOnly,
                    exists: false,
                });
            }
            return Err(ProjectHostError::NodeSourceLinkMissingTarget);
        }

        let file = link
            .file
            .as_deref()
            .expect("checked graph source-link file");
        let file_path = Path::new(file);
        if has_unsafe_path_component(file_path) {
            return Err(ProjectHostError::NodeSourceLinkPathEscapes {
                file: file.to_string(),
            });
        }

        let source_root = self
            .source_root
            .as_ref()
            .ok_or(ProjectHostError::SourceRootRequired)?;
        let project_graph = load_resolved_project_graph(source_root)?;
        let package = link
            .package
            .as_deref()
            .and_then(|hint| find_locked_package(&project_graph.lock.packages, hint));

        let (resolved_path, package_id, package_root, path_kind) = if file_path.is_absolute() {
            if !file_path.starts_with(source_root) {
                return Err(ProjectHostError::NodeSourceLinkPathEscapes {
                    file: file.to_string(),
                });
            }
            (
                file_path.to_path_buf(),
                package.map(|package| package.id.clone()),
                package.map(|package| package.root.to_string_lossy().to_string()),
                NodeSourceLinkPathKind::Absolute,
            )
        } else if let Some(package) = package {
            (
                source_root.join(&package.root).join(file_path),
                Some(package.id.clone()),
                Some(package.root.to_string_lossy().to_string()),
                NodeSourceLinkPathKind::PackageRelative,
            )
        } else {
            (
                source_root.join(file_path),
                None,
                None,
                NodeSourceLinkPathKind::WorkspaceRelative,
            )
        };

        if !resolved_path.starts_with(source_root) {
            return Err(ProjectHostError::NodeSourceLinkPathEscapes {
                file: file.to_string(),
            });
        }
        Ok(NodeSourceLinkTarget {
            source_link: link.clone(),
            exists: resolved_path.is_file(),
            resolved_path: Some(resolved_path.to_string_lossy().to_string()),
            package_id,
            package_root,
            path_kind,
        })
    }

    pub(crate) fn create_graph_document(
        &mut self,
        document_id: DocumentId,
        graph_type: impl Into<String>,
        composition: &CompositionView,
    ) -> Result<DocumentRevision, ProjectHostError> {
        validate_document_id_path(&document_id)?;
        let graph_type = graph_type.into();
        if self.graph_documents.contains_key(&document_id)
            || self.graph_path(&document_id)?.is_file()
        {
            return Err(ProjectHostError::GraphDocumentAlreadyExists { document_id });
        }
        let descriptor = composed_graph_catalog(composition)?
            .graph_type(&GraphTypeId::new(graph_type.clone()))
            .cloned()
            .ok_or(ProjectHostError::UnknownGraphType { graph_type })?;
        self.insert_graph_document(document_id, descriptor.template.document, composition)?;
        Ok(DocumentRevision::new(0))
    }

    pub(crate) fn create_graph_document_snapshot(
        &mut self,
        document_id: &DocumentId,
        graph_type: impl Into<String>,
        composition: &CompositionView,
    ) -> Result<GraphDocumentSnapshot, ProjectHostError> {
        self.create_graph_document(document_id.clone(), graph_type, composition)?;
        self.graph_document_snapshot(document_id, composition)
    }

    pub(crate) fn insert_graph_document(
        &mut self,
        document_id: DocumentId,
        document: VisualGraphDocument,
        composition: &CompositionView,
    ) -> Result<Option<VisualGraphDocument>, ProjectHostError> {
        validate_document_id_path(&document_id)?;
        document.validate_against(&composed_node_catalog(composition)?)?;
        self.graph_revisions
            .entry(document_id.clone())
            .or_insert(DocumentRevision::new(0));
        Ok(self.graph_documents.insert(document_id, document))
    }

    #[must_use]
    pub fn graph_document(&self, id: &DocumentId) -> Option<&VisualGraphDocument> {
        self.graph_documents.get(id)
    }

    #[must_use]
    pub fn graph_document_revision(&self, id: &DocumentId) -> Option<DocumentRevision> {
        self.graph_revisions.get(id).copied()
    }

    pub(crate) fn graph_document_snapshot(
        &mut self,
        document_id: &DocumentId,
        composition: &CompositionView,
    ) -> Result<GraphDocumentSnapshot, ProjectHostError> {
        validate_document_id_path(document_id)?;
        if !self.graph_documents.contains_key(document_id) {
            self.load_graph_document(document_id, composition)?;
        }
        let revision = self.graph_document_revision(document_id).ok_or_else(|| {
            ProjectHostError::GraphDocumentNotFound {
                document_id: document_id.clone(),
            }
        })?;
        let document = self.graph_document(document_id).ok_or_else(|| {
            ProjectHostError::GraphDocumentNotFound {
                document_id: document_id.clone(),
            }
        })?;
        Ok(GraphDocumentSnapshot {
            document_id: document_id.clone(),
            revision,
            document: document.clone(),
        })
    }

    pub(crate) fn save_graph_document_record(
        &mut self,
        document_id: &DocumentId,
        composition: &CompositionView,
    ) -> Result<SavedDocument, ProjectHostError> {
        validate_document_id_path(document_id)?;
        if !self.graph_documents.contains_key(document_id) {
            self.load_graph_document(document_id, composition)?;
        }
        let document = self.graph_documents.get(document_id).ok_or_else(|| {
            ProjectHostError::GraphDocumentNotFound {
                document_id: document_id.clone(),
            }
        })?;
        let revision = self
            .graph_revisions
            .get(document_id)
            .copied()
            .ok_or_else(|| ProjectHostError::GraphDocumentNotFound {
                document_id: document_id.clone(),
            })?;
        let bytes = encode_visual_graph_document_ron(document)?.into_bytes();
        let path = self.graph_path(document_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ProjectHostError::GraphDocumentWrite {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, &bytes).map_err(|source| ProjectHostError::GraphDocumentWrite {
            path: path.clone(),
            source,
        })?;
        info!(document = %document_id.as_str(), revision = revision.0, "saved graph document");
        Ok(SavedDocument {
            document_id: document_id.clone(),
            revision,
            source_path: document_id.as_str().to_string(),
            schema_type: document.graph_type.clone(),
            content_hash: blake3::hash(&bytes).as_bytes().to_vec(),
            byte_length: bytes.len() as u64,
        })
    }

    pub(crate) fn apply_graph_command_batch(
        &mut self,
        batch: GraphCommandBatchSnapshot,
        composition: &CompositionView,
    ) -> Result<GraphCommandStatusSnapshot, ProjectHostError> {
        validate_document_id_path(&batch.document_id)?;
        if !self.graph_documents.contains_key(&batch.document_id) {
            self.load_graph_document(&batch.document_id, composition)?;
        }
        let current_revision = self
            .graph_revisions
            .get(&batch.document_id)
            .copied()
            .ok_or_else(|| ProjectHostError::GraphDocumentNotFound {
                document_id: batch.document_id.clone(),
            })?;
        if let Some(expected_revision) = batch.expected_revision
            && expected_revision != current_revision
        {
            return Ok(rejected_graph_command_status(
                &batch,
                format!(
                    "expected graph document revision {}, current revision is {}",
                    expected_revision.0, current_revision.0
                ),
            ));
        }

        let catalog = composed_node_catalog(composition)?;
        let apply_result = self
            .graph_documents
            .get_mut(&batch.document_id)
            .ok_or_else(|| ProjectHostError::GraphDocumentNotFound {
                document_id: batch.document_id.clone(),
            })?
            .apply_commands(batch.commands.clone(), &catalog);
        match apply_result {
            Ok(()) => {
                let revision =
                    DocumentRevision::new(current_revision.0.checked_add(1).ok_or_else(|| {
                        ProjectHostError::GraphRevisionOverflow {
                            document_id: batch.document_id.clone(),
                            revision: current_revision,
                        }
                    })?);
                let applied_command_count = u32::try_from(batch.commands.len()).map_err(|_| {
                    ProjectHostError::GraphCommandCountOutOfRange {
                        document_id: batch.document_id.clone(),
                        count: batch.commands.len(),
                    }
                })?;
                self.graph_revisions
                    .insert(batch.document_id.clone(), revision);
                Ok(GraphCommandStatusSnapshot {
                    document_id: batch.document_id,
                    client_batch_id: batch.client_batch_id,
                    applied_command_count,
                    outcome: GraphCommandStatusOutcome::Accepted { revision },
                    diagnostics: Vec::new(),
                })
            }
            Err(error) => Ok(rejected_graph_command_status(&batch, error.to_string())),
        }
    }

    #[instrument(skip_all)]
    fn discover_gamedata_tables(&self) -> Result<Vec<DiscoveredGameDataTable>, ProjectHostError> {
        let Some(source_root) = self.source_root.as_deref() else {
            return Ok(Vec::new());
        };
        let project_id = self
            .project_id
            .as_deref()
            .ok_or(ProjectHostError::ProjectIdentityRequired)?;
        let source_root_key = String::from(PortableKey::project_assets(project_id));
        let source_schema_type = gamedata::authoring::GAMEDATA_TABLE_SOURCE_SCHEMA
            .as_str()
            .to_string();
        let patterns = gamedata::authoring::table_source_patterns();
        let mut paths = Vec::new();
        collect_ron_source_paths(source_root, &mut paths)?;
        let mut tables = Vec::new();
        for path in paths {
            let relative = path.strip_prefix(source_root).map_err(|_| {
                ProjectHostError::SourcePathOutsideRoot {
                    path: path.clone(),
                    source_root: source_root.to_path_buf(),
                }
            })?;
            let source_path = relative.to_string_lossy().replace('\\', "/");
            if !patterns.iter().any(|pattern| pattern.matches(&source_path)) {
                continue;
            }
            let bytes = fs::read(&path).map_err(|source| ProjectHostError::SourceRead {
                path: path.clone(),
                source,
            })?;
            let Ok(source) = gamedata::authoring::decode_table_source_ron(&bytes) else {
                continue;
            };
            tables.push(DiscoveredGameDataTable {
                name: source.name().to_string(),
                row_type: source.schema().to_string(),
                source_root: az_asset_builder::PROJECT_SOURCE_ROOT.to_string(),
                source_root_key: source_root_key.clone(),
                source_schema_type: source_schema_type.clone(),
                source_path: source_path.clone(),
                document_id: source_path,
                owner: az_asset_builder::PROJECT_SOURCE_ROOT.to_string(),
                row_count: u64::try_from(source.rows().len()).unwrap_or(u64::MAX),
            });
        }
        tables.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.row_type.cmp(&right.row_type))
                .then_with(|| left.document_id.cmp(&right.document_id))
        });
        Ok(tables)
    }

    fn runtime_launch_snapshot(
        &self,
        request: &RuntimeLaunchSnapshotRequest,
    ) -> Result<RuntimeLaunchSnapshot, ProjectHostError> {
        validate_runtime_launch_context(request)?;
        let mut snapshot = RuntimeLaunchSnapshot::new(
            &request.project_id,
            request.session_id,
            &request.session_slug,
            request.role,
            &request.project_root,
            &request.workspace_path,
        );
        snapshot.workspace_id = request.workspace_id;
        snapshot.include_unsaved_journal = request.include_unsaved_journal;
        snapshot
            .asset_source_roots
            .clone_from(&request.asset_source_roots);
        snapshot
            .asset_package_roots
            .clone_from(&request.asset_package_roots);
        snapshot.launch_profile = if request.launch_profile.trim().is_empty() {
            "editor".to_string()
        } else {
            request.launch_profile.clone()
        };
        if let Some(resolution) = self.runtime_manifest_resolution()? {
            if resolution.project_id != request.project_id {
                return Err(ProjectHostError::InvalidRuntimeLaunchContext {
                    reason: format!(
                        "runtime launch project `{}` does not match source-root project manifest `{}`",
                        request.project_id, resolution.project_id
                    ),
                });
            }
            snapshot.resolved_gems = resolution.resolved_gems;
        }
        Ok(snapshot)
    }

    fn runtime_manifest_resolution(
        &self,
    ) -> Result<Option<RuntimeManifestResolution>, ProjectHostError> {
        let Some(source_root) = self.source_root.as_deref() else {
            return Ok(None);
        };
        let graph = load_resolved_project_graph(source_root)?;
        let resolved_gems = graph
            .gems
            .into_iter()
            .map(|gem| RuntimeResolvedGem {
                id: gem.manifest.gem.id,
                name: gem.manifest.gem.name,
                version: gem.manifest.gem.version,
                root: gem.root.to_string_lossy().into_owned(),
            })
            .collect();
        Ok(Some(RuntimeManifestResolution {
            project_id: graph.manifest.project.id,
            resolved_gems,
        }))
    }

    fn graph_path(&self, document_id: &DocumentId) -> Result<PathBuf, ProjectHostError> {
        let source_root = self
            .source_root
            .as_ref()
            .ok_or(ProjectHostError::SourceRootRequired)?;
        Ok(source_root.join(sanitize_document_id_path(document_id)?))
    }

    fn load_graph_document(
        &mut self,
        document_id: &DocumentId,
        composition: &CompositionView,
    ) -> Result<(), ProjectHostError> {
        let path = self.graph_path(document_id)?;
        let bytes = fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ProjectHostError::GraphDocumentNotFound {
                    document_id: document_id.clone(),
                }
            } else {
                ProjectHostError::GraphDocumentRead {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let source =
            std::str::from_utf8(&bytes).map_err(|source| ProjectHostError::GraphDocumentUtf8 {
                document_id: document_id.clone(),
                source,
            })?;
        let document = decode_visual_graph_document_ron(source)?;
        self.insert_graph_document(document_id.clone(), document, composition)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ProjectInventoryState {
    expected_gems: BTreeMap<String, ExpectedProjectInventoryGem>,
    lock_status: ProjectInventoryLockStatus,
    diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
struct ExpectedProjectInventoryGem {
    package: String,
    name: String,
    version: String,
    kind: ProjectInventoryGemKind,
    capabilities: Vec<String>,
}

impl ProjectInventoryState {
    fn from_host(host: &ProjectHost) -> Self {
        let Some(source_root) = host.source_root.as_deref() else {
            return Self {
                expected_gems: BTreeMap::new(),
                lock_status: ProjectInventoryLockStatus {
                    state: ProjectInventoryLockState::Unavailable,
                    path: String::new(),
                    diagnostic: "project-host was started without a project source root"
                        .to_string(),
                },
                diagnostics: vec![
                    "project inventory cannot resolve expected gems without a source root"
                        .to_string(),
                ],
            };
        };

        match load_resolved_project_graph_with_lock_status(source_root) {
            Ok(load) => {
                let project_service_gem_ids = load
                    .graph
                    .gems
                    .iter()
                    .filter(|gem| {
                        selected_gem_contributions_apply_to(
                            &gem.manifest,
                            &gem.declaration.capabilities,
                            GemTargetRole::ProjectHost,
                        )
                    })
                    .map(|gem| gem.manifest.gem.id.clone())
                    .collect::<BTreeSet<_>>();
                let mut expected_gems = BTreeMap::new();
                for package in load.graph.lock.packages {
                    let kind = match package.kind {
                        LockedPackageKind::ProjectGem => ProjectInventoryGemKind::Project,
                        LockedPackageKind::EngineGem => ProjectInventoryGemKind::Engine,
                        LockedPackageKind::RegistryGem | LockedPackageKind::Project => continue,
                    };
                    if !project_service_gem_ids.contains(package.id.as_str()) {
                        continue;
                    }
                    expected_gems.insert(
                        package.id,
                        ExpectedProjectInventoryGem {
                            package: package.name.clone(),
                            name: package.name,
                            version: package.version,
                            kind,
                            capabilities: package.capabilities,
                        },
                    );
                }
                let lock_status = project_inventory_lock_status(load.lock_status, source_root);
                let mut diagnostics = Vec::new();
                match lock_status.state {
                    ProjectInventoryLockState::Fresh => {}
                    ProjectInventoryLockState::Missing => diagnostics.push(
                        "project lock is missing; regenerate azoth.lock before relying on reproducible builds"
                            .to_string(),
                    ),
                    ProjectInventoryLockState::Stale => diagnostics.push(
                        "project lock is stale; regenerate azoth.lock and rebuild project services"
                            .to_string(),
                    ),
                    ProjectInventoryLockState::Unavailable => {
                        diagnostics.push(lock_status.diagnostic.clone());
                    }
                }
                Self {
                    expected_gems,
                    lock_status,
                    diagnostics,
                }
            }
            Err(error) => Self {
                expected_gems: BTreeMap::new(),
                lock_status: ProjectInventoryLockStatus {
                    state: ProjectInventoryLockState::Unavailable,
                    path: project_lock_path(source_root)
                        .to_string_lossy()
                        .into_owned(),
                    diagnostic: error.to_string(),
                },
                diagnostics: vec![format!(
                    "project inventory could not resolve expected gems: {error}"
                )],
            },
        }
    }

    fn report(
        &self,
        service_role: ServiceRole,
        composition: &CompositionView,
    ) -> ProjectInventoryReport {
        let mut diagnostics = self.diagnostics.clone();
        // A gem is present in this host when it composed a contribution into
        // it. There is no cargo package in a composed identity, so the
        // package-mismatch diagnostic the link-time inventory could raise has
        // no successor: the lock's expected package stays on the wire as
        // `expected_package`, and `package` is only what a composition can
        // actually name — nothing.
        let active_gems = composition.active_gems();
        let mut gem_ids = self.expected_gems.keys().cloned().collect::<BTreeSet<_>>();
        gem_ids.extend(active_gems.iter().cloned());

        let mut mismatch_count = 0usize;
        let gems = gem_ids
            .into_iter()
            .map(|id| {
                let expected = self.expected_gems.get(&id);
                let active = active_gems.contains(&id);
                if expected.is_some() && !active {
                    mismatch_count += 1;
                    diagnostics.push(format!(
                        "gem `{id}` is in the project graph but composed no contribution into this project-host"
                    ));
                } else if expected.is_none() && active {
                    mismatch_count += 1;
                    diagnostics.push(format!(
                        "gem `{id}` composed a contribution into this project-host but is not in the project graph"
                    ));
                }
                ProjectInventoryGem {
                    id,
                    expected_package: expected.map(|gem| gem.package.clone()).unwrap_or_default(),
                    name: expected.map(|gem| gem.name.clone()).unwrap_or_default(),
                    version: expected.map(|gem| gem.version.clone()).unwrap_or_default(),
                    kind: expected
                        .map_or(ProjectInventoryGemKind::Unknown, |gem| gem.kind),
                    expected: expected.is_some(),
                    active,
                    capabilities: expected
                        .map(|gem| gem.capabilities.clone())
                        .unwrap_or_default(),
                }
            })
            .collect();
        let degraded = mismatch_count > 0
            || !diagnostics.is_empty()
            || !matches!(self.lock_status.state, ProjectInventoryLockState::Fresh);
        ProjectInventoryReport {
            service_role: service_role.stable_name().to_string(),
            lock_status: self.lock_status.clone(),
            gems,
            registry: project_inventory_registry_counts(composition),
            degraded,
            diagnostics,
        }
    }
}

fn project_inventory_lock_status(
    status: ProjectLockStatus,
    source_root: &Path,
) -> ProjectInventoryLockStatus {
    match status {
        ProjectLockStatus::Fresh => ProjectInventoryLockStatus {
            state: ProjectInventoryLockState::Fresh,
            path: project_lock_path(source_root)
                .to_string_lossy()
                .into_owned(),
            diagnostic: String::new(),
        },
        ProjectLockStatus::Missing { path } => ProjectInventoryLockStatus {
            state: ProjectInventoryLockState::Missing,
            path: path.to_string_lossy().into_owned(),
            diagnostic: "project lock is missing".to_string(),
        },
        ProjectLockStatus::Stale { path } => ProjectInventoryLockStatus {
            state: ProjectInventoryLockState::Stale,
            path: path.to_string_lossy().into_owned(),
            diagnostic: "project lock does not match the current project graph".to_string(),
        },
        ProjectLockStatus::Unavailable { path, diagnostic } => ProjectInventoryLockStatus {
            state: ProjectInventoryLockState::Unavailable,
            path: path.to_string_lossy().into_owned(),
            diagnostic,
        },
    }
}

/// Registry counts no longer come from a process-global inventory: they are
/// whatever this host composed.
///
/// Only the three registries project-host may name are reported. Graph
/// compiler backends, AOT graph runtime entries, and runtime projections have
/// composed registries too, but they live in `az-graph-builder`,
/// `az-graph-runtime`, and `az-runtime-host` — crates the architecture guard
/// keeps out of project-host. A host cannot count entries of a type it must
/// not name, so the report no longer carries fields for them.
fn project_inventory_registry_counts(
    composition: &CompositionView,
) -> ProjectInventoryRegistryCounts {
    ProjectInventoryRegistryCounts {
        build_rules: composed_entries::<BuildRuleRegistration>(composition.registries()),
        node_types: composed_entries::<NodeTypeRegistration>(composition.registries()),
        graph_types: composed_entries::<GraphTypeRegistration>(composition.registries()),
    }
}

fn composed_entries<T: RegistryEntry>(registries: &Registries) -> u64 {
    registries
        .get::<T>()
        .map_or(0, |registry| registry.len() as u64)
}

struct RuntimeManifestResolution {
    project_id: String,
    resolved_gems: Vec<RuntimeResolvedGem>,
}

fn source_authoring_rpc_result(
    result: Result<SourceAuthoringSessionResult, SourceAuthoringSessionError>,
    fallback: SourceAuthoringSessionResult,
) -> az_proto_project::vnext::SourceAuthoringSessionResult {
    use az_proto_project::vnext::{
        SourceAuthoringFailure, SourceAuthoringFailureCode, SourceAuthoringSessionOutcome,
        SourceAuthoringSessionStatus,
    };

    let (session, outcome) = match result {
        Ok(session) => {
            let outcome = session.snapshot.clone().map_or(
                SourceAuthoringSessionOutcome::Closed,
                SourceAuthoringSessionOutcome::Snapshot,
            );
            (session, outcome)
        }
        Err(error) => {
            let (code, expected_revision, current_revision) = match &error {
                SourceAuthoringSessionError::NotOpen => (SourceAuthoringFailureCode::NotOpen, 0, 0),
                SourceAuthoringSessionError::RevisionConflict { expected, current } => (
                    SourceAuthoringFailureCode::RevisionConflict,
                    *expected,
                    *current,
                ),
                SourceAuthoringSessionError::HistoryEmpty { .. } => {
                    (SourceAuthoringFailureCode::HistoryEmpty, 0, 0)
                }
                SourceAuthoringSessionError::SourceMismatch => {
                    (SourceAuthoringFailureCode::SourceMismatch, 0, 0)
                }
                SourceAuthoringSessionError::Client(SourceAuthoringClientError::Unavailable(_)) => {
                    (SourceAuthoringFailureCode::Unavailable, 0, 0)
                }
                SourceAuthoringSessionError::Client(SourceAuthoringClientError::Transaction(_)) => {
                    (SourceAuthoringFailureCode::Transaction, 0, 0)
                }
            };
            (
                fallback,
                SourceAuthoringSessionOutcome::Failure(SourceAuthoringFailure {
                    code,
                    detail: error.to_string(),
                    expected_revision,
                    current_revision,
                }),
            )
        }
    };
    az_proto_project::vnext::SourceAuthoringSessionResult {
        status: SourceAuthoringSessionStatus {
            open: session.status.open,
            revision: session.status.revision,
            undo_depth: session.status.undo_depth,
            redo_depth: session.status.redo_depth,
        },
        outcome,
    }
}

/// Local Cap'n Proto RPC adapter for the project-host service.
pub struct ProjectHostRpc {
    host: RefCell<ProjectHost>,
    vnext: RefCell<VNextProjectHost>,
    source_authoring:
        Rc<tokio::sync::Mutex<SourceAuthoringSessionService<Box<dyn SourceAuthoringClient>>>>,
    composition: CompositionView,
    gamedata_catalog: GameDataCatalogSideChannel,
    node_type_catalog: NodeTypeCatalogSideChannel,
    graph_type_catalog: GraphTypeCatalogSideChannel,
    graph_commands: GraphCommandSideChannel,
    graph_documents: GraphDocumentSideChannel,
    runtime_launch_snapshots: RuntimeLaunchSnapshotSideChannel,
    service_project_id: Option<String>,
    service_session_id: Option<String>,
    service_run: Uuid,
    started_at: Instant,
    capability_grants: CapabilityGrantSet,
}

impl ProjectHostRpc {
    #[must_use]
    pub(crate) fn new(
        host: ProjectHost,
        side_channel_root: impl Into<PathBuf>,
        capability_grants: CapabilityGrantSet,
        composition: CompositionView,
        source_authoring: Box<dyn SourceAuthoringClient>,
    ) -> Self {
        Self::with_side_channel_root_and_session(
            host,
            side_channel_root,
            None,
            None,
            capability_grants,
            composition,
            source_authoring,
        )
    }

    /// # Panics
    ///
    /// Panics if the empty test composition cannot issue a registry lease,
    /// which would mean [`Composition::new`] admitted a composition that is not
    /// ready.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_new(host: ProjectHost, capability_grants: CapabilityGrantSet) -> Self {
        Self::new(
            host,
            default_project_host_side_channel_root(),
            capability_grants,
            test_project_host_composition()
                .rpc_view()
                .expect("test composition can issue a registry lease"),
            Box::new(UnavailableSourceAuthoringClient),
        )
    }

    /// # Panics
    ///
    /// Panics if `composition` cannot issue a registry lease, which would mean
    /// it was already shut down.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_new_composed(
        host: ProjectHost,
        capability_grants: CapabilityGrantSet,
        composition: &Composition,
    ) -> Self {
        Self::new(
            host,
            default_project_host_side_channel_root(),
            capability_grants,
            composition
                .rpc_view()
                .expect("test composition can issue a registry lease"),
            Box::new(UnavailableSourceAuthoringClient),
        )
    }

    /// The composed reflected registry this host serves.
    ///
    /// Reflected type *data* that no [`az_prefab::PrefabType`] can carry —
    /// an editor policy callback, for instance — is inserted here by the test
    /// harness that needs it, after composition.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn registry(&self) -> ComposedTypeRegistry {
        self.vnext.borrow().registry().clone()
    }

    #[must_use]
    pub(crate) fn for_project(
        host: ProjectHost,
        side_channel_root: impl Into<PathBuf>,
        project_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        composition: CompositionView,
        source_authoring: Box<dyn SourceAuthoringClient>,
    ) -> Self {
        Self::with_side_channel_root_and_session(
            host,
            side_channel_root,
            Some(project_id.into()),
            None,
            capability_grants,
            composition,
            source_authoring,
        )
    }

    #[must_use]
    pub(crate) fn for_session(
        host: ProjectHost,
        side_channel_root: impl Into<PathBuf>,
        session_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        composition: CompositionView,
        source_authoring: Box<dyn SourceAuthoringClient>,
    ) -> Self {
        Self::with_side_channel_root_and_session(
            host,
            side_channel_root,
            None,
            Some(session_id.into()),
            capability_grants,
            composition,
            source_authoring,
        )
    }

    #[must_use]
    pub(crate) fn for_project_session(
        host: ProjectHost,
        side_channel_root: impl Into<PathBuf>,
        project_id: impl Into<String>,
        session_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        composition: CompositionView,
        source_authoring: Box<dyn SourceAuthoringClient>,
    ) -> Self {
        Self::with_side_channel_root_and_session(
            host,
            side_channel_root,
            Some(project_id.into()),
            Some(session_id.into()),
            capability_grants,
            composition,
            source_authoring,
        )
    }

    fn with_side_channel_root_and_session(
        host: ProjectHost,
        root: impl Into<PathBuf>,
        service_project_id: Option<String>,
        service_session_id: Option<String>,
        capability_grants: CapabilityGrantSet,
        composition: CompositionView,
        source_authoring: Box<dyn SourceAuthoringClient>,
    ) -> Self {
        let root = root.into();
        let vnext = VNextProjectHost::compose(host.source_root(), composition.prefabs())
            .expect("composed Prefab registrations must compose");
        Self {
            host: RefCell::new(host),
            vnext: RefCell::new(vnext),
            source_authoring: Rc::new(tokio::sync::Mutex::new(SourceAuthoringSessionService::new(
                source_authoring,
            ))),
            composition,
            gamedata_catalog: GameDataCatalogSideChannel::new(root.join("gamedata-catalog")),
            node_type_catalog: NodeTypeCatalogSideChannel::new(root.join("node-type-catalog")),
            graph_type_catalog: GraphTypeCatalogSideChannel::new(root.join("graph-type-catalog")),
            graph_commands: GraphCommandSideChannel::new(root.join("graph-commands")),
            graph_documents: GraphDocumentSideChannel::new(root.join("graph-documents")),
            runtime_launch_snapshots: RuntimeLaunchSnapshotSideChannel::new(
                root.join("runtime-launch"),
            ),
            service_project_id,
            service_session_id,
            service_run: Uuid::nil(),
            started_at: Instant::now(),
            capability_grants,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn host(&self) -> Ref<'_, ProjectHost> {
        self.host.borrow()
    }

    #[must_use]
    fn service_session_id(&self) -> Option<&str> {
        self.service_session_id.as_deref()
    }

    #[must_use]
    fn service_project_id(&self) -> Option<&str> {
        self.service_project_id.as_deref()
    }

    #[must_use]
    pub const fn capability_grants(&self) -> &CapabilityGrantSet {
        &self.capability_grants
    }

    #[must_use]
    pub(crate) const fn with_service_run(mut self, run: Uuid) -> Self {
        self.service_run = run;
        self
    }

    #[must_use]
    pub(crate) fn into_client(self) -> project_capnp::project_host::Client {
        capnp_rpc::new_client(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> project_capnp::project_host::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }

    fn health_snapshot(&self) -> ServiceHealth {
        ServiceHealth::ready(
            ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            self.service_run,
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_message("project-host reachable")
    }
}

impl project_capnp::project_host::Server for ProjectHostRpc {
    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: project_capnp::project_host::HealthParams,
        mut results: project_capnp::project_host::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn type_registry_snapshot(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::TypeRegistrySnapshotParams,
        mut results: project_capnp::project_host::TypeRegistrySnapshotResults,
    ) -> Result<(), Error> {
        let capability = az_proto_core::Capability::from_capnp(params.get()?.get_capability()?)?;
        validate_registry_capability_for_session(
            &capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_registry_error_to_capnp(&error))?;
        let snapshot = self
            .vnext
            .borrow()
            .registry_snapshot()
            .map_err(|error| Error::failed(error.to_string()))?;
        (snapshot).to_capnp(results.get().init_snapshot())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn prefab_source_snapshot(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::PrefabSourceSnapshotParams,
        mut results: project_capnp::project_host::PrefabSourceSnapshotResults,
    ) -> Result<(), Error> {
        let request = params.get()?;
        let capability = az_proto_core::Capability::from_capnp(request.get_capability()?)?;
        validate_document_read_capability_for_session(
            &capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let source_path = request.get_source_path()?.to_string()?;
        let result = self.vnext.borrow().prefab_snapshot(&source_path);
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn apply_prefab_edit_command(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::ApplyPrefabEditCommandParams,
        mut results: project_capnp::project_host::ApplyPrefabEditCommandResults,
    ) -> Result<(), Error> {
        let request = params.get()?;
        let capability = az_proto_core::Capability::from_capnp(request.get_capability()?)?;
        validate_edit_capability_for_session(
            &capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let source_path = request.get_source_path()?.to_string()?;
        let command =
            az_proto_project::vnext::PrefabEditCommand::from_capnp(request.get_command()?)?;
        let result = self.vnext.borrow_mut().apply_edit(
            &source_path,
            request.get_expected_revision(),
            command,
        );
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn invoke_typed_action(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::InvokeTypedActionParams,
        mut results: project_capnp::project_host::InvokeTypedActionResults,
    ) -> Result<(), Error> {
        let request = params.get()?;
        let capability = az_proto_core::Capability::from_capnp(request.get_capability()?)?;
        validate_edit_capability_for_session(
            &capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let source_path = request.get_source_path()?.to_string()?;
        let target = az_proto_project::vnext::PrefabValueTarget::from_capnp(request.get_target()?)?;
        let action_id = request.get_action_id()?.to_string()?;
        let result = self.vnext.borrow_mut().invoke_action(
            &source_path,
            request.get_expected_revision(),
            target,
            action_id,
        );
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn prefab_diagnostics(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::PrefabDiagnosticsParams,
        mut results: project_capnp::project_host::PrefabDiagnosticsResults,
    ) -> Result<(), Error> {
        let request = params.get()?;
        let capability = az_proto_core::Capability::from_capnp(request.get_capability()?)?;
        validate_document_read_capability_for_session(
            &capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let source_path = request.get_source_path()?.to_string()?;
        let diagnostics = self.vnext.borrow().diagnostics(&source_path);
        let count = u32::try_from(diagnostics.len())
            .map_err(|_| Error::failed("diagnostic count exceeds the protocol u32 range".into()))?;
        let mut output = results.get().init_diagnostics(count);
        for (index, diagnostic) in (0..count).zip(&diagnostics) {
            (diagnostic).to_capnp(output.reborrow().get(index))?;
        }
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn source_session_lifecycle(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::SourceSessionLifecycleParams,
        mut results: project_capnp::project_host::SourceSessionLifecycleResults,
    ) -> Result<(), Error> {
        let request = params.get()?;
        let capability = az_proto_core::Capability::from_capnp(request.get_capability()?)?;
        let command =
            az_proto_project::vnext::SourceSessionCommand::from_capnp(request.get_command()?);
        match command {
            az_proto_project::vnext::SourceSessionCommand::Open
            | az_proto_project::vnext::SourceSessionCommand::Status => {
                validate_document_read_capability_for_session(
                    &capability,
                    self.service_session_id(),
                    self.capability_grants(),
                )
                .map_err(|error| project_host_source_error_to_capnp(&error))?;
            }
            _ => {
                validate_document_write_capability_for_session(
                    &capability,
                    self.service_session_id(),
                    self.capability_grants(),
                )
                .map_err(|error| project_host_source_error_to_capnp(&error))?;
            }
        }
        let source_path = request.get_source_path()?.to_string()?;
        let result = self.vnext.borrow_mut().lifecycle(
            &source_path,
            command,
            request.get_expected_revision(),
        );
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn source_authoring_session(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::SourceAuthoringSessionParams,
        mut results: project_capnp::project_host::SourceAuthoringSessionResults,
    ) -> Result<(), Error> {
        use az_proto_project::vnext::SourceAuthoringSessionCommand;

        let request = az_proto_project::vnext::SourceAuthoringSessionRequest::from_capnp(
            params.get()?.get_request()?,
        )?;
        match request.command {
            SourceAuthoringSessionCommand::Open | SourceAuthoringSessionCommand::Status => {
                validate_document_read_capability_for_session(
                    &request.capability,
                    self.service_session_id(),
                    self.capability_grants(),
                )
                .map_err(|error| project_host_source_error_to_capnp(&error))?;
            }
            _ => validate_document_write_capability_for_session(
                &request.capability,
                self.service_session_id(),
                self.capability_grants(),
            )
            .map_err(|error| project_host_source_error_to_capnp(&error))?,
        }
        validate_source_authoring_request_session(
            &request.session_id,
            &request.capability,
            self.service_session_id(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let source = request.source.clone();
        let session_id = request.session_id.clone();
        let mut service = self.source_authoring.lock().await;
        let result = match request.command {
            SourceAuthoringSessionCommand::Open => service.open(&session_id, source.clone()).await,
            SourceAuthoringSessionCommand::Apply(operation) => {
                service
                    .apply(
                        &session_id,
                        source.clone(),
                        request.expected_revision,
                        operation,
                    )
                    .await
            }
            SourceAuthoringSessionCommand::Undo => {
                service
                    .undo(&session_id, source.clone(), request.expected_revision)
                    .await
            }
            SourceAuthoringSessionCommand::Redo => {
                service
                    .redo(&session_id, source.clone(), request.expected_revision)
                    .await
            }
            SourceAuthoringSessionCommand::Close => {
                service
                    .close(&session_id, &source, request.expected_revision)
                    .await
            }
            SourceAuthoringSessionCommand::Status => Ok(service.status(&session_id, &source)),
        };
        let fallback = service.status(&session_id, &source);
        drop(service);
        source_authoring_rpc_result(result, fallback).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn gamedata_catalog(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::GamedataCatalogParams,
        mut results: project_capnp::project_host::GamedataCatalogResults,
    ) -> Result<(), Error> {
        let request = ProjectHostCapabilityRequest::from_capnp(params.get()?)?;
        validate_gamedata_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_gamedata_catalog_error_to_capnp(&error))?;
        let discovered_tables = self
            .host
            .borrow()
            .discover_gamedata_tables()
            .map_err(|error| project_host_gamedata_catalog_error_to_capnp(&error))?;
        let registries = self.composition.registries();
        let snapshot = self
            .gamedata_catalog
            .write_registered_snapshot(registries, &discovered_tables)
            .map_err(|error| gamedata_catalog_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&snapshot.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn node_type_catalog(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::NodeTypeCatalogParams,
        mut results: project_capnp::project_host::NodeTypeCatalogResults,
    ) -> Result<(), Error> {
        let request = ProjectHostCapabilityRequest::from_capnp(params.get()?)?;
        validate_node_catalog_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_node_catalog_error_to_capnp(&error))?;
        let registries = self.composition.registries();
        let snapshot = self
            .node_type_catalog
            .write_registered_snapshot(registries)
            .map_err(|error| node_type_catalog_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&snapshot.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn graph_type_catalog(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::GraphTypeCatalogParams,
        mut results: project_capnp::project_host::GraphTypeCatalogResults,
    ) -> Result<(), Error> {
        let request = ProjectHostCapabilityRequest::from_capnp(params.get()?)?;
        validate_graph_catalog_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_graph_catalog_error_to_capnp(&error))?;
        let registries = self.composition.registries();
        let snapshot = self
            .graph_type_catalog
            .write_registered_snapshot(registries)
            .map_err(|error| graph_type_catalog_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&snapshot.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn project_inventory(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::ProjectInventoryParams,
        mut results: project_capnp::project_host::ProjectInventoryResults,
    ) -> Result<(), Error> {
        let request = ProjectHostCapabilityRequest::from_capnp(params.get()?)?;
        validate_inventory_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_error_to_capnp(&error))?;
        let inventory_state = ProjectInventoryState::from_host(&self.host.borrow());
        let report = inventory_state.report(ServiceRole::ProjectHost, &self.composition);
        report.to_capnp((results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn resolve_node_source_link(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::ResolveNodeSourceLinkParams,
        mut results: project_capnp::project_host::ResolveNodeSourceLinkResults,
    ) -> Result<(), Error> {
        let request = NodeSourceLinkRequest::from_capnp(params.get()?)?;
        validate_source_navigation_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_source_navigation_error_to_capnp(&error))?;
        let target = self
            .host
            .borrow()
            .resolve_node_source_link(&request.source_link)
            .map_err(|error| project_host_source_navigation_error_to_capnp(&error))?;
        target.to_capnp(results.get())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn apply_graph_commands(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::ApplyGraphCommandsParams,
        mut results: project_capnp::project_host::ApplyGraphCommandsResults,
    ) -> Result<(), Error> {
        let request = GraphCommandBatchRequest::from_capnp(params.get()?)?;
        validate_edit_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_graph_commands_error_to_capnp(&error))?;
        let batch = load_graph_command_batch_side_channel(&request.batch)
            .map_err(|error| project_host_graph_command_side_channel_error_to_capnp(&error))?;
        let status = self
            .host
            .borrow_mut()
            .apply_graph_command_batch(batch, &self.composition)
            .map_err(|error| project_host_graph_commands_error_to_capnp(&error))?;
        let status = self
            .graph_commands
            .write_status(&status)
            .map_err(|error| graph_command_status_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&status.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn create_graph_document(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::CreateGraphDocumentParams,
        mut results: project_capnp::project_host::CreateGraphDocumentResults,
    ) -> Result<(), Error> {
        let request = CreateGraphDocumentRequest::from_capnp(params.get()?)?;
        validate_document_write_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        let snapshot = self
            .host
            .borrow_mut()
            .create_graph_document_snapshot(
                &request.document_id,
                request.graph_type,
                &self.composition,
            )
            .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        let snapshot = self
            .graph_documents
            .write_snapshot(&snapshot)
            .map_err(|error| graph_document_snapshot_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&snapshot.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn graph_document_snapshot(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::GraphDocumentSnapshotParams,
        mut results: project_capnp::project_host::GraphDocumentSnapshotResults,
    ) -> Result<(), Error> {
        let request = ProjectDocumentRequest::from_capnp(params.get()?)?;
        validate_document_read_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        let snapshot = self
            .host
            .borrow_mut()
            .graph_document_snapshot(&request.document_id, &self.composition)
            .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        let snapshot = self
            .graph_documents
            .write_snapshot(&snapshot)
            .map_err(|error| graph_document_snapshot_error_to_capnp(&error))?;
        ToCapnp::to_capnp(&snapshot.handle, (results.get(), &request.capability))?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn save_graph_document(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::SaveGraphDocumentParams,
        mut results: project_capnp::project_host::SaveGraphDocumentResults,
    ) -> Result<(), Error> {
        let request = ProjectDocumentRequest::from_capnp(params.get()?)?;
        validate_document_write_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        let saved = self
            .host
            .borrow_mut()
            .save_graph_document_record(&request.document_id, &self.composition)
            .map_err(|error| project_host_graph_document_error_to_capnp(&error))?;
        SaveDocumentResult {
            revision: saved.revision,
            saved,
        }
        .to_capnp(results.get())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC
    // stack.
    #[allow(clippy::future_not_send)]
    async fn runtime_launch_snapshot(
        self: capnp::capability::Rc<Self>,
        params: project_capnp::project_host::RuntimeLaunchSnapshotParams,
        mut results: project_capnp::project_host::RuntimeLaunchSnapshotResults,
    ) -> Result<(), Error> {
        let request = RuntimeLaunchSnapshotRequest::from_capnp(params.get()?.get_request()?)?;
        validate_runtime_launch_capability_for_session(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )
        .map_err(|error| project_host_runtime_launch_error_to_capnp(&error))?;
        validate_runtime_launch_snapshot_capability_for_session(
            &request.runtime_launch_capability,
            &request,
            self.service_session_id(),
        )
        .map_err(|error| project_host_runtime_launch_error_to_capnp(&error))?;
        validate_runtime_launch_project_scope(&request, self.service_project_id())
            .map_err(|error| project_host_runtime_launch_error_to_capnp(&error))?;
        let mut launch_snapshot = self
            .host
            .borrow()
            .runtime_launch_snapshot(&request)
            .map_err(|error| project_host_runtime_launch_error_to_capnp(&error))?;
        launch_snapshot.schema_catalog_hash = self
            .vnext
            .borrow()
            .schema_catalog_hash()
            .map_err(|error| Error::failed(error.to_string()))?;
        let snapshot = self
            .runtime_launch_snapshots
            .write_snapshot(&launch_snapshot)
            .map_err(ProjectHostError::from)
            .map_err(|error| project_host_runtime_launch_error_to_capnp(&error))?;
        ToCapnp::to_capnp(
            &snapshot.handle,
            (results.get(), &request.runtime_launch_capability),
        )?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ProjectHostError {
    #[error("graph document `{document_id:?}` already exists")]
    GraphDocumentAlreadyExists { document_id: DocumentId },

    #[error("graph document `{document_id:?}` was not found")]
    GraphDocumentNotFound { document_id: DocumentId },

    #[error("graph document `{document_id:?}` source payload is not UTF-8: {source}")]
    GraphDocumentUtf8 {
        document_id: DocumentId,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("graph document `{document_id:?}` revision {revision:?} cannot be advanced")]
    GraphRevisionOverflow {
        document_id: DocumentId,
        revision: DocumentRevision,
    },

    #[error("graph document `{document_id:?}` command count {count} does not fit protocol range")]
    GraphCommandCountOutOfRange {
        document_id: DocumentId,
        count: usize,
    },

    #[error("failed to read graph document {path}: {source}")]
    GraphDocumentRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write graph document {path}: {source}")]
    GraphDocumentWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid project-host protocol message: {0}")]
    Protocol(#[from] Error),

    #[error("visual graph document is invalid: {0}")]
    GraphDocumentValidation(#[from] az_node_graph::VisualGraphValidationError),

    #[error("visual graph document IO failed: {0}")]
    GraphDocumentIo(#[from] VisualGraphDocumentIoError),

    #[error("node type catalog failed: {0}")]
    NodeTypeCatalog(#[from] NodeTypeCatalogError),

    #[error("graph type catalog failed: {0}")]
    GraphTypeCatalog(#[from] GraphTypeCatalogError),

    #[error("project-host composition does not validate: {0}")]
    Compose(#[from] ComposeError),

    #[error(transparent)]
    RegistryLease(#[from] RegistryLeaseError),
    #[error(transparent)]
    Cleanup(#[from] ProcessCompositionCleanupError),

    #[error("project-host composition targets {actual:?}, expected ProjectHost")]
    CompositionRole { actual: GemTargetRole },

    #[error("project-host composition is not ready")]
    CompositionNotReady,

    #[error("graph type `{graph_type}` is not registered with project-host")]
    UnknownGraphType { graph_type: String },

    #[error("invalid project-host capability: {reason}")]
    InvalidCapability { reason: String },

    #[error("project manifest failed: {0}")]
    ProjectManifest(#[from] ProjectManifestError),

    #[error("project-host source root {source_root} is not absolute")]
    SourceRootNotAbsolute { source_root: PathBuf },

    #[error("failed to canonicalize project-host source root {source_root}: {source}")]
    SourceRootCanonicalize {
        source_root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("project-host source root {source_root} is not a directory")]
    SourceRootNotDirectory { source_root: PathBuf },

    #[error(
        "project-host source root {source_root} has project id `{actual}`, expected `{expected}`"
    )]
    SourceRootProjectIdMismatch {
        source_root: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("project-host source root is required")]
    SourceRootRequired,

    #[error("GameData catalog discovery requires the canonical project identity")]
    ProjectIdentityRequired,

    #[error("node source link does not declare a file or docs URL target")]
    NodeSourceLinkMissingTarget,

    #[error("node source link file `{file}` escapes the project-host source root")]
    NodeSourceLinkPathEscapes { file: String },

    #[error("document id `{document_id:?}` is not a relative source path")]
    InvalidDocumentPath { document_id: DocumentId },

    #[error("failed to scan project source path {path}: {source}")]
    SourceScan {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read project source {path}: {source}")]
    SourceRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("project source path {path} is outside root {source_root}")]
    SourcePathOutsideRoot { path: PathBuf, source_root: PathBuf },

    #[error("runtime launch snapshot failed: {0}")]
    RuntimeLaunchSnapshot(#[from] RuntimeLaunchSnapshotError),

    #[error("invalid runtime launch context: {reason}")]
    InvalidRuntimeLaunchContext { reason: String },
}

fn composed_node_catalog(
    composition: &CompositionView,
) -> Result<NodeTypeCatalog, ProjectHostError> {
    Ok(NodeTypeCatalog::compose(
        az_proto_project::NODE_TYPE_CATALOG_SNAPSHOT_VERSION,
        0,
        composition.registries(),
    )?)
}

fn composed_graph_catalog(
    composition: &CompositionView,
) -> Result<GraphTypeCatalog, ProjectHostError> {
    Ok(GraphTypeCatalog::compose(
        az_proto_project::GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION,
        0,
        composition.registries(),
    )?)
}

fn rejected_graph_command_status(
    batch: &GraphCommandBatchSnapshot,
    reason: String,
) -> GraphCommandStatusSnapshot {
    GraphCommandStatusSnapshot {
        document_id: batch.document_id.clone(),
        client_batch_id: batch.client_batch_id.clone(),
        applied_command_count: 0,
        outcome: GraphCommandStatusOutcome::Rejected {
            command_index: None,
            reason: reason.clone(),
        },
        diagnostics: vec![GraphCommandDiagnostic {
            command_index: None,
            severity: GraphDiagnosticSeverity::Error,
            message: reason,
        }],
    }
}

fn normalize_source_root(source_root: PathBuf) -> Result<PathBuf, ProjectHostError> {
    if !source_root.is_absolute() {
        return Err(ProjectHostError::SourceRootNotAbsolute { source_root });
    }
    let source_root = fs::canonicalize(&source_root).map_err(|source| {
        ProjectHostError::SourceRootCanonicalize {
            source_root: source_root.clone(),
            source,
        }
    })?;
    if !source_root.is_dir() {
        return Err(ProjectHostError::SourceRootNotDirectory { source_root });
    }
    Ok(source_root)
}

fn sanitize_document_id_path(document_id: &DocumentId) -> Result<PathBuf, ProjectHostError> {
    let mut clean = PathBuf::new();
    for part in validate_document_id_path(document_id)? {
        clean.push(part);
    }
    Ok(clean)
}

fn validate_document_id_path(document_id: &DocumentId) -> Result<Vec<&str>, ProjectHostError> {
    let value = document_id.as_str();
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.trim() != value
    {
        return Err(ProjectHostError::InvalidDocumentPath {
            document_id: document_id.clone(),
        });
    }
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(ProjectHostError::InvalidDocumentPath {
            document_id: document_id.clone(),
        });
    }
    Ok(parts)
}

fn collect_ron_source_paths(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), ProjectHostError> {
    let entries = fs::read_dir(directory).map_err(|source| ProjectHostError::SourceScan {
        path: directory.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ProjectHostError::SourceScan {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| ProjectHostError::SourceScan {
                path: path.clone(),
                source,
            })?;
        if file_type.is_dir() {
            if !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".azoth" | ".git" | "target")
            ) {
                collect_ron_source_paths(&path, paths)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ron"))
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn find_locked_package<'a>(
    packages: &'a [LockedPackage],
    package_hint: &str,
) -> Option<&'a LockedPackage> {
    let normalized_hint = package_hint.replace('_', "-");
    packages.iter().find(|package| {
        package.id == package_hint
            || package.name == package_hint
            || package.id.rsplit('.').next() == Some(package_hint)
            || package.id.rsplit('.').next() == Some(normalized_hint.as_str())
    })
}

fn has_unsafe_path_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(any(test, feature = "test-support"))]
fn default_project_host_side_channel_root() -> PathBuf {
    std::env::temp_dir().join("azoth").join("project-host")
}

fn project_host_source_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host source operation failed: {error}"))
}

fn project_host_registry_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host type registry failed: {error}"))
}

fn gamedata_catalog_error_to_capnp(error: &GameDataCatalogPublishError) -> Error {
    Error::failed(format!("project-host gamedataCatalog failed: {error}"))
}

fn project_host_gamedata_catalog_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host gamedataCatalog failed: {error}"))
}

fn node_type_catalog_error_to_capnp(error: &NodeTypeCatalogPublishError) -> Error {
    Error::failed(format!("project-host nodeTypeCatalog failed: {error}"))
}

fn project_host_node_catalog_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host nodeTypeCatalog failed: {error}"))
}

fn graph_type_catalog_error_to_capnp(error: &GraphTypeCatalogPublishError) -> Error {
    Error::failed(format!("project-host graphTypeCatalog failed: {error}"))
}

fn project_host_graph_catalog_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host graphTypeCatalog failed: {error}"))
}

fn graph_command_status_error_to_capnp(error: &GraphCommandStatusPublishError) -> Error {
    Error::failed(format!("project-host graph command status failed: {error}"))
}

fn project_host_graph_command_side_channel_error_to_capnp(
    error: &az_proto_project::GraphCommandBatchSideChannelError,
) -> Error {
    Error::failed(format!(
        "project-host applyGraphCommands batch side-channel failed: {error}"
    ))
}

fn project_host_graph_commands_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host applyGraphCommands failed: {error}"))
}

fn graph_document_snapshot_error_to_capnp(error: &GraphDocumentSnapshotPublishError) -> Error {
    Error::failed(format!(
        "project-host graph document snapshot failed: {error}"
    ))
}

fn project_host_graph_document_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!("project-host graph document failed: {error}"))
}

fn project_host_runtime_launch_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!(
        "project-host runtimeLaunchSnapshot failed: {error}"
    ))
}

fn project_host_source_navigation_error_to_capnp(error: &ProjectHostError) -> Error {
    Error::failed(format!(
        "project-host resolveNodeSourceLink failed: {error}"
    ))
}

fn validate_edit_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_EDIT_PERMISSION,
        &[ServiceRole::Editor, ServiceRole::ProjectHost],
        service_session_id,
        capability_grants,
    )
}

fn validate_registry_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_SCHEMA_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::RuntimeHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_gamedata_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_GAMEDATA_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::RuntimeHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_node_catalog_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_NODE_CATALOG_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::RuntimeHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_graph_catalog_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_GRAPH_CATALOG_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::RuntimeHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_inventory_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_INVENTORY_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::RuntimeHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_document_read_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_DOCUMENT_READ_PERMISSION,
        &[ServiceRole::Editor, ServiceRole::ProjectHost],
        service_session_id,
        capability_grants,
    )
}

fn validate_document_write_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_DOCUMENT_WRITE_PERMISSION,
        &[
            ServiceRole::Editor,
            ServiceRole::ProjectHost,
            ServiceRole::SessionSupervisor,
        ],
        service_session_id,
        capability_grants,
    )
}

fn validate_runtime_launch_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_RUNTIME_LAUNCH_PERMISSION,
        &[ServiceRole::Editor, ServiceRole::ProjectHost],
        service_session_id,
        capability_grants,
    )
}

fn validate_source_navigation_capability_for_session(
    capability: &Capability,
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    validate_project_host_capability(
        capability,
        PROJECT_SOURCE_NAVIGATION_PERMISSION,
        &[ServiceRole::Editor, ServiceRole::ProjectHost],
        service_session_id,
        capability_grants,
    )
}

fn validate_runtime_launch_snapshot_capability_for_session(
    capability: &Capability,
    request: &RuntimeLaunchSnapshotRequest,
    service_session_id: Option<&str>,
) -> Result<(), ProjectHostError> {
    capability
        .validate_lifetime()
        .map_err(|error| ProjectHostError::InvalidCapability {
            reason: error.to_string(),
        })?;
    if capability.service.namespace != PROJECT_HOST_NAMESPACE
        || capability.service.name != PROJECT_HOST_SERVICE_NAME
    {
        return Err(ProjectHostError::InvalidCapability {
            reason: format!(
                "runtime launch capability must be issued to `{PROJECT_HOST_NAMESPACE}/{PROJECT_HOST_SERVICE_NAME}`"
            ),
        });
    }
    if capability.role != ServiceRole::ProjectHost
        || capability.audience != RUNTIME_HOST_AUDIENCE
        || !capability
            .permissions
            .iter()
            .any(|permission| permission == RUNTIME_CONTROL_PERMISSION)
        || capability.session != Some(request.session_id)
    {
        return Err(ProjectHostError::InvalidCapability {
            reason: "runtime launch capability scope is invalid".to_string(),
        });
    }
    validate_service_session_scope(capability, service_session_id, RUNTIME_CONTROL_PERMISSION)
}

fn validate_runtime_launch_context(
    request: &RuntimeLaunchSnapshotRequest,
) -> Result<(), ProjectHostError> {
    if request.project_id.trim().is_empty()
        || request.session_slug.trim().is_empty()
        || request.project_root.trim().is_empty()
        || request.workspace_path.trim().is_empty()
    {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: "project, session, and workspace identity must be present".to_string(),
        });
    }
    if request.session_id.is_nil() {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: "session id cannot be nil".to_string(),
        });
    }
    if let Some(capability_session) = request.capability.session
        && capability_session != request.session_id
    {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: "runtime launch session does not match capability session".to_string(),
        });
    }
    if request.workspace_id <= 0 || request.asset_source_roots.is_empty() {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: "runtime launch requires a positive asset view and source roots".to_string(),
        });
    }
    let mut source_root_ids = BTreeSet::new();
    let mut portable_keys = BTreeSet::new();
    for root in &request.asset_source_roots {
        if root.workspace_id != request.workspace_id
            || root.workspace_root_id <= 0
            || root.root_id <= 0
            || root.owner_id.trim().is_empty()
            || root.source_root.trim().is_empty()
            || root.portable_key.trim().is_empty()
            || !source_root_ids.insert(root.workspace_root_id)
            || !portable_keys.insert(root.portable_key.as_str())
        {
            return Err(ProjectHostError::InvalidRuntimeLaunchContext {
                reason: format!("invalid asset source root `{}`", root.portable_key),
            });
        }
    }
    let project_assets_key = format!("project:{}:assets", request.project_id);
    let project_assets = request
        .asset_source_roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
        .ok_or_else(|| ProjectHostError::InvalidRuntimeLaunchContext {
            reason: format!("missing project assets root `{project_assets_key}`"),
        })?;
    let project_assets_path = PathBuf::from(&project_assets.source_root);
    let workspace_path = PathBuf::from(&request.workspace_path);
    // Clippy reads the field-name asymmetry as a typo and proposes
    // `request.owner_id`, which does not exist on `RuntimeLaunchSnapshotRequest`
    // and would not compile: the project-assets root is owned *by* the project,
    // so its `owner_id` is compared against the request's `project_id`.
    #[allow(clippy::suspicious_operation_groupings)]
    if project_assets.owner_id != request.project_id
        || project_assets_path == workspace_path
        || !project_assets_path.starts_with(&workspace_path)
        || !project_assets.is_root
        || !project_assets.output_prefix.is_empty()
    {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: format!("invalid project assets root `{project_assets_key}`"),
        });
    }
    az_proto_runtime::validate_runtime_asset_package_roots(&request.asset_package_roots).map_err(
        |error| ProjectHostError::InvalidRuntimeLaunchContext {
            reason: error.to_string(),
        },
    )
}

fn validate_runtime_launch_project_scope(
    request: &RuntimeLaunchSnapshotRequest,
    service_project_id: Option<&str>,
) -> Result<(), ProjectHostError> {
    if let Some(service_project_id) = service_project_id
        && request.project_id != service_project_id
    {
        return Err(ProjectHostError::InvalidRuntimeLaunchContext {
            reason: format!(
                "runtime launch project `{}` does not match service project `{service_project_id}`",
                request.project_id
            ),
        });
    }
    Ok(())
}

fn validate_project_host_capability(
    capability: &Capability,
    required_permission: &'static str,
    allowed_roles: &[ServiceRole],
    service_session_id: Option<&str>,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), ProjectHostError> {
    capability
        .validate_lifetime()
        .map_err(|error| ProjectHostError::InvalidCapability {
            reason: error.to_string(),
        })?;
    if capability.audience != PROJECT_HOST_AUDIENCE
        || !allowed_roles.contains(&capability.role)
        || !capability
            .permissions
            .iter()
            .any(|permission| permission == required_permission)
    {
        return Err(ProjectHostError::InvalidCapability {
            reason: format!("capability does not grant `{required_permission}`"),
        });
    }
    validate_service_session_scope(capability, service_session_id, required_permission)?;
    capability_grants
        .validate(capability, required_permission)
        .map_err(|error| ProjectHostError::InvalidCapability {
            reason: error.to_string(),
        })
}

fn validate_service_session_scope(
    capability: &Capability,
    service_session_id: Option<&str>,
    required_permission: &'static str,
) -> Result<(), ProjectHostError> {
    let Some(service_session_id) = service_session_id else {
        return Ok(());
    };
    let Some(capability_session) = capability.session else {
        return Err(ProjectHostError::InvalidCapability {
            reason: format!("missing session scope for `{required_permission}`"),
        });
    };
    if capability_session.to_string() != service_session_id {
        return Err(ProjectHostError::InvalidCapability {
            reason: format!("capability session cannot use `{required_permission}` here"),
        });
    }
    Ok(())
}

/// A project-scoped host still has to bind a generic authoring request to the
/// editor session that authorized it.  The brokered `ProjectHost` -> Asset
/// Processor capability is deliberately not part of this comparison.
fn validate_source_authoring_request_session(
    request_session_id: &str,
    capability: &Capability,
    service_session_id: Option<&str>,
) -> Result<(), ProjectHostError> {
    let request_session = uuid::Uuid::parse_str(request_session_id).map_err(|_| {
        ProjectHostError::InvalidCapability {
            reason: "source authoring session id must be a UUID".to_owned(),
        }
    })?;
    if request_session == uuid::Uuid::nil() || capability.session != Some(request_session) {
        return Err(ProjectHostError::InvalidCapability {
            reason: "source authoring request session does not match editor capability".to_owned(),
        });
    }
    if service_session_id.is_some_and(|session| session != request_session_id) {
        return Err(ProjectHostError::InvalidCapability {
            reason: "source authoring session id is outside this host session".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod architecture_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use az_gem_contract::{
        Contribution, ContributionDescriptor, ContributionId, GemContext, GemId, ProductActivation,
        declare_caps,
    };

    use super::*;

    declare_caps!(ProjectHostLifecycleCaps:);

    struct ProjectHostLifecycleContribution {
        ready: Arc<AtomicBool>,
        finished: Arc<AtomicUsize>,
        cleaned: Arc<AtomicUsize>,
    }

    impl Contribution for ProjectHostLifecycleContribution {
        type Caps = ProjectHostLifecycleCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.project-host-lifecycle-test"),
                contribution: ContributionId::new("project-host"),
                roles: &[GemTargetRole::ProjectHost],
            }
        }

        fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}

        fn ready(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }

        fn finish(&self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn cleanup(&self) {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FinalizeFailingAssetWorkerContribution;

    impl Contribution for FinalizeFailingAssetWorkerContribution {
        type Caps = ProjectHostLifecycleCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.project-host-role-test"),
                contribution: ContributionId::new("unresolved-clock"),
                roles: &[GemTargetRole::AssetWorker],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
            ctx.registrar::<az_gem_contract::ClockUse>()
                .register(az_gem_contract::ClockUse {
                    clock: az_gem_contract::ClockName::new("azoth.unresolved"),
                    system: "role-check",
                });
        }
    }

    fn lifecycle_composer(
        role: GemTargetRole,
        ready: bool,
    ) -> (Composer, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(role);
        composer
            .add(
                ProjectHostLifecycleContribution {
                    ready: Arc::new(AtomicBool::new(ready)),
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .expect("test contribution has no capability floor");
        (composer, finished, cleaned)
    }

    /// The engine bundles a `project-host` role names, ahead of any fixture.
    ///
    /// One helper keeps this crate's harnesses aligned with
    /// `az_engine_ids::contributions`. It remains local because each host's
    /// contribution list also declares its native link surface; a cross-host
    /// helper would link bundles that this role does not use.
    ///
    /// Production receives the same role-fixed composition from its generated
    /// entrypoint. The fixture includes the complete engine floor so tests
    /// exercise the production registry and link surface.
    pub fn floor(composer: &mut Composer) {
        for outcome in [
            composer.floor(az_engine_types::types_contribution()),
            composer.floor(az_engine_assets::assets_contribution()),
            composer.floor(az_engine_builders::builders_contribution()),
        ] {
            outcome.expect("the engine floor declares no host-capability floor");
        }
    }

    const TEST_BROKERED_TOKEN: [u8; 2] = [0x70, 0x68];

    fn test_session_id() -> uuid::Uuid {
        uuid::Uuid::from_bytes([0x44; 16])
    }

    fn test_capability(permissions: impl IntoIterator<Item = &'static str>) -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_session(test_session_id())
            .with_permissions(permissions)
            .with_token_hash(TEST_BROKERED_TOKEN)
    }

    fn test_capability_grants() -> CapabilityGrantSet {
        CapabilityGrantSet::from_grants(vec![test_capability([
            PROJECT_EDIT_PERMISSION,
            PROJECT_SCHEMA_PERMISSION,
            PROJECT_GAMEDATA_PERMISSION,
            PROJECT_NODE_CATALOG_PERMISSION,
            PROJECT_GRAPH_CATALOG_PERMISSION,
            PROJECT_INVENTORY_PERMISSION,
            PROJECT_DOCUMENT_READ_PERMISSION,
            PROJECT_DOCUMENT_WRITE_PERMISSION,
            PROJECT_RUNTIME_LAUNCH_PERMISSION,
            PROJECT_SOURCE_NAVIGATION_PERMISSION,
        ])])
    }

    fn write_test_capability(
        builder: az_proto_core::core_capnp::capability::Builder<'_>,
        permissions: impl IntoIterator<Item = &'static str>,
    ) {
        (test_capability(permissions)).to_capnp(builder).unwrap();
    }

    #[test]
    fn source_authoring_rejects_a_session_not_owned_by_the_editor_capability() {
        let other_session = uuid::Uuid::from_bytes([0x45; 16]);
        let error = validate_source_authoring_request_session(
            &other_session.to_string(),
            &test_capability([PROJECT_DOCUMENT_READ_PERMISSION]),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, ProjectHostError::InvalidCapability { .. }));
    }

    #[test]
    fn composition_rejects_another_process_role_and_cleans_up() {
        let (composer, finished, cleaned) = lifecycle_composer(GemTargetRole::AssetWorker, true);

        assert!(matches!(
            Composition::new(composer),
            Err(ProjectHostError::CompositionRole {
                actual: GemTargetRole::AssetWorker
            })
        ));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn composition_rejects_another_role_before_finalization_errors() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(
                FinalizeFailingAssetWorkerContribution,
                ProductActivation::default(),
            )
            .expect("fixture contribution composes before finalization");

        assert!(matches!(
            Composition::new(composer),
            Err(ProjectHostError::CompositionRole {
                actual: GemTargetRole::AssetWorker
            })
        ));
    }

    #[test]
    fn composition_rejects_not_ready_contributions_and_cleans_up() {
        let (composer, finished, cleaned) = lifecycle_composer(GemTargetRole::ProjectHost, false);

        assert!(matches!(
            Composition::new(composer),
            Err(ProjectHostError::CompositionNotReady)
        ));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn composition_owner_outlives_the_rpc_registry_view() {
        let (composer, finished, cleaned) = lifecycle_composer(GemTargetRole::ProjectHost, true);
        let composition = Composition::new(composer).unwrap();
        let rpc = ProjectHostRpc::test_new_composed(
            ProjectHost::default(),
            test_capability_grants(),
            &composition,
        );

        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);
        drop(rpc);
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);
        drop(composition);
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    mod vnext_parity;
}
