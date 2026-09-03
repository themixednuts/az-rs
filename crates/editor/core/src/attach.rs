//! Session/workspace attach orchestration for the universal editor.
//!
//! This is the editor-owned attach path: discover the session-supervisor,
//! resolve project services through it, then prove resolved descriptors are
//! live Cap'n Proto endpoints before the UI starts normal tools.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use az_endpoint_discovery::{
    read_project_daemon_endpoint_record, remove_project_daemon_endpoint_record,
};
use az_filesystem::normalize;
use az_proto_asset::{
    ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME, AssetRootScope, WorkspaceRoot,
    WorkspaceSnapshot,
};
use az_proto_core::{Endpoint, ServiceDescriptor, ServiceId, ServiceRole};
use az_proto_daemon::ProjectRecord;
use az_proto_project::{
    GameDataCatalogSnapshot, PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
    ProjectInventoryReport, vnext::TypeRegistrySnapshot,
};
use az_proto_runtime::{RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME};
use az_proto_session as session_proto;
use az_service_supervision::ProcessIdentity;
use az_source_control::{LoreCli, SourceControlProvider, SourceStatus};
use gpui::Global;
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::asset_processor::AssetProcessorClient;
use crate::daemon::{AzDaemonClient, ensure_daemon_endpoint_is_local};
use crate::daemon_bootstrap::{is_daemon_transport_failure, probe_daemon_endpoint_record};
use crate::error::{EditorError, EditorResult};
use crate::project_host::ProjectHostClient;
use crate::runtime_host::RuntimeHostClient;
use crate::session_supervisor::SessionSupervisorClient;

const ATTACH_REQUIRED_RPC_TIMEOUT: Duration = Duration::from_secs(10);
const ATTACH_OPTIONAL_RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct EditorAttachSession {
    pub project_id: String,
    pub project_root: PathBuf,
    pub daemon_endpoint: Endpoint,
    pub session_id: Uuid,
    pub session_slug: String,
    pub workspace: AttachedWorkspace,
    pub source_status: SourceStatus,
    pub run_dir: PathBuf,
    pub session_supervisor: ServiceDescriptor,
    pub services: EditorAttachServices,
    pub workspace_snapshot: WorkspaceSnapshot,
    pub type_registry: TypeRegistrySnapshot,
    pub gamedata_catalog: GameDataCatalogSnapshot,
    pub project_inventory: ProjectInventoryReport,
    pub verification: EditorAttachVerification,
}

impl Global for EditorAttachSession {}

/// Editor-owned identity resolved from the session manifest and source control.
///
/// This is deliberately not an `AssetDB` or asset-service request key: the
/// asset service derives its workspace from the session capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedWorkspace {
    pub project_id: String,
    pub workspace_root: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAttachServices {
    pub project_host: ServiceDescriptor,
    pub asset_processor: ServiceDescriptor,
    pub runtime_host: Option<ServiceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAttachVerification {
    pub project_host_type_count: usize,
    pub asset_processor_workspace_id: i64,
    pub asset_processor_root_count: usize,
    pub runtime_host_verified: bool,
    pub runtime_host_projection_count: Option<usize>,
    pub project_inventory_expected_gem_count: usize,
    pub project_inventory_active_gem_count: usize,
    pub project_inventory_diagnostics_count: usize,
    pub project_inventory_degraded: bool,
}

#[derive(Debug, Clone)]
struct EditorAttachProbe {
    type_registry: TypeRegistrySnapshot,
    gamedata_catalog: GameDataCatalogSnapshot,
    project_inventory: ProjectInventoryReport,
    workspace_snapshot: WorkspaceSnapshot,
    verification: EditorAttachVerification,
}

/// Attach to a session through the azd named by this project's endpoint record.
///
/// The gate order is bootstrap's, for bootstrap's reasons: locality is a trust
/// gate on untrusted project state, so nothing dials ahead of it, and
/// reachability then the record's protocol claim follow together on
/// [`probe_daemon_endpoint_record`], which owns why they run in that order.
///
/// Attach's *consequence* is what differs, and stays here: it starts no
/// sidecar, so a record whose daemon is gone is evicted and the failure
/// surfaced to the caller rather than recovered from.
///
/// # Errors
///
/// Returns [`EditorError::ServiceDiscovery`] when no record exists, or when a
/// live daemon answers under a record claiming another protocol build — the
/// record is left on disk, since a version conflict is the user's to resolve.
/// Returns the underlying transport error when the recorded endpoint is
/// unreachable or fails mid-attach, having first evicted the record so the next
/// attempt can rediscover a live daemon. Anything the attach itself rejects
/// (session state, service verification) is surfaced unchanged.
pub fn attach_to_session(
    project_root: impl AsRef<Path>,
    requested_session: Option<&str>,
) -> EditorResult<EditorAttachSession> {
    let project_root = project_root.as_ref().to_path_buf();
    let record = read_project_daemon_endpoint_record(&project_root).map_err(|error| {
        EditorError::ServiceDiscovery(format!("failed to read azd endpoint record: {error}"))
    })?;
    let Some(record) = record else {
        return Err(EditorError::ServiceDiscovery(
            "editor session attach requires a reachable azd endpoint; start azd or pass --daemon-endpoint"
                .to_string(),
        ));
    };
    ensure_daemon_endpoint_is_local(&record.endpoint)?;

    // The probe runs before the record's protocol claim so a foreign-protocol
    // record whose daemon is gone reaches the eviction arm below instead of
    // failing ahead of it; while it failed first, nothing on this path could
    // ever clear such a record and it had to be deleted by hand. It also bounds
    // first contact: `attach_to_session_via_daemon` has no deadline of its own.
    let attached = probe_daemon_endpoint_record(&record).and_then(|()| {
        attach_to_session_via_daemon(&project_root, requested_session, record.endpoint.clone())
    });
    match attached {
        Ok(session) => Ok(session),
        Err(error) if is_daemon_transport_failure(&error) => {
            warn!(
                error = %error,
                "azd runtime endpoint record was stale; removing record"
            );
            remove_project_daemon_endpoint_record(&project_root).map_err(|remove_error| {
                EditorError::ServiceDiscovery(format!(
                    "failed to remove stale azd endpoint record after attach failure: {remove_error}"
                ))
            })?;
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// Attach the editor to a daemon-registered session, resolving the project
/// record, session manifest, workspace and project services in one pass.
///
/// # Errors
///
/// Returns [`EditorError::GeneratedProjectTargets`] if the project's generated
/// targets cannot be synchronized, [`EditorError::Io`] if the current-thread
/// runtime or the editor process identity cannot be built,
/// [`EditorError::NoActiveEditorSession`] /
/// [`EditorError::AmbiguousEditorSession`] if `requested_session` is omitted
/// and the project has no single active session,
/// [`EditorError::SessionNotActive`] if the resolved session is not active,
/// [`EditorError::ServiceDiscovery`] if the daemon's project record does not
/// match `project_root` or a stale endpoint record cannot be removed,
/// [`EditorError::MissingSessionService`] if the manifest registers no
/// project-host service, or [`EditorError::RpcTransport`] if any daemon RPC
/// fails.
pub fn attach_to_session_via_daemon(
    project_root: impl AsRef<Path>,
    requested_session: Option<&str>,
    daemon_endpoint: Endpoint,
) -> EditorResult<EditorAttachSession> {
    let project_root = project_root.as_ref().to_path_buf();
    az_project_scaffold::ensure_project_generated_targets(&project_root)
        .map_err(|error| EditorError::GeneratedProjectTargets(error.to_string()))?;
    let requested_session = requested_session.map(str::to_string);
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();

    let attach_daemon_endpoint = daemon_endpoint.clone();
    let editor_process = ProcessIdentity::current()?;

    local.block_on(&runtime, async move {
        let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
        let lease = daemon.touch_editor_process_lease(editor_process).await?;
        info!(
            lease_id = %lease.lease_id,
            active_lease_count = lease.active_lease_count,
            "touched azd editor process lease for attached editor"
        );
        let project = daemon.register_project_root(&project_root).await?;
        ensure_project_record_matches_requested_root(&project, &project_root)?;
        let (session_supervisor, supervisor, manifest) =
            resolve_daemon_attach_session(&daemon, &project, requested_session.as_deref()).await?;
        ensure_proto_session_active(&manifest)?;
        ensure_proto_session_matches_project(&manifest, &project)?;
        let (workspace, source_status) =
            resolve_attached_workspace(&project.project_id, Path::new(&manifest.workspace_root))?;
        let services = resolve_attach_services_from_manifest(&manifest, &supervisor).await?;
        let session_id = manifest.id;
        let probe = verify_attach_services_async(&services, &manifest, &workspace).await?;

        Ok(EditorAttachSession {
            project_id: project.project_id,
            project_root,
            daemon_endpoint: attach_daemon_endpoint,
            session_id,
            session_slug: manifest.slug,
            workspace,
            source_status,
            run_dir: PathBuf::from(manifest.run_dir),
            session_supervisor,
            services,
            workspace_snapshot: probe.workspace_snapshot,
            type_registry: probe.type_registry,
            gamedata_catalog: probe.gamedata_catalog,
            project_inventory: probe.project_inventory,
            verification: probe.verification,
        })
    })
}

fn resolve_attached_workspace(
    project_id: &str,
    workspace_root: &Path,
) -> EditorResult<(AttachedWorkspace, SourceStatus)> {
    let workspace_root = std::fs::canonicalize(workspace_root).map_err(|source| {
        EditorError::ServiceDiscovery(format!(
            "session workspace `{}` cannot be canonicalized: {source}",
            workspace_root.display()
        ))
    })?;
    let source_status = LoreCli.status(&workspace_root, false).map_err(|source| {
        EditorError::ServiceDiscovery(format!(
            "could not resolve Lore status for workspace `{}`: {source}",
            workspace_root.display()
        ))
    })?;
    let branch = source_status.branch.clone().ok_or_else(|| {
        EditorError::ServiceDiscovery(format!(
            "workspace `{}` is not on a named Lore branch",
            workspace_root.display()
        ))
    })?;
    Ok((
        AttachedWorkspace {
            project_id: project_id.to_string(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
            branch,
        },
        source_status,
    ))
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn resolve_daemon_attach_session(
    daemon: &AzDaemonClient,
    project: &ProjectRecord,
    requested_session: Option<&str>,
) -> EditorResult<(
    ServiceDescriptor,
    SessionSupervisorClient,
    session_proto::SessionManifest,
)> {
    if let Some(session_slug) = requested_session {
        let session_supervisor = daemon
            .resolve_session_supervisor_descriptor(&project.project_id, session_slug)
            .await?;
        let supervisor =
            SessionSupervisorClient::connect_for_discovery(&session_supervisor).await?;
        let manifest = supervisor.session_manifest(session_slug).await?;
        ensure_proto_session_matches_project(&manifest, project)?;
        let supervisor =
            SessionSupervisorClient::connect_for_session(&session_supervisor, manifest.id).await?;
        return Ok((session_supervisor, supervisor, manifest));
    }

    let supervisors = daemon
        .list_session_supervisor_descriptors(&project.project_id)
        .await?;
    if supervisors.is_empty() {
        return Err(EditorError::NoActiveEditorSession);
    }

    let mut last_error = None;
    let mut reached_supervisor = false;
    for listed in &supervisors {
        let supervisor =
            match SessionSupervisorClient::connect_for_discovery(&listed.descriptor).await {
                Ok(supervisor) => supervisor,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
        let sessions = match supervisor.list_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        reached_supervisor = true;
        let active = sessions
            .into_iter()
            .filter(|manifest| manifest.state == session_proto::SessionState::Active)
            .map(|manifest| {
                ensure_proto_session_matches_project(&manifest, project)?;
                Ok(manifest)
            })
            .collect::<EditorResult<Vec<_>>>()?;
        match active.len() {
            0 => {}
            1 => {
                let manifest = active
                    .into_iter()
                    .next()
                    .expect("length checked to contain exactly one active session");
                let session_supervisor = supervisors
                    .iter()
                    .find(|candidate| candidate.session_slug == manifest.slug)
                    .map_or_else(
                        || listed.descriptor.clone(),
                        |candidate| candidate.descriptor.clone(),
                    );
                let supervisor =
                    SessionSupervisorClient::connect_for_session(&session_supervisor, manifest.id)
                        .await?;
                return Ok((session_supervisor, supervisor, manifest));
            }
            _ => return Err(EditorError::AmbiguousEditorSession),
        }
    }

    if reached_supervisor {
        Err(EditorError::NoActiveEditorSession)
    } else {
        Err(last_error.unwrap_or(EditorError::NoActiveEditorSession))
    }
}

fn ensure_proto_session_active(manifest: &session_proto::SessionManifest) -> EditorResult<()> {
    if manifest.state == session_proto::SessionState::Active {
        Ok(())
    } else {
        Err(EditorError::SessionNotActive {
            session: manifest.slug.clone(),
            state: format_proto_state(manifest.state).to_string(),
        })
    }
}

fn ensure_project_record_matches_requested_root(
    project: &ProjectRecord,
    requested_root: &Path,
) -> EditorResult<()> {
    if same_protocol_path(Path::new(&project.root), requested_root) {
        Ok(())
    } else {
        Err(EditorError::ServiceDiscovery(format!(
            "azd registered project `{}` at root `{}`, expected requested editor project root `{}`",
            project.project_id,
            project.root,
            requested_root.display()
        )))
    }
}

fn ensure_proto_session_matches_project(
    manifest: &session_proto::SessionManifest,
    project: &ProjectRecord,
) -> EditorResult<()> {
    if manifest.project_id != project.project_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "session `{}` belongs to project `{}`, expected daemon-registered project `{}`",
            manifest.slug, manifest.project_id, project.project_id
        )));
    }
    if !same_protocol_path(Path::new(&manifest.project_root), Path::new(&project.root)) {
        return Err(EditorError::ServiceDiscovery(format!(
            "session `{}` project root `{}` does not match daemon-registered project root `{}`",
            manifest.slug, manifest.project_root, project.root
        )));
    }
    Ok(())
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn resolve_attach_services_from_manifest(
    manifest: &session_proto::SessionManifest,
    supervisor: &SessionSupervisorClient,
) -> EditorResult<EditorAttachServices> {
    let project_host = resolve_required_proto_service(
        manifest,
        || supervisor.resolve_project_host_descriptor(&manifest.slug),
        &ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
        ServiceRole::ProjectHost,
    )
    .await?;
    let asset_processor = resolve_required_proto_service(
        manifest,
        || supervisor.resolve_asset_processor_descriptor(&manifest.slug),
        &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
        ServiceRole::AssetProcessor,
    )
    .await?;
    let runtime_host = resolve_optional_proto_service(
        manifest,
        || supervisor.resolve_runtime_host_descriptor(&manifest.slug),
        &ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
        ServiceRole::RuntimeHost,
    )
    .await?;

    Ok(EditorAttachServices {
        project_host,
        asset_processor,
        runtime_host,
    })
}

#[cfg(test)]
fn ensure_attach_status_matches_selected_session(
    status: &session_proto::SessionWorkspaceStatus,
    selected: &session_proto::SessionManifest,
    project: &ProjectRecord,
) -> EditorResult<()> {
    ensure_proto_session_matches_project(&status.manifest, project)?;
    ensure_status_manifest_matches_selected_session(&status.manifest, selected)?;
    if status.manifest.state == session_proto::SessionState::FailedPreserved
        && status
            .failure_reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(EditorError::ServiceDiscovery(format!(
            "failed-preserved session `{}` status must include a failure reason",
            status.manifest.slug
        )));
    }
    Ok(())
}

#[cfg(test)]
fn ensure_status_manifest_matches_selected_session(
    status_manifest: &session_proto::SessionManifest,
    selected: &session_proto::SessionManifest,
) -> EditorResult<()> {
    if status_manifest.id != selected.id {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status session id `{}` does not match selected session id `{}`",
                status_manifest.id, selected.id
            ),
        ));
    }
    if status_manifest.slug != selected.slug {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status session slug `{}` does not match selected session slug `{}`",
                status_manifest.slug, selected.slug
            ),
        ));
    }
    if status_manifest.project_id != selected.project_id {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status project `{}` does not match selected project `{}`",
                status_manifest.project_id, selected.project_id
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&status_manifest.project_root),
        Path::new(&selected.project_root),
    ) {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status project root `{}` does not match selected project root `{}`",
                status_manifest.project_root, selected.project_root
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&status_manifest.workspace_root),
        Path::new(&selected.workspace_root),
    ) {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status workspace `{}` does not match selected workspace `{}`",
                status_manifest.workspace_root, selected.workspace_root
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&status_manifest.run_dir),
        Path::new(&selected.run_dir),
    ) {
        return Err(status_session_mismatch(
            status_manifest,
            &format!(
                "status run directory `{}` does not match selected run directory `{}`",
                status_manifest.run_dir, selected.run_dir
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn status_session_mismatch(manifest: &session_proto::SessionManifest, reason: &str) -> EditorError {
    EditorError::ServiceDiscovery(format!(
        "session-supervisor status for session `{}` does not match selected editor session: {reason}",
        manifest.slug
    ))
}

async fn resolve_required_proto_service<F, Fut>(
    manifest: &session_proto::SessionManifest,
    resolve: F,
    id: &ServiceId,
    role: ServiceRole,
) -> EditorResult<ServiceDescriptor>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = EditorResult<ServiceDescriptor>>,
{
    let expected = registered_proto_service_descriptor(manifest, id, role)?.clone();

    let descriptor = resolve().await?;
    if !descriptor.has_same_connection_contract(&expected) {
        return Err(EditorError::ServiceDiscovery(format!(
            "active editor session `{}` service `{}` changed its connection contract while resolving editor attach",
            manifest.slug,
            service_label(id),
        )));
    }

    Ok(descriptor)
}

async fn resolve_optional_proto_service<F, Fut>(
    manifest: &session_proto::SessionManifest,
    resolve: F,
    id: &ServiceId,
    role: ServiceRole,
) -> EditorResult<Option<ServiceDescriptor>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = EditorResult<ServiceDescriptor>>,
{
    let Some(expected) = find_proto_service_descriptor(manifest, id, role).cloned() else {
        return Ok(None);
    };

    if let Err(error) = ensure_proto_service_process_running(manifest, &expected) {
        warn!(
            session = %manifest.slug,
            service = %service_label(id),
            role = ?role,
            error = %error,
            "skipping optional editor attach service because it is not running"
        );
        return Ok(None);
    }

    let descriptor = resolve().await?;
    if !descriptor.has_same_connection_contract(&expected) {
        warn!(
            session = %manifest.slug,
            service = %service_label(id),
            role = ?role,
            "skipping optional editor attach service because its connection contract changed while resolving"
        );
        return Ok(None);
    }

    Ok(Some(descriptor))
}

fn registered_proto_service_descriptor<'a>(
    manifest: &'a session_proto::SessionManifest,
    id: &ServiceId,
    role: ServiceRole,
) -> EditorResult<&'a ServiceDescriptor> {
    find_proto_service_descriptor(manifest, id, role).ok_or_else(|| {
        EditorError::MissingSessionService {
            session: manifest.slug.clone(),
            service: service_label(id),
        }
    })
}

fn find_proto_service_descriptor<'a>(
    manifest: &'a session_proto::SessionManifest,
    id: &ServiceId,
    role: ServiceRole,
) -> Option<&'a ServiceDescriptor> {
    manifest
        .services
        .iter()
        .find(|descriptor| descriptor.id == *id && descriptor.role == role)
}

fn ensure_proto_service_process_running(
    manifest: &session_proto::SessionManifest,
    descriptor: &ServiceDescriptor,
) -> EditorResult<()> {
    let Some(process) = find_proto_service_process(manifest, descriptor) else {
        return Err(EditorError::SessionServiceNotRunning {
            session: manifest.slug.clone(),
            service: service_label(&descriptor.id),
            run: descriptor.run,
            state: "missing process record".to_string(),
        });
    };

    if process.state != session_proto::ServiceProcessState::Running {
        return Err(EditorError::SessionServiceNotRunning {
            session: manifest.slug.clone(),
            service: service_label(&descriptor.id),
            run: descriptor.run,
            state: format_proto_process_state(process.state).to_string(),
        });
    }

    if process.endpoint != descriptor.endpoint {
        return Err(EditorError::ServiceDiscovery(format!(
            "active editor session `{}` service `{}` descriptor endpoint {:?} `{}` does not match running process endpoint {:?} `{}`",
            manifest.slug,
            service_label(&descriptor.id),
            descriptor.endpoint.kind,
            descriptor.endpoint.address,
            process.endpoint.kind,
            process.endpoint.address
        )));
    }

    Ok(())
}

fn find_proto_service_process<'a>(
    manifest: &'a session_proto::SessionManifest,
    descriptor: &ServiceDescriptor,
) -> Option<&'a session_proto::ServiceProcessRecord> {
    manifest.processes.iter().find(|process| {
        process.service_name == descriptor.id.name && process.role == descriptor.role
    })
}

fn service_label(id: &ServiceId) -> String {
    format!("{}/{}", id.namespace, id.name)
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn verify_attach_services_async(
    services: &EditorAttachServices,
    manifest: &session_proto::SessionManifest,
    workspace: &AttachedWorkspace,
) -> EditorResult<EditorAttachProbe> {
    let session_id = manifest.id;
    // These three probes target three DIFFERENT services over distinct
    // connections and share no data dependency (they only join into the probe
    // below), so we overlap them. The editor attach runs on a current-thread
    // tokio runtime under a `LocalSet`, and the capnp RPC futures are `!Send`,
    // so `try_join!` (await-driven concurrency on a single thread) is the
    // correct primitive here rather than a thread-spawning fan-out.
    let (
        (type_registry, gamedata_catalog, project_inventory),
        workspace_snapshot,
        runtime_host_projection_count,
    ) = tokio::try_join!(
        verify_project_host_rpc(&services.project_host, session_id),
        verify_asset_processor_rpc(&services.asset_processor, manifest, workspace),
        async {
            match &services.runtime_host {
                Some(runtime_host) => verify_runtime_host_rpc(runtime_host, session_id)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    )?;
    let project_host_type_count = type_registry.types.len();
    let project_inventory_expected_gem_count = project_inventory
        .gems
        .iter()
        .filter(|gem| gem.expected)
        .count();
    let project_inventory_active_gem_count = project_inventory
        .gems
        .iter()
        .filter(|gem| gem.active)
        .count();
    let project_inventory_diagnostics_count = project_inventory.diagnostics.len();
    let project_inventory_degraded = project_inventory_is_degraded(&project_inventory);

    Ok(EditorAttachProbe {
        type_registry,
        gamedata_catalog,
        project_inventory,
        workspace_snapshot: workspace_snapshot.clone(),
        verification: EditorAttachVerification {
            project_host_type_count,
            asset_processor_workspace_id: workspace_snapshot.workspace_id,
            asset_processor_root_count: workspace_snapshot.roots.len(),
            runtime_host_verified: runtime_host_projection_count.is_some(),
            runtime_host_projection_count,
            project_inventory_expected_gem_count,
            project_inventory_active_gem_count,
            project_inventory_diagnostics_count,
            project_inventory_degraded,
        },
    })
}

#[instrument(
    skip_all,
    fields(
        session_id = %session_id,
        endpoint = %descriptor.endpoint.address,
    )
)]
#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn verify_project_host_rpc(
    descriptor: &ServiceDescriptor,
    session_id: Uuid,
) -> EditorResult<(
    TypeRegistrySnapshot,
    GameDataCatalogSnapshot,
    ProjectInventoryReport,
)> {
    ensure_descriptor(
        descriptor,
        PROJECT_HOST_NAMESPACE,
        PROJECT_HOST_SERVICE_NAME,
        ServiceRole::ProjectHost,
    )?;
    let client = ProjectHostClient::connect_for_session(descriptor, session_id).await?;
    let type_registry = attach_required_rpc(
        "project-host.typeRegistrySnapshot",
        ATTACH_REQUIRED_RPC_TIMEOUT,
        client.type_registry_snapshot(),
    )
    .await?;
    info!(
        schema_catalog_hash = ?type_registry.schema_catalog_hash,
        type_count = type_registry.types.len(),
        "verified project-host reflected type registry"
    );
    let gamedata_catalog = GameDataCatalogSnapshot::empty(0);
    let project_inventory = fallback_project_inventory_report(
        "project-host inventory is loaded after attach; deferred catalogs must not block editor attach"
            .to_owned(),
    );
    info!(
        "deferred project-host GameData catalog and project inventory until session controllers are installed"
    );
    Ok((type_registry, gamedata_catalog, project_inventory))
}

fn fallback_project_inventory_report(diagnostic: String) -> ProjectInventoryReport {
    ProjectInventoryReport {
        service_role: ServiceRole::ProjectHost.stable_name().to_string(),
        lock_status: az_proto_project::ProjectInventoryLockStatus {
            state: az_proto_project::ProjectInventoryLockState::Unavailable,
            path: String::new(),
            diagnostic: diagnostic.clone(),
        },
        gems: Vec::new(),
        registry: az_proto_project::ProjectInventoryRegistryCounts {
            build_rules: 0,
            node_types: 0,
            graph_types: 0,
        },
        degraded: true,
        diagnostics: vec![diagnostic],
    }
}

const fn project_inventory_is_degraded(report: &ProjectInventoryReport) -> bool {
    report.degraded
        || !report.diagnostics.is_empty()
        || !matches!(
            report.lock_status.state,
            az_proto_project::ProjectInventoryLockState::Fresh
        )
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn verify_asset_processor_rpc(
    descriptor: &ServiceDescriptor,
    manifest: &session_proto::SessionManifest,
    workspace: &AttachedWorkspace,
) -> EditorResult<az_proto_asset::WorkspaceSnapshot> {
    ensure_descriptor(
        descriptor,
        ASSET_PROCESSOR_NAMESPACE,
        ASSET_PROCESSOR_SERVICE_NAME,
        ServiceRole::AssetProcessor,
    )?;
    let session_id = manifest.id;
    let client = AssetProcessorClient::connect_for_session(descriptor, session_id).await?;
    let snapshot = attach_required_rpc(
        "asset-processor.workspaceSnapshot",
        ATTACH_REQUIRED_RPC_TIMEOUT,
        client.workspace_snapshot(AssetRootScope::All),
    )
    .await?
    .ok_or_else(|| {
        EditorError::ServiceDiscovery(format!(
            "asset-processor has no workspace snapshot for attached workspace `{}` on branch `{}`",
            workspace.workspace_root, workspace.branch
        ))
    })?;
    validate_workspace_snapshot_matches_workspace(&snapshot, workspace)?;
    validate_workspace_snapshot_roots(&snapshot, manifest, workspace)?;
    Ok(snapshot)
}

fn validate_workspace_snapshot_matches_workspace(
    snapshot: &WorkspaceSnapshot,
    workspace: &AttachedWorkspace,
) -> EditorResult<()> {
    if snapshot.project_id != workspace.project_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor workspace snapshot {} project `{}` does not match attached project `{}`",
            snapshot.workspace_id, snapshot.project_id, workspace.project_id
        )));
    }
    if !same_protocol_path(&snapshot.workspace_root, &workspace.workspace_root) {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor workspace snapshot {} root `{}` does not match attached workspace `{}`",
            snapshot.workspace_id, snapshot.workspace_root, workspace.workspace_root
        )));
    }
    if snapshot.branch != workspace.branch {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor workspace snapshot {} branch `{}` does not match attached branch `{}`",
            snapshot.workspace_id, snapshot.branch, workspace.branch
        )));
    }
    Ok(())
}

fn validate_workspace_snapshot_roots(
    snapshot: &WorkspaceSnapshot,
    manifest: &session_proto::SessionManifest,
    workspace: &AttachedWorkspace,
) -> EditorResult<()> {
    if snapshot.roots.is_empty() {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor workspace snapshot {} for session `{}` has no roots",
            snapshot.workspace_id, manifest.id
        )));
    }
    let mut seen_workspace_root_ids = HashSet::new();
    let mut seen_portable_keys = HashSet::new();
    for root in &snapshot.roots {
        validate_workspace_snapshot_root(snapshot, root)?;
        if !seen_workspace_root_ids.insert(root.workspace_root_id) {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} returned duplicate workspace-root id {}",
                snapshot.workspace_id, root.workspace_root_id
            )));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} returned duplicate root key `{}`",
                snapshot.workspace_id, root.portable_key
            )));
        }
    }

    let project_assets_key = project_assets_root_portable_key(&manifest.project_id);
    let project_assets = snapshot
        .roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
        .ok_or_else(|| {
            EditorError::ServiceDiscovery(format!(
                "asset-processor workspace snapshot {} is missing project assets root `{project_assets_key}`",
                snapshot.workspace_id
            ))
        })?;
    if project_assets.owner_id != manifest.project_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor project assets root `{project_assets_key}` owner `{}` does not match attached project `{}`",
            project_assets.owner_id, manifest.project_id
        )));
    }
    let project_assets_path = PathBuf::from(&project_assets.source_root);
    let workspace_root = Path::new(&workspace.workspace_root);
    if project_assets_path == workspace_root || !project_assets_path.starts_with(workspace_root) {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor project assets root `{project_assets_key}` path `{}` must be inside attached workspace `{}`",
            project_assets.source_root, workspace.workspace_root
        )));
    }
    if !project_assets.is_root {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor project assets root `{project_assets_key}` must be marked as a root source"
        )));
    }
    if !project_assets.output_prefix.is_empty() {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor project assets root `{project_assets_key}` must use an empty output prefix"
        )));
    }

    Ok(())
}

fn validate_workspace_snapshot_root(
    snapshot: &WorkspaceSnapshot,
    root: &WorkspaceRoot,
) -> EditorResult<()> {
    if root.workspace_id != snapshot.workspace_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor root `{}` belongs to workspace {}, expected {}",
            root.portable_key, root.workspace_id, snapshot.workspace_id
        )));
    }
    if root.workspace_root_id <= 0 {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor root `{}` must carry a positive workspace-root id",
            root.portable_key
        )));
    }
    if root.root_id <= 0 {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor root `{}` must carry a positive root id",
            root.portable_key
        )));
    }
    if root.owner_id.trim().is_empty() {
        return Err(EditorError::ServiceDiscovery(
            "asset-processor root owner id cannot be empty".to_string(),
        ));
    }
    if root.source_root.trim().is_empty() {
        return Err(EditorError::ServiceDiscovery(format!(
            "asset-processor root `{}` physical root cannot be empty",
            root.portable_key
        )));
    }
    if root.portable_key.trim().is_empty() {
        return Err(EditorError::ServiceDiscovery(
            "asset-processor root portable key cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn project_assets_root_portable_key(project_id: &str) -> String {
    format!("project:{project_id}:assets")
}

fn same_protocol_path(left: impl AsRef<Path>, right: impl AsRef<Path>) -> bool {
    let left = comparable_path(left.as_ref());
    let right = comparable_path(right.as_ref());
    #[cfg(windows)]
    {
        comparable_path_text(&left) == comparable_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn comparable_path(path: &Path) -> PathBuf {
    strip_trailing_path_separators(normalize(path))
}

fn strip_trailing_path_separators(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let stripped = text.trim_end_matches(['/', '\\']);
    if stripped.is_empty() {
        path
    } else {
        PathBuf::from(stripped)
    }
}

#[cfg(windows)]
fn comparable_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn verify_runtime_host_rpc(
    descriptor: &ServiceDescriptor,
    session_id: Uuid,
) -> EditorResult<usize> {
    ensure_descriptor(
        descriptor,
        RUNTIME_HOST_NAMESPACE,
        RUNTIME_HOST_SERVICE_NAME,
        ServiceRole::RuntimeHost,
    )?;
    let client = RuntimeHostClient::connect_for_session(descriptor, session_id).await?;
    let catalog = attach_required_rpc(
        "runtime-host.projectionCatalog",
        ATTACH_OPTIONAL_RPC_TIMEOUT,
        client.projection_catalog(),
    )
    .await?;
    Ok(catalog.projections.len())
}

async fn attach_required_rpc<T>(
    operation: &'static str,
    timeout: Duration,
    future: impl Future<Output = EditorResult<T>>,
) -> EditorResult<T> {
    tokio::time::timeout(timeout, future)
        .await
        .unwrap_or_else(|_| {
            Err(EditorError::ServiceDiscovery(format!(
                "{operation} did not answer within {} ms during editor attach",
                timeout.as_millis()
            )))
        })
}

fn ensure_descriptor(
    descriptor: &ServiceDescriptor,
    namespace: &str,
    name: &str,
    role: ServiceRole,
) -> EditorResult<()> {
    let expected = ServiceId::new(namespace, name);
    if descriptor.id != expected || descriptor.role != role {
        return Err(EditorError::ServiceDiscovery(format!(
            "expected `{}`/`{}` with role {:?}, got `{}`/`{}` with role {:?}",
            expected.namespace,
            expected.name,
            role,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        )));
    }
    Ok(())
}

const fn format_proto_state(state: session_proto::SessionState) -> &'static str {
    match state {
        session_proto::SessionState::Preparing => "preparing",
        session_proto::SessionState::Active => "active",
        session_proto::SessionState::FailedPreserved => "failed-preserved",
        session_proto::SessionState::Removed => "removed",
    }
}

const fn format_proto_process_state(state: session_proto::ServiceProcessState) -> &'static str {
    match state {
        session_proto::ServiceProcessState::Planned => "planned",
        session_proto::ServiceProcessState::Starting => "starting",
        session_proto::ServiceProcessState::Running => "running",
        session_proto::ServiceProcessState::Exited => "exited",
        session_proto::ServiceProcessState::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use az_daemon::{AzDaemon, start_az_daemon_rpc_server_with_daemon};
    use az_filesystem::AzothDataHome;
    use az_project::{ProjectManifest, refresh_project_lock, write_project_manifest};
    use az_proto_core::{CapabilityGrantSet, Endpoint, EndpointKind};
    use az_service_catalog::{
        asset_processor_service_descriptor, project_host_service_descriptor,
        runtime_host_service_descriptor, session_supervisor_service_descriptor,
    };
    use az_service_supervision::{ServiceProcessRecord, SupervisedServiceRole};

    /// One process-lifetime composition, the shape a runtime host requires.
    fn runtime_host_registries() -> &'static az_gem_contract::Registries {
        static REGISTRIES: std::sync::OnceLock<az_gem_contract::Registries> =
            std::sync::OnceLock::new();
        REGISTRIES.get_or_init(az_gem_contract::Registries::new)
    }
    use az_session::{
        CreateSessionRequest, SESSION_MANIFEST_FILE, SessionId, SessionManager, SessionManifest,
        start_session_supervisor_rpc_server_with_manager,
    };
    use uuid::Uuid;

    use super::*;
    // The record fixtures belong to the probe-then-validate order these tests
    // pin, so they come from its owner rather than being transcribed again.
    use crate::daemon_bootstrap::test_records::{
        foreign_protocol_version, record_test_project, unreachable_local_endpoint,
        write_foreign_protocol_record,
    };

    fn temp_project(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("az-editor-attach-test-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        write_test_project_manifest(
            &path,
            &ProjectManifest::new(
                format!("local.az_editor_attach_{name}"),
                "Editor Attach Test",
                "0.1.0",
            ),
        );
        path
    }

    fn write_test_project_manifest(root: &Path, manifest: &ProjectManifest) {
        write_project_manifest(root, manifest).unwrap();
        refresh_project_lock(root).unwrap();
    }

    fn test_tcp_endpoint(label: impl AsRef<str>) -> Endpoint {
        let hash = label.as_ref().bytes().fold(0_u16, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(u16::from(byte))
        });
        Endpoint::new(
            EndpointKind::Tcp,
            format!("127.0.0.1:{}", 30_000 + hash % 20_000),
        )
    }

    fn proto_manifest_with_state(
        state: session_proto::SessionState,
    ) -> session_proto::SessionManifest {
        session_proto::SessionManifest::new(
            Uuid::new_v4(),
            "local.editor_attach",
            "editor",
            "project",
            "project",
            "project/.azoth/sessions/editor",
            state,
        )
    }

    fn proto_process_for_descriptor(
        descriptor: &ServiceDescriptor,
        state: session_proto::ServiceProcessState,
    ) -> session_proto::ServiceProcessRecord {
        let is_running = state == session_proto::ServiceProcessState::Running;
        let is_terminal = matches!(
            state,
            session_proto::ServiceProcessState::Exited | session_proto::ServiceProcessState::Failed
        );
        // Supervision captures the program artifact when a process leaves `Planned`
        // (`mark_service_starting` -> `capture_program_artifact`), so only an unlaunched
        // plan legitimately carries `None` here.
        let is_launched = state != session_proto::ServiceProcessState::Planned;

        session_proto::ServiceProcessRecord {
            owner_id: "local.editor_attach".to_string(),
            service_name: descriptor.id.name.clone(),
            role: descriptor.role,
            run: descriptor.run,
            previous_run: None,
            endpoint: descriptor.endpoint.clone(),
            program: "az-test-service".to_string(),
            program_artifact: is_launched.then_some(session_proto::ServiceProgramArtifact {
                byte_length: 4096,
                modified_unix_ns: 1_700_000_000_000_000_000,
                file_system_id: Some(7),
                file_id: Some(11),
            }),
            cwd: "project".to_string(),
            owner_root: "project".to_string(),
            args: Vec::new(),
            stdout_log: "project/.azoth/sessions/editor/logs/service.stdout.log".to_string(),
            stderr_log: "project/.azoth/sessions/editor/logs/service.stderr.log".to_string(),
            structured_log: "project/.azoth/sessions/editor/logs/service.capnp.log".to_string(),
            state,
            pid: is_running.then_some(4242),
            process_start_time: is_running.then_some(1_700_000_000),
            exit_code: (state == session_proto::ServiceProcessState::Failed).then_some(1),
            failure: (state == session_proto::ServiceProcessState::Failed)
                .then_some("failed".to_string()),
            planned_unix_ms: 10,
            updated_unix_ms: if is_terminal { 30 } else { 20 },
            started_unix_ms: is_running.then_some(20),
            exited_unix_ms: is_terminal.then_some(30),
        }
    }

    fn session_workspace_status_for_manifest(
        manifest: session_proto::SessionManifest,
    ) -> session_proto::SessionWorkspaceStatus {
        session_proto::SessionWorkspaceStatus {
            manifest,
            failure_reason: None,
        }
    }

    fn project_record_for_manifest(manifest: &session_proto::SessionManifest) -> ProjectRecord {
        ProjectRecord {
            project_id: manifest.project_id.clone(),
            name: "Editor Attach Test".to_string(),
            root: manifest.project_root.clone(),
            manifest_path: Path::new(&manifest.project_root)
                .join("azoth.toml")
                .to_string_lossy()
                .into_owned(),
            engine_version: "0.1.0".to_string(),
        }
    }

    fn workspace_snapshot_for_manifest(
        manifest: &session_proto::SessionManifest,
    ) -> az_proto_asset::WorkspaceSnapshot {
        az_proto_asset::WorkspaceSnapshot {
            workspace_id: 42,
            workspace_root: manifest.workspace_root.clone(),
            branch: "main".to_string(),
            created_unix_ms: 10,
            updated_unix_ms: 20,
            project_id: manifest.project_id.clone(),
            roots: Vec::new(),
        }
    }

    fn attached_workspace_for_manifest(
        manifest: &session_proto::SessionManifest,
    ) -> AttachedWorkspace {
        AttachedWorkspace {
            project_id: manifest.project_id.clone(),
            workspace_root: manifest.workspace_root.clone(),
            branch: "main".to_string(),
        }
    }

    fn workspace_snapshot_with_roots_for_manifest(
        manifest: &session_proto::SessionManifest,
    ) -> az_proto_asset::WorkspaceSnapshot {
        let mut snapshot = workspace_snapshot_for_manifest(manifest);
        snapshot.roots = vec![
            az_proto_asset::WorkspaceRoot {
                workspace_root_id: 420,
                workspace_id: snapshot.workspace_id,
                root_id: 421,
                declared_root_id: "project.assets".to_string(),
                owner_id: manifest.project_id.clone(),
                source_root: format!("{}/assets", manifest.workspace_root),
                display_name: "Project Assets".to_string(),
                portable_key: format!("project:{}:assets", manifest.project_id),
                mount: "@assets@".to_string(),
                recursive: true,
                watch: true,
                writable: true,
                output_prefix: String::new(),
                is_root: true,
            },
            az_proto_asset::WorkspaceRoot {
                workspace_root_id: 422,
                workspace_id: snapshot.workspace_id,
                root_id: 423,
                declared_root_id: "gem.azoth.physics.assets".to_string(),
                owner_id: "azoth.physics".to_string(),
                source_root: format!("{}/gems/physics/assets", manifest.workspace_root),
                display_name: "Physics Assets".to_string(),
                portable_key: "gem:azoth.physics:assets".to_string(),
                mount: "@gems/physics/assets@".to_string(),
                recursive: true,
                watch: true,
                writable: false,
                output_prefix: "gems/azoth.physics".to_string(),
                is_root: false,
            },
        ];
        snapshot
    }

    fn local_running_process_for_descriptor(
        descriptor: &ServiceDescriptor,
        cwd: &Path,
        owner_id: &str,
    ) -> ServiceProcessRecord {
        let role = SupervisedServiceRole::from_proto(descriptor.role)
            .expect("test descriptors should use concrete service roles");
        let mut process = ServiceProcessRecord::planned(
            descriptor.id.name.clone(),
            role,
            descriptor.run,
            &descriptor.endpoint,
            "az-test-service",
            cwd.to_path_buf(),
            Vec::new(),
            cwd.join(format!("{}.stdout.log", descriptor.id.name)),
            cwd.join(format!("{}.stderr.log", descriptor.id.name)),
            cwd.join(format!("{}.capnp.log", descriptor.id.name)),
            None,
            10,
        );
        process.owner_id = owner_id.to_string();
        process.owner_root = cwd.to_path_buf();
        process
            .mark_running(ProcessIdentity::current().unwrap(), 20)
            .unwrap();
        process
    }

    #[test]
    fn rejects_non_active_sessions() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::FailedPreserved);

        let err = ensure_proto_session_active(&manifest).unwrap_err();

        assert!(matches!(err, EditorError::SessionNotActive { .. }));
    }

    #[test]
    fn attach_session_must_match_daemon_registered_project_identity() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let other_root = temp.path().join("other");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let project = ProjectRecord {
            project_id: "local.editor_attach".to_string(),
            name: "Editor Attach".to_string(),
            root: project_root.to_string_lossy().into_owned(),
            manifest_path: project_root
                .join("azoth.toml")
                .to_string_lossy()
                .into_owned(),
            engine_version: "0.1.0".to_string(),
        };
        let mut manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        manifest.project_id = project.project_id.clone();
        manifest.project_root = project.root.clone();

        ensure_project_record_matches_requested_root(&project, &project_root).unwrap();
        ensure_proto_session_matches_project(&manifest, &project).unwrap();

        let mut wrong_project = manifest.clone();
        wrong_project.project_id = "local.other".to_string();
        assert!(matches!(
            ensure_proto_session_matches_project(&wrong_project, &project),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("local.other")
                    && message.contains("daemon-registered project")
        ));

        let mut wrong_session_root = manifest;
        wrong_session_root.project_root = other_root.to_string_lossy().into_owned();
        assert!(matches!(
            ensure_proto_session_matches_project(&wrong_session_root, &project),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("project root")
                    && message.contains("daemon-registered project root")
        ));

        let mut wrong_daemon_project = project;
        wrong_daemon_project.root = other_root.to_string_lossy().into_owned();
        assert!(matches!(
            ensure_project_record_matches_requested_root(&wrong_daemon_project, &project_root),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("requested editor project root")
        ));
    }

    #[test]
    fn attach_status_must_match_selected_session_identity() {
        let selected = proto_manifest_with_state(session_proto::SessionState::Active);
        let project = project_record_for_manifest(&selected);
        let mut status_manifest = selected.clone();
        let mut status = session_workspace_status_for_manifest(status_manifest.clone());

        ensure_attach_status_matches_selected_session(&status, &selected, &project).unwrap();

        status_manifest.id = Uuid::new_v4();
        status.manifest = status_manifest.clone();
        let mismatch = ensure_attach_status_matches_selected_session(&status, &selected, &project)
            .unwrap_err();
        assert!(matches!(
            mismatch,
            EditorError::ServiceDiscovery(message)
                if message.contains("status session id")
                    && message.contains("selected session id")
        ));

        status_manifest = selected.clone();
        status_manifest.workspace_root = "project/other-workspace".to_string();
        status.manifest = status_manifest;
        let mismatch = ensure_attach_status_matches_selected_session(&status, &selected, &project)
            .unwrap_err();
        assert!(matches!(
            mismatch,
            EditorError::ServiceDiscovery(message)
                if message.contains("status workspace")
                    && message.contains("selected workspace")
        ));
    }

    #[test]
    fn attach_status_requires_failure_reason_for_failed_preserved_session() {
        let selected = proto_manifest_with_state(session_proto::SessionState::Active);
        let project = project_record_for_manifest(&selected);
        let mut failed = selected.clone();
        failed.state = session_proto::SessionState::FailedPreserved;
        let status = session_workspace_status_for_manifest(failed);
        let mismatch = ensure_attach_status_matches_selected_session(&status, &selected, &project)
            .unwrap_err();
        assert!(matches!(
            mismatch,
            EditorError::ServiceDiscovery(message)
                if message.contains("failed-preserved")
                    && message.contains("failure reason")
        ));
    }

    #[test]
    fn proto_optional_service_detects_registered_project_host() {
        let mut manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let descriptor = project_host_service_descriptor(
            Uuid::now_v7(),
            Endpoint::in_process("project-host:editor"),
        );
        manifest.services.push(descriptor);
        manifest.processes.push(proto_process_for_descriptor(
            &manifest.services[0],
            session_proto::ServiceProcessState::Running,
        ));

        let service = futures::executor::block_on(resolve_optional_proto_service(
            &manifest,
            || async { Ok(manifest.services[0].clone()) },
            &ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        ))
        .unwrap();

        assert_eq!(service.unwrap().endpoint.address, "project-host:editor");
    }

    #[test]
    fn proto_optional_service_skips_registered_project_host_without_running_process() {
        let mut manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let descriptor = project_host_service_descriptor(
            Uuid::now_v7(),
            Endpoint::in_process("project-host:editor"),
        );
        manifest.services.push(descriptor);

        let service = futures::executor::block_on(resolve_optional_proto_service(
            &manifest,
            || async { Ok(manifest.services[0].clone()) },
            &ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        ))
        .unwrap();

        assert!(service.is_none());
    }

    #[test]
    fn proto_required_service_rejects_missing_asset_processor() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let descriptor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            Endpoint::in_process("asset-processor:editor"),
        );

        let err = futures::executor::block_on(resolve_required_proto_service(
            &manifest,
            || async move { Ok(descriptor.clone()) },
            &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
        ))
        .unwrap_err();

        assert!(matches!(
            err,
            EditorError::MissingSessionService { service, .. }
                if service.contains(ASSET_PROCESSOR_SERVICE_NAME)
        ));
    }

    #[test]
    fn proto_required_project_service_accepts_attachment_without_session_process() {
        let mut manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let descriptor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            Endpoint::in_process("asset-processor:editor"),
        );
        manifest.services.push(descriptor.clone());

        let service = futures::executor::block_on(resolve_required_proto_service(
            &manifest,
            || async { Ok(descriptor) },
            &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
        ))
        .unwrap();

        assert_eq!(service.endpoint.address, "asset-processor:editor");
        assert!(manifest.processes.is_empty());
    }

    #[test]
    fn asset_processor_workspace_snapshot_must_match_manifest_identity() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let valid = workspace_snapshot_for_manifest(&manifest);
        let workspace = attached_workspace_for_manifest(&manifest);
        validate_workspace_snapshot_matches_workspace(&valid, &workspace).unwrap();

        let mut wrong_project = valid.clone();
        wrong_project.project_id = "local.other".to_string();
        assert!(matches!(
            validate_workspace_snapshot_matches_workspace(&wrong_project, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("project") && message.contains("local.other")
        ));

        let mut wrong_root = valid.clone();
        wrong_root.workspace_root = "project/.azoth/workspaces/stale".to_string();
        assert!(matches!(
            validate_workspace_snapshot_matches_workspace(&wrong_root, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("root") && message.contains("stale")
        ));

        let mut wrong_branch = valid;
        wrong_branch.branch = "az/session/stale".to_string();
        assert!(matches!(
            validate_workspace_snapshot_matches_workspace(&wrong_branch, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("branch") && message.contains("stale")
        ));
    }

    #[test]
    fn asset_processor_workspace_snapshot_roots_must_have_identity() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        let workspace = attached_workspace_for_manifest(&manifest);
        validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace).unwrap();

        snapshot.roots[0].workspace_root_id = 0;
        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("positive workspace-root id")
        ));

        snapshot.roots[0].workspace_root_id = 420;
        snapshot.roots[0].root_id = 0;
        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("positive root id")
        ));
    }

    #[test]
    fn asset_processor_workspace_snapshot_requires_project_assets_root() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        let workspace = attached_workspace_for_manifest(&manifest);
        snapshot
            .roots
            .retain(|root| root.portable_key != format!("project:{}:assets", manifest.project_id));

        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("missing project assets root")
        ));

        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        snapshot.roots[0].source_root = "elsewhere/project/.azoth/workspaces/stale".to_string();
        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("must be inside attached workspace")
        ));

        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        snapshot.roots[0].is_root = false;
        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("marked as a root source")
        ));

        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        snapshot.roots[0].output_prefix = "prefabs".to_string();
        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("empty output prefix")
        ));
    }

    #[test]
    fn asset_processor_workspace_snapshot_rejects_duplicate_roots() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let mut duplicate_id = workspace_snapshot_with_roots_for_manifest(&manifest);
        let workspace = attached_workspace_for_manifest(&manifest);
        duplicate_id.roots[1].workspace_root_id = duplicate_id.roots[0].workspace_root_id;

        assert!(matches!(
            validate_workspace_snapshot_roots(&duplicate_id, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("duplicate workspace-root id")
        ));

        let mut duplicate_key = workspace_snapshot_with_roots_for_manifest(&manifest);
        duplicate_key.roots[1].portable_key = duplicate_key.roots[0].portable_key.clone();

        assert!(matches!(
            validate_workspace_snapshot_roots(&duplicate_key, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("duplicate root key")
        ));
    }

    #[test]
    fn asset_processor_workspace_snapshot_project_assets_owner_must_match_project() {
        let manifest = proto_manifest_with_state(session_proto::SessionState::Active);
        let mut snapshot = workspace_snapshot_with_roots_for_manifest(&manifest);
        let workspace = attached_workspace_for_manifest(&manifest);
        snapshot.roots[0].owner_id = "azoth.physics".to_string();

        assert!(matches!(
            validate_workspace_snapshot_roots(&snapshot, &manifest, &workspace),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("owner")
                    && message.contains("azoth.physics")
                    && message.contains(&manifest.project_id)
        ));
    }

    #[test]
    fn attach_services_require_project_host_and_asset_processor_but_not_runtime_host() {
        let root = temp_project("required-services");
        let manager = SessionManager::with_data_home(
            &root,
            AzothDataHome::new(root.join(".azoth-test-home")),
        )
        .unwrap();
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Required"))
            .unwrap();
        let project_host = project_host_service_descriptor(
            Uuid::now_v7(),
            test_tcp_endpoint("project-host:editor-required"),
        );
        let asset_processor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            test_tcp_endpoint("asset-processor:editor-required"),
        );
        manager
            .attach_project_service_descriptors(
                "editor-required",
                &[project_host.clone(), asset_processor.clone()],
            )
            .unwrap();

        let mut proto_manifest = az_session::session_manifest_to_proto(&manifest);
        proto_manifest.services.push(project_host.clone());
        proto_manifest.services.push(asset_processor.clone());
        assert!(proto_manifest.processes.is_empty());

        let session_supervisor_rpc =
            std::rc::Rc::new(az_session::SessionSupervisorRpc::test_with_manager(manager));
        let session_supervisor = SessionSupervisorClient::new(
            az_session::SessionSupervisorRpc::client_from_rc(&session_supervisor_rpc),
        )
        .with_session_scope(manifest.id.0);

        let services = futures::executor::block_on(resolve_attach_services_from_manifest(
            &proto_manifest,
            &session_supervisor,
        ))
        .unwrap();

        assert_eq!(services.project_host, project_host);
        assert_eq!(services.asset_processor, asset_processor);
        assert_eq!(services.runtime_host, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn daemon_attach_uses_manifest_without_full_dirty_status() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/attach.rs")).unwrap();
        let function_start = source
            .find("pub fn attach_to_session_via_daemon")
            .expect("attach function should exist");
        let rest = &source[function_start..];
        let function_end = rest
            .find("\nasync fn resolve_daemon_attach_session")
            .expect("next attach helper should follow attach function");
        let function_body = &rest[..function_end];

        assert!(
            !az_architecture_guard::symbols_contain(function_body, "supervisor.status("),
            "initial editor attach must not request full session status; that path can include large dirty workspace output"
        );
        assert!(az_architecture_guard::symbols_contain(
            function_body,
            "resolve_attach_services_from_manifest"
        ));
    }

    #[test]
    fn project_host_attach_probe_does_not_load_deferred_catalogs() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/attach.rs")).unwrap();
        let function_start = source
            .find("async fn verify_project_host_rpc")
            .expect("project-host attach probe should exist");
        let rest = &source[function_start..];
        let function_end = rest
            .find("\nfn fallback_project_inventory_report")
            .expect("fallback inventory helper should follow project-host probe");
        let function_body = &rest[..function_end];

        assert!(
            !az_architecture_guard::symbols_contain(function_body, "load_gamedata_catalog("),
            "initial editor attach must not request the GameData catalog; it is refreshed after the session is installed"
        );
        assert!(az_architecture_guard::symbols_contain(
            function_body,
            "GameDataCatalogSnapshot::empty"
        ));
    }

    #[test]
    fn resolves_project_host_through_session_supervisor_rpc() {
        let root = temp_project("resolve-supervisor");
        let manager = SessionManager::with_data_home(
            &root,
            AzothDataHome::new(root.join(".azoth-test-home")),
        )
        .unwrap();
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Resolve"))
            .unwrap();
        let project_host = project_host_service_descriptor(
            Uuid::now_v7(),
            test_tcp_endpoint("project-host:editor-resolve"),
        );
        manager
            .attach_project_service_descriptors(
                "editor-resolve",
                std::slice::from_ref(&project_host),
            )
            .unwrap();

        let session_supervisor_rpc =
            std::rc::Rc::new(az_session::SessionSupervisorRpc::test_with_manager(manager));
        let session_supervisor = SessionSupervisorClient::new(
            az_session::SessionSupervisorRpc::client_from_rc(&session_supervisor_rpc),
        )
        .with_session_scope(manifest.id.0);

        let resolved = futures::executor::block_on(
            session_supervisor.resolve_project_host_descriptor("editor-resolve"),
        )
        .unwrap();

        assert_eq!(resolved, project_host);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Builds the on-disk fixture attach reads: a project manifest, a real Lore
    /// repository for the workspace, and an activated session manifest under a
    /// fresh session-supervisor data home.
    fn build_attach_fixture(
        root: &std::path::Path,
        lore_remote: String,
    ) -> (SessionManager, SessionManifest, std::path::PathBuf) {
        let project_manifest =
            ProjectManifest::new("local.editor_attach", "Editor Attach", "0.1.0");
        write_test_project_manifest(root, &project_manifest);
        let session_id = SessionId::new();
        let workspace_path = root.to_path_buf();
        std::fs::create_dir_all(workspace_path.join("assets")).unwrap();
        write_test_project_manifest(&workspace_path, &project_manifest);
        // Attach resolves Lore status for the workspace; the fixture needs a
        // real repository for that call to succeed.
        LoreCli
            .create_repository(&az_source_control::CreateRepositoryRequest {
                remote_url: lore_remote,
                path: workspace_path.clone(),
                description: Some("attach test workspace".to_string()),
                use_shared_store: false,
            })
            .unwrap();
        let session_supervisor_manager =
            SessionManager::with_data_home(root, AzothDataHome::new(root.join("azoth-test-home")))
                .unwrap();
        let run_dir = session_supervisor_manager
            .sessions_dir()
            .join(session_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut manifest = SessionManifest::new(
            session_id,
            "local.editor_attach".to_string(),
            "editor".to_string(),
            root.to_path_buf(),
            workspace_path,
            run_dir.clone(),
            0,
        );
        manifest.activate(0);
        (session_supervisor_manager, manifest, run_dir)
    }

    /// Records the four live services in the session manifest and writes it
    /// where the session supervisor serves it from.
    fn publish_attach_fixture_manifest(
        manifest: &mut SessionManifest,
        services: [&ServiceDescriptor; 4],
        run_dir: &std::path::Path,
    ) {
        let [
            project_host,
            asset_processor,
            runtime_host,
            session_supervisor,
        ] = services;
        // Project-owned services are attached, not claimed: their current
        // process records remain in the project's supervision scope.
        for descriptor in [project_host, asset_processor] {
            manifest.attach_service_descriptor(descriptor, 1).unwrap();
        }
        for descriptor in [runtime_host, session_supervisor] {
            manifest.upsert_service_descriptor(descriptor, 1).unwrap();
        }
        let project_id = manifest.project_id.clone();
        manifest.upsert_process_record(
            local_running_process_for_descriptor(runtime_host, run_dir, &project_id),
            2,
        );
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&*manifest).unwrap(),
        )
        .unwrap();
    }

    /// Registers the fixture workspace's asset source roots against a fresh
    /// `AssetDB`. Attach verifies its own counts against this registration.
    fn register_fixture_asset_source_roots(
        asset_db: &std::path::Path,
        manifest: &SessionManifest,
    ) -> az_asset_processor::WorkspaceAssetSourceRootsRegistration {
        let registration = az_asset_processor::register_workspace_asset_source_roots_blocking(
            asset_db,
            &manifest.project_id,
            Some(&manifest.id.0.to_string()),
            &manifest.workspace_root,
            "main",
            0,
            // The engine's own composition, which is what classifies
            // engine-owned source families before any worker connects
            // (asset-contract 014, D7).
            az_asset_processor::engine_host_registries(),
        )
        .unwrap();
        assert!(!registration.source_roots.is_empty());
        registration
    }

    /// Registers the project and its session supervisor with a live daemon, then
    /// attaches twice: once naming the session, once letting the daemon discover
    /// it. The daemon has no part in what follows, so it stops here.
    fn attach_twice_through_daemon(
        project_root: &std::path::Path,
        session_supervisor: &ServiceDescriptor,
    ) -> (EditorAttachSession, EditorAttachSession) {
        let daemon = AzDaemon::new();
        daemon.register_project_root(project_root).unwrap();
        daemon
            .register_session_supervisor("local.editor_attach", "editor", session_supervisor)
            .unwrap();
        let daemon_server = start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let named = attach_to_session_via_daemon(
            project_root,
            Some("editor"),
            daemon_server.endpoint().clone(),
        )
        .unwrap();
        let discovered =
            attach_to_session_via_daemon(project_root, None, daemon_server.endpoint().clone())
                .unwrap();
        daemon_server.stop();
        (named, discovered)
    }

    /// What a verified attach against the fixture above must report back.
    struct ExpectedAttach<'a> {
        project_id: &'a str,
        project_host: &'a ServiceDescriptor,
        asset_processor: &'a ServiceDescriptor,
        runtime_host: &'a ServiceDescriptor,
        asset_registration: &'a az_asset_processor::WorkspaceAssetSourceRootsRegistration,
    }

    /// The named attach carries the manifest's identity, the exact descriptors
    /// the manifest published, and each host's verification counts; the
    /// discovered attach resolves to the same session.
    fn assert_attach_matches_fixture(
        daemon_attach: &EditorAttachSession,
        daemon_discovered_attach: &EditorAttachSession,
        expected: &ExpectedAttach<'_>,
    ) {
        assert_attach_reports_the_published_descriptors(daemon_attach, expected);
        assert_attach_reports_each_host_verification(daemon_attach, expected);
        assert_discovered_attach_resolves_the_same_session(daemon_attach, daemon_discovered_attach);
    }

    /// The attach carries the manifest's identity and the exact service
    /// descriptors the manifest published.
    fn assert_attach_reports_the_published_descriptors(
        daemon_attach: &EditorAttachSession,
        expected: &ExpectedAttach<'_>,
    ) {
        assert_eq!(daemon_attach.session_slug, "editor");
        assert_eq!(daemon_attach.project_id, expected.project_id);
        assert_eq!(daemon_attach.services.project_host, *expected.project_host);
        assert_eq!(
            daemon_attach.services.asset_processor,
            *expected.asset_processor
        );
        assert_eq!(
            daemon_attach.services.runtime_host.as_ref(),
            Some(expected.runtime_host)
        );
    }

    /// Each host answered its verification RPC, and the counts attach
    /// recorded match what the fixture registered.
    fn assert_attach_reports_each_host_verification(
        daemon_attach: &EditorAttachSession,
        expected: &ExpectedAttach<'_>,
    ) {
        assert_eq!(
            daemon_attach.verification.project_host_type_count,
            daemon_attach.type_registry.types.len()
        );
        assert!(
            daemon_attach
                .type_registry
                .types
                .iter()
                .any(
                    |descriptor| descriptor.editor_attributes.label.as_deref() == Some("Transform")
                )
        );
        assert_eq!(
            daemon_attach.verification.asset_processor_workspace_id,
            expected.asset_registration.workspace_id
        );
        assert_eq!(
            daemon_attach.verification.asset_processor_root_count,
            expected.asset_registration.source_roots.len()
        );
        assert!(daemon_attach.verification.runtime_host_verified);
        assert!(
            daemon_attach
                .verification
                .runtime_host_projection_count
                .is_some()
        );
    }

    /// Attaching without naming the session resolves the same one.
    fn assert_discovered_attach_resolves_the_same_session(
        daemon_attach: &EditorAttachSession,
        daemon_discovered_attach: &EditorAttachSession,
    ) {
        assert_eq!(
            daemon_discovered_attach.session_slug,
            daemon_attach.session_slug
        );
        assert_eq!(
            daemon_discovered_attach.session_id,
            daemon_attach.session_id
        );
        assert_eq!(daemon_discovered_attach.services, daemon_attach.services);
    }

    #[test]
    fn attaches_through_session_supervisor_and_verifies_live_service_rpcs() {
        let temp = tempfile::tempdir().unwrap();
        // Attach resolves Lore status against a real repository whose remote
        // is a live Lore server; without one this is an integration test, not
        // a unit test. Set AZOTH_LORE_TEST_REMOTE (e.g. `lore://host:41337/x`)
        // to enable it.
        let Ok(lore_remote) = std::env::var("AZOTH_LORE_TEST_REMOTE") else {
            eprintln!(
                "skipping attaches_through_session_supervisor_and_verifies_live_service_rpcs: \
                 AZOTH_LORE_TEST_REMOTE must point at a live Lore remote"
            );
            return;
        };
        let (session_supervisor_manager, mut manifest, run_dir) =
            build_attach_fixture(temp.path(), lore_remote);

        let mut project_host = project_host_service_descriptor(
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let project_host_server =
            az_project_host::start_project_host_rpc_server_with_capability_grants(
                temp.path(),
                project_host.endpoint.clone(),
                CapabilityGrantSet::from_grants(project_host.capabilities.clone()),
            )
            .unwrap();
        project_host.endpoint = project_host_server.endpoint().clone();
        let asset_db = temp.path().join("assetdb.sqlite");
        let asset_registration = register_fixture_asset_source_roots(&asset_db, &manifest);
        let mut asset_processor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let asset_processor_server =
            az_asset_processor::start_asset_processor_rpc_server_with_capability_grants(
                &asset_db,
                asset_processor.endpoint.clone(),
                CapabilityGrantSet::from_grants(asset_processor.capabilities.clone()),
            )
            .unwrap();
        asset_processor.endpoint = asset_processor_server.endpoint().clone();
        let mut runtime_host = runtime_host_service_descriptor(
            manifest.id.0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let runtime_host_server =
            az_runtime_host::start_runtime_host_rpc_server_for_session_with_capability_grants(
                runtime_host.endpoint.clone(),
                manifest.id.0.to_string(),
                run_dir.join("side-channels").join("runtime-host"),
                CapabilityGrantSet::from_grants(runtime_host.capabilities.clone()),
                // The editor composes nothing of its own, so this host serves
                // an empty composition and launches metadata-only runtimes
                // until the prebuilt host composes projections (Push 2).
                runtime_host_registries(),
            )
            .unwrap();
        runtime_host.endpoint = runtime_host_server.endpoint().clone();
        let mut session_supervisor = session_supervisor_service_descriptor(
            manifest.id.0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let session_supervisor_server = start_session_supervisor_rpc_server_with_manager(
            session_supervisor_manager,
            session_supervisor.endpoint.clone(),
            &manifest.slug,
        )
        .unwrap();
        session_supervisor.endpoint = session_supervisor_server.endpoint().clone();

        publish_attach_fixture_manifest(
            &mut manifest,
            [
                &project_host,
                &asset_processor,
                &runtime_host,
                &session_supervisor,
            ],
            &run_dir,
        );

        let (daemon_attach, daemon_discovered_attach) =
            attach_twice_through_daemon(temp.path(), &session_supervisor);

        assert_attach_matches_fixture(
            &daemon_attach,
            &daemon_discovered_attach,
            &ExpectedAttach {
                project_id: &manifest.project_id,
                project_host: &project_host,
                asset_processor: &asset_processor,
                runtime_host: &runtime_host,
                asset_registration: &asset_registration,
            },
        );

        session_supervisor_server.stop();
        project_host_server
            .stop()
            .expect("stop project-host server");
        asset_processor_server
            .stop()
            .expect("stop asset-processor server");
        runtime_host_server
            .stop()
            .expect("stop runtime-host server");
    }

    // ---- attaching through a project endpoint record ---------------------

    /// A record left by a *different protocol build* of azd is stale first and
    /// foreign second: the daemon that wrote it is usually long gone, so its
    /// recorded protocol is a claim about nobody. While that claim was checked
    /// ahead of any connect, such a record failed the attach hard and evicted
    /// nothing, so nothing on this path could ever clear it and the
    /// machine-wide record had to be deleted by hand — the same asymmetry
    /// ticket 044 fixed in bootstrap. It now takes the ordinary stale-record
    /// route.
    ///
    /// Attach's consequence is its own: it starts no sidecar, so the record is
    /// evicted *and* the transport failure is surfaced, rather than becoming a
    /// silent recovery.
    #[test]
    fn a_stale_foreign_protocol_record_is_evicted_by_attach_like_any_other_stale_record() {
        let root = record_test_project("attach-record", "stale-protocol-record");
        let record_path = write_foreign_protocol_record(&root, &unreachable_local_endpoint());
        assert!(
            record_path.is_file(),
            "test setup must leave a record on disk"
        );

        let error = attach_to_session(&root, None).unwrap_err();

        assert!(
            is_daemon_transport_failure(&error),
            "a foreign-protocol record whose daemon is gone is stale rather than \
             a version conflict, and must reach the eviction arm; got {error:?}"
        );
        assert!(
            !record_path.is_file(),
            "the stale record must be removed so a restarted azd can republish"
        );
    }

    /// The other half of that split. A foreign-protocol record at an address
    /// where a daemon *is* answering is a real version conflict: attach fails
    /// fast and leaves the record alone, because evicting it would discard the
    /// only evidence of the conflict the user has.
    ///
    /// The live peer is a current-protocol azd, since that is the only kind
    /// this build can start; the mismatch is therefore the recorded field
    /// against `ProtocolVersion::CURRENT`, which is what the record gate
    /// compares. A peer that truly speaks another protocol fails one gate
    /// earlier, inside the probe's own preflight — also a non-transport error,
    /// so also surfaced rather than evicted.
    ///
    /// Fail-fast is pinned by the daemon rather than by a clock: attach's first
    /// two acts past the gate are touching an editor lease and registering the
    /// project root, and this daemon shows neither.
    #[test]
    fn a_live_daemon_under_a_foreign_protocol_record_fails_attach_fast_and_is_left_on_disk() {
        let root = record_test_project("attach-record", "live-protocol-record");
        let daemon = AzDaemon::new();
        let server = start_az_daemon_rpc_server_with_daemon(
            daemon.clone(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let record_path = write_foreign_protocol_record(&root, server.endpoint());

        let outcome = attach_to_session(&root, None);

        server.stop();
        let error = outcome.expect_err("a live protocol conflict must fail the attach");
        let version = foreign_protocol_version();
        let rejected = format!(
            "service protocol version {}.{}.{}",
            version.major, version.minor, version.patch
        );
        assert!(
            matches!(&error, EditorError::ServiceDiscovery(message)
                if message.contains("azd unavailable until restarted")
                    && message.contains(&rejected)),
            "expected the recorded protocol to be the thing rejected, got {error:?}"
        );
        assert!(
            daemon.list_projects().is_empty(),
            "the record gate must fail before attach registers its project root"
        );
        assert_eq!(
            daemon.active_editor_lease_count(),
            0,
            "the record gate must fail before attach touches an editor lease"
        );
        assert!(
            record_path.is_file(),
            "a live version conflict is the user's to resolve, so attach fails \
             without touching the record"
        );

        std::fs::remove_file(&record_path).unwrap();
    }
}
