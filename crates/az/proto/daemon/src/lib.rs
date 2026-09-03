//! Cap'n Proto protocol types for the user-level `azd` daemon.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod daemon_capnp, "azoth/daemon_capnp.rs");

use std::path::{Path, PathBuf};

use capnp::Error;
use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

pub use az_proto_core as core;
pub use az_proto_session as session;

pub const DAEMON_NAMESPACE: &str = "azoth";
pub const DAEMON_SERVICE_NAME: &str = "daemon";
pub const DAEMON_AUDIENCE: &str = "daemon";
pub const DAEMON_READ_PERMISSION: &str = "daemon.read";
pub const DAEMON_PROJECTS_PERMISSION: &str = "daemon.projects";
pub const DAEMON_SESSIONS_PERMISSION: &str = "daemon.sessions";
pub const DAEMON_CONTROL_PERMISSION: &str = "daemon.control";
pub const DAEMON_LEASE_PERMISSION: &str = "daemon.lease";

#[derive(Debug, ThisError)]
pub enum DaemonProjectRegistryError {
    #[error("failed to read azd project registry {path}: {source}")]
    ProjectRegistryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write azd project registry {path}: {source}")]
    ProjectRegistryWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse azd project registry {path}: {source}")]
    ProjectRegistryParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to encode azd project registry: {0}")]
    ProjectRegistryEncode(#[from] toml::ser::Error),

    #[error("azd project registry {path} has schema version `{actual}`, expected `{expected}`")]
    UnsupportedProjectRegistrySchema {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectRegistryFile {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    projects: Vec<ProjectRecord>,
}

pub const PROJECT_REGISTRY_SCHEMA_VERSION: u32 = 1;

/// # Errors
///
/// Returns [`DaemonProjectRegistryError::ProjectRegistryRead`] if `path` exists
/// but cannot be read, [`DaemonProjectRegistryError::ProjectRegistryParse`] if
/// its contents are not valid registry TOML, and
/// [`DaemonProjectRegistryError::UnsupportedProjectRegistrySchema`] if the file
/// declares a schema version other than
/// [`PROJECT_REGISTRY_SCHEMA_VERSION`]. A missing file is not an error: it
/// reads as an empty registry.
pub fn read_project_registry(
    path: &Path,
) -> Result<Vec<ProjectRecord>, DaemonProjectRegistryError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(DaemonProjectRegistryError::ProjectRegistryRead {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let file: ProjectRegistryFile = toml::from_str(&contents).map_err(|source| {
        DaemonProjectRegistryError::ProjectRegistryParse {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if file.schema_version != PROJECT_REGISTRY_SCHEMA_VERSION {
        return Err(
            DaemonProjectRegistryError::UnsupportedProjectRegistrySchema {
                path: path.to_path_buf(),
                expected: PROJECT_REGISTRY_SCHEMA_VERSION,
                actual: file.schema_version,
            },
        );
    }

    Ok(file
        .projects
        .into_iter()
        .filter(|project| {
            validate_project_record_for_transport("project registry", project).is_ok()
        })
        .collect())
}

/// # Errors
///
/// Returns [`DaemonProjectRegistryError::ProjectRegistryWrite`] if the parent
/// directory cannot be created, if the temporary file cannot be written, if a
/// pre-existing registry cannot be removed, or if the temporary file cannot be
/// renamed over `path`. Returns
/// [`DaemonProjectRegistryError::ProjectRegistryEncode`] if the records cannot
/// be serialized to TOML.
pub fn write_project_registry(
    path: &Path,
    projects: &[ProjectRecord],
) -> Result<(), DaemonProjectRegistryError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            DaemonProjectRegistryError::ProjectRegistryWrite {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let file = ProjectRegistryFile {
        schema_version: PROJECT_REGISTRY_SCHEMA_VERSION,
        projects: projects
            .iter()
            .filter(|project| {
                validate_project_record_for_transport("project registry", project).is_ok()
            })
            .cloned()
            .collect(),
    };
    let contents = toml::to_string_pretty(&file)?;
    let temp_path = path.with_extension("toml.tmp");
    std::fs::write(&temp_path, contents).map_err(|source| {
        DaemonProjectRegistryError::ProjectRegistryWrite {
            path: temp_path.clone(),
            source,
        }
    })?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(DaemonProjectRegistryError::ProjectRegistryWrite {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    std::fs::rename(&temp_path, path).map_err(|source| {
        DaemonProjectRegistryError::ProjectRegistryWrite {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Remove one project registration and return the record that was removed.
///
/// Forgetting an already absent registration is idempotent.
///
/// # Errors
///
/// Returns any error [`read_project_registry`] returns for `path`, or any error
/// [`write_project_registry`] returns when persisting the shortened registry.
pub fn forget_project_registration(
    path: &Path,
    project_id: &str,
) -> Result<Option<ProjectRecord>, DaemonProjectRegistryError> {
    let mut projects = read_project_registry(path)?;
    let Some(index) = projects
        .iter()
        .position(|project| project.project_id == project_id)
    else {
        return Ok(None);
    };
    let removed = projects.remove(index);
    write_project_registry(path, &projects)?;
    Ok(Some(removed))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub project_id: String,
    pub name: String,
    pub root: String,
    pub manifest_path: String,
    pub engine_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectResult {
    pub project: Option<ProjectRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterProjectRequest {
    pub capability: core::Capability,
    pub project: ProjectRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterProjectRootRequest {
    pub capability: core::Capability,
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveProjectRequest {
    pub capability: core::Capability,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListProjectsRequest {
    pub capability: core::Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListProjectsResult {
    pub projects: Vec<ProjectRecord>,
    pub protocol_version: core::ProtocolVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterSessionSupervisorRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_slug: String,
    pub descriptor: core::ServiceDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnregisterSessionSupervisorRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_slug: String,
    pub descriptor: core::ServiceDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnregisterSessionSupervisorResult {
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveSessionSupervisorRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSessionSupervisorsRequest {
    pub capability: core::Capability,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorDescriptor {
    pub session_slug: String,
    pub descriptor: core::ServiceDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSessionSupervisorsResult {
    pub supervisors: Vec<SessionSupervisorDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorResult {
    pub descriptor: Option<core::ServiceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureProjectSessionRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSessionResult {
    pub manifest: session::SessionManifest,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareProjectSessionServicesRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_slug: String,
    pub endpoint_kind: core::EndpointKind,
    pub skip_build: bool,
    pub service_names: Vec<String>,
    pub otlp_endpoint: Option<String>,
    pub recover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSessionServicesResult {
    pub manifest: session::SessionManifest,
    pub service_names: Vec<String>,
    pub build_command_count: u32,
    pub prepared_process_count: u32,
    pub built: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureProjectSessionServicesRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_name: String,
    pub endpoint_kind: core::EndpointKind,
    pub skip_build: bool,
    pub start_service_names: Vec<String>,
    pub timeout_ms: u64,
    pub daemon_endpoint: core::Endpoint,
}

// The four bools mirror four separately-numbered `Bool` fields in daemon.capnp
// (`built @5`, `sessionCreated @6`, `reusedSupervisor @8`, `startRequested @9`);
// collapsing them into a flags type would break the field-for-field
// correspondence the write/read pair is audited against.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSessionServicesStartResult {
    pub manifest: session::SessionManifest,
    pub service_names: Vec<String>,
    pub running_service_names: Vec<String>,
    pub build_command_count: u32,
    pub prepared_process_count: u32,
    pub built: bool,
    pub session_created: bool,
    pub supervisor: core::ServiceDescriptor,
    pub reused_supervisor: bool,
    pub start_requested: bool,
    pub sessiond_pid: u32,
    pub log_path: String,
}

/// Open-project phase taxonomy carried on the wire.
///
/// The frozen aggregate weights live with the daemon's phase aggregator; this
/// type is only the transport shape plus its capnp mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenProjectPhase {
    ResolvePlan,
    Build,
    StartServices,
    Attach,
    LoadCatalogs,
}

impl OpenProjectPhase {
    #[must_use]
    pub const fn to_capnp(self) -> daemon_capnp::OpenProjectPhase {
        match self {
            Self::ResolvePlan => daemon_capnp::OpenProjectPhase::ResolvePlan,
            Self::Build => daemon_capnp::OpenProjectPhase::Build,
            Self::StartServices => daemon_capnp::OpenProjectPhase::StartServices,
            Self::Attach => daemon_capnp::OpenProjectPhase::Attach,
            Self::LoadCatalogs => daemon_capnp::OpenProjectPhase::LoadCatalogs,
        }
    }

    #[must_use]
    pub const fn from_capnp(phase: daemon_capnp::OpenProjectPhase) -> Self {
        match phase {
            daemon_capnp::OpenProjectPhase::ResolvePlan => Self::ResolvePlan,
            daemon_capnp::OpenProjectPhase::Build => Self::Build,
            daemon_capnp::OpenProjectPhase::StartServices => Self::StartServices,
            daemon_capnp::OpenProjectPhase::Attach => Self::Attach,
            daemon_capnp::OpenProjectPhase::LoadCatalogs => Self::LoadCatalogs,
        }
    }
}

/// One streamed open-project progress event (wire form).
///
/// `done_bp` is the aggregate completion in basis points (`0..=10000`);
/// `phase_done`/`phase_total` are the raw fraction for `phase`, where
/// `phase_total == 0` means the phase total is not yet known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectOpenProgressEvent {
    pub seq: u64,
    pub phase: OpenProjectPhase,
    pub done_bp: u32,
    pub phase_done: u64,
    pub phase_total: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProjectBuildRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub profile: String,
    pub target_triple: Option<String>,
    pub package_selectors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteProjectBuildRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub profile: String,
    pub target_triple: Option<String>,
    pub package_selectors: Vec<String>,
}

/// One streamed project-build progress event (wire form).
///
/// `done_bp` is aggregate build completion in basis points (`0..=10000`).
/// `command_done`/`command_total` are the raw fraction for the active command,
/// where `command_total == 0` means the command total is not yet known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildProgressEvent {
    pub seq: u64,
    /// Zero-based index into the returned build plan's command list.
    pub command_index: u32,
    pub command_count: u32,
    pub target_name: String,
    pub done_bp: u32,
    pub command_done: u64,
    pub command_total: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildCommand {
    pub owner_id: String,
    pub owner_root: String,
    pub target_name: String,
    pub program: String,
    pub cwd: String,
    pub args: Vec<String>,
    /// Set as `CARGO_TARGET_DIR` only for generated-workspace invocations.
    pub cargo_target_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildPackageProfile {
    pub name: String,
    pub asset_platform: String,
    pub cargo_profile: String,
    pub container: String,
    pub compression: String,
    pub oodle_compressor: Option<String>,
    pub oodle_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildPlan {
    pub commands: Vec<ProjectBuildCommand>,
    pub package_profile: Option<ProjectBuildPackageProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildExecutionResult {
    pub success: bool,
    pub plan: ProjectBuildPlan,
    pub completed_command_count: u32,
    pub failing_command: Option<ProjectBuildCommand>,
    pub diagnostic_headline: String,
    pub diagnostic_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProjectServicesRequest {
    pub capability: core::Capability,
    pub project_id: String,
    pub session_slug: String,
    pub endpoint_kind: core::EndpointKind,
    pub workspace_root: Option<String>,
    pub service_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectServiceCommand {
    pub owner_id: String,
    pub owner_root: String,
    pub build_output_root: String,
    pub service_name: String,
    pub role: core::ServiceRole,
    pub endpoint: core::Endpoint,
    pub program: String,
    pub cwd: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectServicePlan {
    pub build_commands: Vec<ProjectBuildCommand>,
    pub commands: Vec<ProjectServiceCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownDaemonRequest {
    pub capability: core::Capability,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownDaemonResult {
    pub accepted: bool,
    pub reason: String,
}

/// Transport projection of the process identity carried by the daemon ABI.
///
/// OS-specific identity capture and liveness assessment remain owned by
/// `az-service-supervision`; service boundaries explicitly lower this wire
/// value into that domain type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub process_id: u32,
    pub process_start_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchEditorLeaseRequest {
    pub capability: core::Capability,
    pub lease_id: String,
    pub owner_process: ProcessIdentity,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchEditorLeaseResult {
    pub accepted: bool,
    pub lease_id: String,
    pub active_lease_count: u32,
}

#[must_use]
pub fn editor_process_lease_id(owner_process: ProcessIdentity) -> String {
    format!(
        "editor-process:{}:{}",
        owner_process.process_id, owner_process.process_start_time
    )
}

fn validate_non_empty_text(kind: &str, field: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(invalid_daemon_protocol_value(
            kind,
            format!("{field} cannot be empty"),
        ));
    }
    Ok(())
}

fn validate_endpoint_kind_for_transport(
    kind: core::EndpointKind,
    operation: &'static str,
) -> Result<(), Error> {
    if matches!(kind, core::EndpointKind::InProcess) {
        return Err(invalid_daemon_protocol_value(
            operation,
            "in-process endpoints are test-only; use platform IPC or explicit TCP debug endpoints",
        ));
    }
    Ok(())
}

fn validate_endpoint_for_transport(
    endpoint: &core::Endpoint,
    kind: &'static str,
) -> Result<(), Error> {
    validate_endpoint_kind_for_transport(endpoint.kind, kind)?;
    validate_non_empty_text(kind, "endpoint address", &endpoint.address)
}

fn validate_session_supervisor_descriptor_for_transport(
    descriptor: &core::ServiceDescriptor,
) -> Result<(), Error> {
    if descriptor.role != core::ServiceRole::SessionSupervisor {
        return Err(invalid_daemon_protocol_value(
            "session supervisor descriptor",
            format!("expected role SessionSupervisor, got {:?}", descriptor.role),
        ));
    }
    validate_endpoint_for_transport(&descriptor.endpoint, "session supervisor descriptor")?;
    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            invalid_daemon_protocol_value(
                "session supervisor descriptor",
                format!("invalid brokered capability templates: {error}"),
            )
        })
}

fn validate_session_supervisor_listing_for_transport(
    descriptor: &SessionSupervisorDescriptor,
) -> Result<(), Error> {
    validate_non_empty_text(
        "session supervisor descriptor",
        "session slug",
        &descriptor.session_slug,
    )?;
    validate_session_supervisor_descriptor_for_transport(&descriptor.descriptor)
}

fn validate_project_service_command_for_transport(
    command: &ProjectServiceCommand,
) -> Result<(), Error> {
    validate_non_empty_text("project service command", "owner id", &command.owner_id)?;
    validate_non_empty_text("project service command", "owner root", &command.owner_root)?;
    validate_non_empty_text(
        "project service command",
        "build output root",
        &command.build_output_root,
    )?;
    validate_non_empty_text(
        "project service command",
        "service name",
        &command.service_name,
    )?;
    if !matches!(
        command.role,
        core::ServiceRole::ProjectHost
            | core::ServiceRole::AssetProcessor
            | core::ServiceRole::RuntimeHost
            | core::ServiceRole::Worker
    ) {
        return Err(invalid_daemon_protocol_value(
            "project service command",
            format!("unsupported service role {:?}", command.role),
        ));
    }
    validate_endpoint_for_transport(&command.endpoint, "project service command")?;
    validate_non_empty_text("project service command", "program", &command.program)?;
    validate_non_empty_text("project service command", "cwd", &command.cwd)
}

fn validate_project_service_plan_for_transport(plan: &ProjectServicePlan) -> Result<(), Error> {
    if plan.commands.is_empty() {
        return Err(invalid_daemon_protocol_value(
            "project service plan",
            "service commands cannot be empty",
        ));
    }
    for command in &plan.commands {
        validate_project_service_command_for_transport(command)?;
    }
    for command in &plan.build_commands {
        validate_project_build_command_for_transport(command)?;
    }
    Ok(())
}

fn validate_daemon_capability_request_for_transport(
    kind: &'static str,
    capability: &core::Capability,
    required_permission: &'static str,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_daemon_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != DAEMON_AUDIENCE {
        return Err(invalid_daemon_protocol_value(
            kind,
            format!(
                "capability audience must be `{DAEMON_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if !matches!(
        capability.role,
        core::ServiceRole::Editor
            | core::ServiceRole::Daemon
            | core::ServiceRole::SessionSupervisor
    ) {
        return Err(invalid_daemon_protocol_value(
            kind,
            format!("capability role {:?} is not allowed", capability.role),
        ));
    }
    let has_required = capability.has_permissions(&[required_permission]);
    let has_projects = capability.has_permissions(&[DAEMON_PROJECTS_PERMISSION]);
    let has_sessions = capability.has_permissions(&[DAEMON_SESSIONS_PERMISSION]);
    if !has_required
        && !(required_permission == DAEMON_READ_PERMISSION && (has_projects || has_sessions))
    {
        return Err(invalid_daemon_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_daemon_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_project_record_for_transport(
    kind: &'static str,
    project: &ProjectRecord,
) -> Result<(), Error> {
    validate_non_empty_text(kind, "project id", &project.project_id)?;
    validate_non_empty_text(kind, "project name", &project.name)?;
    validate_non_empty_text(kind, "project root", &project.root)?;
    validate_non_empty_text(kind, "project manifest path", &project.manifest_path)?;
    validate_non_empty_text(kind, "engine version", &project.engine_version)
}

fn validate_optional_non_empty_text(
    kind: &'static str,
    field: &str,
    value: Option<&str>,
) -> Result<(), Error> {
    if let Some(value) = value {
        validate_non_empty_text(kind, field, value)?;
    }
    Ok(())
}

fn validate_register_project_request_for_transport(
    request: &RegisterProjectRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "register project request",
        &request.capability,
        DAEMON_PROJECTS_PERMISSION,
    )?;
    validate_project_record_for_transport("register project request", &request.project)
}

fn validate_register_project_root_request_for_transport(
    request: &RegisterProjectRootRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "register project root request",
        &request.capability,
        DAEMON_PROJECTS_PERMISSION,
    )?;
    validate_non_empty_text(
        "register project root request",
        "project root",
        &request.root,
    )
}

fn validate_resolve_project_request_for_transport(
    request: &ResolveProjectRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "resolve project request",
        &request.capability,
        DAEMON_READ_PERMISSION,
    )?;
    validate_non_empty_text("resolve project request", "project id", &request.project_id)
}

fn validate_list_projects_request_for_transport(
    request: &ListProjectsRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "list projects request",
        &request.capability,
        DAEMON_READ_PERMISSION,
    )
}

fn validate_register_session_supervisor_request_for_transport(
    request: &RegisterSessionSupervisorRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "register session supervisor request",
        &request.capability,
        DAEMON_SESSIONS_PERMISSION,
    )?;
    validate_non_empty_text(
        "register session supervisor request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "register session supervisor request",
        "session slug",
        &request.session_slug,
    )?;
    validate_session_supervisor_descriptor_for_transport(&request.descriptor)
}

fn validate_unregister_session_supervisor_request_for_transport(
    request: &UnregisterSessionSupervisorRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "unregister session supervisor request",
        &request.capability,
        DAEMON_SESSIONS_PERMISSION,
    )?;
    validate_non_empty_text(
        "unregister session supervisor request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "unregister session supervisor request",
        "session slug",
        &request.session_slug,
    )?;
    validate_session_supervisor_descriptor_for_transport(&request.descriptor)
}

fn validate_resolve_session_supervisor_request_for_transport(
    request: &ResolveSessionSupervisorRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "resolve session supervisor request",
        &request.capability,
        DAEMON_READ_PERMISSION,
    )?;
    validate_non_empty_text(
        "resolve session supervisor request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "resolve session supervisor request",
        "session slug",
        &request.session_slug,
    )
}

fn validate_list_session_supervisors_request_for_transport(
    request: &ListSessionSupervisorsRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "list session supervisors request",
        &request.capability,
        DAEMON_READ_PERMISSION,
    )?;
    validate_non_empty_text(
        "list session supervisors request",
        "project id",
        &request.project_id,
    )
}

fn validate_ensure_project_session_request_for_transport(
    request: &EnsureProjectSessionRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "ensure project session request",
        &request.capability,
        DAEMON_SESSIONS_PERMISSION,
    )?;
    validate_non_empty_text(
        "ensure project session request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "ensure project session request",
        "session name",
        &request.session_name,
    )?;
    Ok(())
}

fn validate_prepare_project_session_services_request_for_transport(
    request: &PrepareProjectSessionServicesRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "prepare project session services request",
        &request.capability,
        DAEMON_SESSIONS_PERMISSION,
    )?;
    validate_non_empty_text(
        "prepare project session services request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "prepare project session services request",
        "session slug",
        &request.session_slug,
    )?;
    validate_endpoint_kind_for_transport(
        request.endpoint_kind,
        "prepare project session services request",
    )?;
    for service_name in &request.service_names {
        validate_non_empty_text(
            "prepare project session services request",
            "service name",
            service_name,
        )?;
    }
    validate_optional_non_empty_text(
        "prepare project session services request",
        "OTLP endpoint",
        request.otlp_endpoint.as_deref(),
    )
}

fn validate_ensure_project_session_services_request_for_transport(
    request: &EnsureProjectSessionServicesRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "ensure project session services request",
        &request.capability,
        DAEMON_SESSIONS_PERMISSION,
    )?;
    validate_non_empty_text(
        "ensure project session services request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "ensure project session services request",
        "session name",
        &request.session_name,
    )?;
    validate_endpoint_kind_for_transport(
        request.endpoint_kind,
        "ensure project session services request",
    )?;
    validate_endpoint_for_transport(
        &request.daemon_endpoint,
        "ensure project session services request daemon endpoint",
    )?;
    if request.timeout_ms == 0 {
        return Err(invalid_daemon_protocol_value(
            "ensure project session services request",
            "timeout_ms must be positive",
        ));
    }
    for service_name in &request.start_service_names {
        validate_non_empty_text(
            "ensure project session services request",
            "start service name",
            service_name,
        )?;
    }
    Ok(())
}

fn validate_plan_project_build_request_for_transport(
    request: &PlanProjectBuildRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "plan project build request",
        &request.capability,
        DAEMON_PROJECTS_PERMISSION,
    )?;
    validate_non_empty_text(
        "plan project build request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text("plan project build request", "profile", &request.profile)?;
    validate_optional_non_empty_text(
        "plan project build request",
        "target triple",
        request.target_triple.as_deref(),
    )?;
    for selector in &request.package_selectors {
        validate_non_empty_text("plan project build request", "package selector", selector)?;
    }
    Ok(())
}

fn validate_execute_project_build_request_for_transport(
    request: &ExecuteProjectBuildRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "execute project build request",
        &request.capability,
        DAEMON_PROJECTS_PERMISSION,
    )?;
    validate_non_empty_text(
        "execute project build request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text("execute project build request", "profile", &request.profile)?;
    validate_optional_non_empty_text(
        "execute project build request",
        "target triple",
        request.target_triple.as_deref(),
    )?;
    for selector in &request.package_selectors {
        validate_non_empty_text(
            "execute project build request",
            "package selector",
            selector,
        )?;
    }
    Ok(())
}

fn validate_plan_project_services_request_for_transport(
    request: &PlanProjectServicesRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "plan project services request",
        &request.capability,
        DAEMON_PROJECTS_PERMISSION,
    )?;
    validate_non_empty_text(
        "plan project services request",
        "project id",
        &request.project_id,
    )?;
    validate_non_empty_text(
        "plan project services request",
        "session slug",
        &request.session_slug,
    )?;
    validate_optional_non_empty_text(
        "plan project services request",
        "workspace root",
        request.workspace_root.as_deref(),
    )?;
    validate_endpoint_kind_for_transport(
        request.endpoint_kind,
        "plan project services request endpoint kind",
    )?;
    for service_name in &request.service_names {
        validate_non_empty_text(
            "plan project services request",
            "service name",
            service_name,
        )?;
    }
    Ok(())
}

fn validate_shutdown_daemon_request_for_transport(
    request: &ShutdownDaemonRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "shutdown daemon request",
        &request.capability,
        DAEMON_CONTROL_PERMISSION,
    )?;
    validate_non_empty_text("shutdown daemon request", "reason", &request.reason)
}

fn validate_touch_editor_lease_request_for_transport(
    request: &TouchEditorLeaseRequest,
) -> Result<(), Error> {
    validate_daemon_capability_request_for_transport(
        "touch editor lease request",
        &request.capability,
        DAEMON_LEASE_PERMISSION,
    )?;
    validate_non_empty_text("touch editor lease request", "lease id", &request.lease_id)?;
    validate_non_empty_text("touch editor lease request", "purpose", &request.purpose)?;
    if request.owner_process.process_id == 0 {
        return Err(invalid_daemon_protocol_value(
            "touch editor lease request",
            "owner process id must be non-zero",
        ));
    }
    if request.owner_process.process_start_time == 0 {
        return Err(invalid_daemon_protocol_value(
            "touch editor lease request",
            "owner process start time must be non-zero",
        ));
    }
    let canonical_lease_id = editor_process_lease_id(request.owner_process);
    if request.lease_id != canonical_lease_id {
        return Err(invalid_daemon_protocol_value(
            "touch editor lease request",
            format!("lease id must be `{canonical_lease_id}`"),
        ));
    }
    Ok(())
}

fn validate_touch_editor_lease_result_for_transport(
    result: &TouchEditorLeaseResult,
) -> Result<(), Error> {
    if !result.accepted {
        return Err(invalid_daemon_protocol_value(
            "touch editor lease result",
            "accepted must be true",
        ));
    }
    validate_non_empty_text("touch editor lease result", "lease id", &result.lease_id)
}

fn validate_project_build_command_for_transport(
    command: &ProjectBuildCommand,
) -> Result<(), Error> {
    validate_non_empty_text("project build command", "owner id", &command.owner_id)?;
    validate_non_empty_text("project build command", "owner root", &command.owner_root)?;
    validate_non_empty_text("project build command", "target name", &command.target_name)?;
    validate_non_empty_text("project build command", "program", &command.program)?;
    validate_non_empty_text("project build command", "cwd", &command.cwd)?;
    if let Some(target_dir) = &command.cargo_target_dir {
        validate_non_empty_text("project build command", "Cargo target dir", target_dir)?;
    }
    Ok(())
}

fn validate_project_build_package_profile_for_transport(
    profile: &ProjectBuildPackageProfile,
) -> Result<(), Error> {
    validate_non_empty_text("project build package profile", "name", &profile.name)?;
    validate_non_empty_text(
        "project build package profile",
        "asset platform",
        &profile.asset_platform,
    )?;
    validate_non_empty_text(
        "project build package profile",
        "cargo profile",
        &profile.cargo_profile,
    )?;
    validate_non_empty_text(
        "project build package profile",
        "container",
        &profile.container,
    )?;
    validate_non_empty_text(
        "project build package profile",
        "compression",
        &profile.compression,
    )?;
    validate_optional_non_empty_text(
        "project build package profile",
        "oodle compressor",
        profile.oodle_compressor.as_deref(),
    )?;
    validate_optional_non_empty_text(
        "project build package profile",
        "oodle effort",
        profile.oodle_effort.as_deref(),
    )?;
    match (
        profile.compression.as_str(),
        profile.oodle_compressor.as_deref(),
        profile.oodle_effort.as_deref(),
    ) {
        ("oodle", Some(_), Some(_)) => Ok(()),
        ("oodle", _, _) => Err(invalid_daemon_protocol_value(
            "project build package profile",
            "oodle compression requires oodle compressor and effort",
        )),
        (_, None, None) => Ok(()),
        (_, _, _) => Err(invalid_daemon_protocol_value(
            "project build package profile",
            "oodle settings require oodle compression",
        )),
    }
}

fn validate_project_build_plan_for_transport(plan: &ProjectBuildPlan) -> Result<(), Error> {
    if plan.commands.is_empty() {
        return Err(invalid_daemon_protocol_value(
            "project build plan",
            "build commands cannot be empty",
        ));
    }
    for command in &plan.commands {
        validate_project_build_command_for_transport(command)?;
    }
    if let Some(profile) = &plan.package_profile {
        validate_project_build_package_profile_for_transport(profile)?;
    }
    Ok(())
}

fn validate_project_build_execution_result_for_transport(
    result: &ProjectBuildExecutionResult,
) -> Result<(), Error> {
    validate_project_build_plan_for_transport(&result.plan)?;
    if result.completed_command_count as usize > result.plan.commands.len() {
        return Err(invalid_daemon_protocol_value(
            "project build execution result",
            "completed command count exceeds planned command count",
        ));
    }
    match (&result.success, &result.failing_command) {
        (true, None) => {}
        (true, Some(_)) => {
            return Err(invalid_daemon_protocol_value(
                "project build execution result",
                "successful result cannot include a failing command",
            ));
        }
        (false, Some(command)) => validate_project_build_command_for_transport(command)?,
        (false, None) => {
            return Err(invalid_daemon_protocol_value(
                "project build execution result",
                "failed result must include a failing command",
            ));
        }
    }
    Ok(())
}

fn validate_project_build_progress_event_for_transport(
    event: &ProjectBuildProgressEvent,
) -> Result<(), Error> {
    if event.seq == 0 {
        return Err(invalid_daemon_protocol_value(
            "project build progress event",
            "seq must be non-zero",
        ));
    }
    if event.command_count == 0 {
        return Err(invalid_daemon_protocol_value(
            "project build progress event",
            "command count must be positive",
        ));
    }
    if event.command_index >= event.command_count {
        return Err(invalid_daemon_protocol_value(
            "project build progress event",
            "command index must be within command count",
        ));
    }
    validate_non_empty_text(
        "project build progress event",
        "target name",
        &event.target_name,
    )?;
    if event.done_bp > 10_000 {
        return Err(invalid_daemon_protocol_value(
            "project build progress event",
            "done_bp cannot exceed 10000",
        ));
    }
    Ok(())
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

fn invalid_daemon_protocol_value(kind: &str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

fn write_project_record(
    project: &ProjectRecord,
    mut builder: daemon_capnp::project_record::Builder<'_>,
) {
    builder.set_project_id(&project.project_id);
    builder.set_name(&project.name);
    builder.set_root(&project.root);
    builder.set_manifest_path(&project.manifest_path);
    builder.set_engine_version(&project.engine_version);
}

fn read_project_record(
    reader: daemon_capnp::project_record::Reader<'_>,
) -> Result<ProjectRecord, Error> {
    Ok(ProjectRecord {
        project_id: reader.get_project_id()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        root: reader.get_root()?.to_string()?,
        manifest_path: reader.get_manifest_path()?.to_string()?,
        engine_version: reader.get_engine_version()?.to_string()?,
    })
}

fn write_project_result(
    result: &ProjectResult,
    mut builder: daemon_capnp::project_result::Builder<'_>,
) {
    match &result.project {
        Some(project) => (project).to_capnp(builder.init_project()),
        None => builder.set_none(()),
    }
}

fn read_project_result(
    reader: daemon_capnp::project_result::Reader<'_>,
) -> Result<ProjectResult, Error> {
    let project = match reader.which()? {
        daemon_capnp::project_result::Which::None(()) => None,
        daemon_capnp::project_result::Which::Project(project) => {
            Some(ProjectRecord::from_capnp(project?)?)
        }
    };
    Ok(ProjectResult { project })
}

fn write_register_project_request(
    request: &RegisterProjectRequest,
    mut builder: daemon_capnp::register_project_request::Builder<'_>,
) -> Result<(), Error> {
    validate_register_project_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    (request.project).to_capnp(builder.init_project());
    Ok(())
}

fn read_register_project_request(
    reader: daemon_capnp::register_project_request::Reader<'_>,
) -> Result<RegisterProjectRequest, Error> {
    let request = RegisterProjectRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project: ProjectRecord::from_capnp(reader.get_project()?)?,
    };
    validate_register_project_request_for_transport(&request)?;
    Ok(request)
}

fn write_register_project_root_request(
    request: &RegisterProjectRootRequest,
    mut builder: daemon_capnp::register_project_root_request::Builder<'_>,
) -> Result<(), Error> {
    validate_register_project_root_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_root(&request.root);
    Ok(())
}

fn read_register_project_root_request(
    reader: daemon_capnp::register_project_root_request::Reader<'_>,
) -> Result<RegisterProjectRootRequest, Error> {
    let request = RegisterProjectRootRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        root: reader.get_root()?.to_string()?,
    };
    validate_register_project_root_request_for_transport(&request)?;
    Ok(request)
}

fn write_resolve_project_request(
    request: &ResolveProjectRequest,
    mut builder: daemon_capnp::resolve_project_request::Builder<'_>,
) -> Result<(), Error> {
    validate_resolve_project_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    Ok(())
}

fn read_resolve_project_request(
    reader: daemon_capnp::resolve_project_request::Reader<'_>,
) -> Result<ResolveProjectRequest, Error> {
    let request = ResolveProjectRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
    };
    validate_resolve_project_request_for_transport(&request)?;
    Ok(request)
}

fn write_list_projects_request(
    request: &ListProjectsRequest,
    mut builder: daemon_capnp::list_projects_request::Builder<'_>,
) -> Result<(), Error> {
    validate_list_projects_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    Ok(())
}

fn read_list_projects_request(
    reader: daemon_capnp::list_projects_request::Reader<'_>,
) -> Result<ListProjectsRequest, Error> {
    let request = ListProjectsRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_list_projects_request_for_transport(&request)?;
    Ok(request)
}

fn write_list_projects_result(
    result: &ListProjectsResult,
    mut builder: daemon_capnp::list_projects_result::Builder<'_>,
) -> Result<(), Error> {
    let mut projects = builder
        .reborrow()
        .init_projects(capnp_list_index(result.projects.len())?);
    for (index, project) in result.projects.iter().enumerate() {
        (project).to_capnp(projects.reborrow().get(capnp_list_index(index)?));
    }
    result
        .protocol_version
        .to_capnp(builder.init_protocol_version());
    Ok(())
}

fn read_list_projects_result(
    reader: daemon_capnp::list_projects_result::Reader<'_>,
) -> Result<ListProjectsResult, Error> {
    let projects = reader
        .get_projects()?
        .iter()
        .map(read_project_record)
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(ListProjectsResult {
        projects,
        protocol_version: core::ProtocolVersion::from_capnp(reader.get_protocol_version()?),
    })
}

fn write_register_session_supervisor_request(
    request: &RegisterSessionSupervisorRequest,
    mut builder: daemon_capnp::register_session_supervisor_request::Builder<'_>,
) -> Result<(), Error> {
    validate_register_session_supervisor_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_slug(&request.session_slug);
    core::ServiceDescriptor::to_capnp(&request.descriptor, builder.init_descriptor())?;
    Ok(())
}

fn read_register_session_supervisor_request(
    reader: daemon_capnp::register_session_supervisor_request::Reader<'_>,
) -> Result<RegisterSessionSupervisorRequest, Error> {
    let request = RegisterSessionSupervisorRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
        descriptor: core::ServiceDescriptor::from_capnp(reader.get_descriptor()?)?,
    };
    validate_register_session_supervisor_request_for_transport(&request)?;
    Ok(request)
}

fn write_unregister_session_supervisor_request(
    request: &UnregisterSessionSupervisorRequest,
    mut builder: daemon_capnp::unregister_session_supervisor_request::Builder<'_>,
) -> Result<(), Error> {
    validate_unregister_session_supervisor_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_slug(&request.session_slug);
    core::ServiceDescriptor::to_capnp(&request.descriptor, builder.init_descriptor())?;
    Ok(())
}

fn read_unregister_session_supervisor_request(
    reader: daemon_capnp::unregister_session_supervisor_request::Reader<'_>,
) -> Result<UnregisterSessionSupervisorRequest, Error> {
    let request = UnregisterSessionSupervisorRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
        descriptor: core::ServiceDescriptor::from_capnp(reader.get_descriptor()?)?,
    };
    validate_unregister_session_supervisor_request_for_transport(&request)?;
    Ok(request)
}

fn write_unregister_session_supervisor_result(
    result: &UnregisterSessionSupervisorResult,
    mut builder: daemon_capnp::unregister_session_supervisor_result::Builder<'_>,
) {
    builder.set_removed(result.removed);
}

fn read_unregister_session_supervisor_result(
    reader: daemon_capnp::unregister_session_supervisor_result::Reader<'_>,
) -> UnregisterSessionSupervisorResult {
    UnregisterSessionSupervisorResult {
        removed: reader.get_removed(),
    }
}

fn write_resolve_session_supervisor_request(
    request: &ResolveSessionSupervisorRequest,
    mut builder: daemon_capnp::resolve_session_supervisor_request::Builder<'_>,
) -> Result<(), Error> {
    validate_resolve_session_supervisor_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_slug(&request.session_slug);
    Ok(())
}

fn read_resolve_session_supervisor_request(
    reader: daemon_capnp::resolve_session_supervisor_request::Reader<'_>,
) -> Result<ResolveSessionSupervisorRequest, Error> {
    let request = ResolveSessionSupervisorRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
    };
    validate_resolve_session_supervisor_request_for_transport(&request)?;
    Ok(request)
}

fn write_list_session_supervisors_request(
    request: &ListSessionSupervisorsRequest,
    mut builder: daemon_capnp::list_session_supervisors_request::Builder<'_>,
) -> Result<(), Error> {
    validate_list_session_supervisors_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    Ok(())
}

fn read_list_session_supervisors_request(
    reader: daemon_capnp::list_session_supervisors_request::Reader<'_>,
) -> Result<ListSessionSupervisorsRequest, Error> {
    let request = ListSessionSupervisorsRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
    };
    validate_list_session_supervisors_request_for_transport(&request)?;
    Ok(request)
}

fn write_session_supervisor_descriptor(
    descriptor: &SessionSupervisorDescriptor,
    mut builder: daemon_capnp::session_supervisor_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_session_supervisor_listing_for_transport(descriptor)?;
    builder.set_session_slug(&descriptor.session_slug);
    core::ServiceDescriptor::to_capnp(&descriptor.descriptor, builder.init_descriptor())?;
    Ok(())
}

fn read_session_supervisor_descriptor(
    reader: daemon_capnp::session_supervisor_descriptor::Reader<'_>,
) -> Result<SessionSupervisorDescriptor, Error> {
    let descriptor = SessionSupervisorDescriptor {
        session_slug: reader.get_session_slug()?.to_string()?,
        descriptor: core::ServiceDescriptor::from_capnp(reader.get_descriptor()?)?,
    };
    validate_session_supervisor_listing_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_list_session_supervisors_result(
    result: &ListSessionSupervisorsResult,
    mut builder: daemon_capnp::list_session_supervisors_result::Builder<'_>,
) -> Result<(), Error> {
    let mut supervisors = builder
        .reborrow()
        .init_supervisors(capnp_list_index(result.supervisors.len())?);
    for (index, descriptor) in result.supervisors.iter().enumerate() {
        (descriptor).to_capnp(supervisors.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_list_session_supervisors_result(
    reader: daemon_capnp::list_session_supervisors_result::Reader<'_>,
) -> Result<ListSessionSupervisorsResult, Error> {
    let supervisors = reader
        .get_supervisors()?
        .iter()
        .map(read_session_supervisor_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(ListSessionSupervisorsResult { supervisors })
}

fn write_session_supervisor_result(
    result: &SessionSupervisorResult,
    mut builder: daemon_capnp::session_supervisor_result::Builder<'_>,
) -> Result<(), Error> {
    match &result.descriptor {
        Some(descriptor) => {
            validate_session_supervisor_descriptor_for_transport(descriptor)?;
            core::ServiceDescriptor::to_capnp(descriptor, builder.init_descriptor())?;
        }
        None => builder.set_none(()),
    }
    Ok(())
}

fn read_session_supervisor_result(
    reader: daemon_capnp::session_supervisor_result::Reader<'_>,
) -> Result<SessionSupervisorResult, Error> {
    let descriptor = match reader.which()? {
        daemon_capnp::session_supervisor_result::Which::None(()) => None,
        daemon_capnp::session_supervisor_result::Which::Descriptor(descriptor) => {
            let descriptor = core::ServiceDescriptor::from_capnp(descriptor?)?;
            validate_session_supervisor_descriptor_for_transport(&descriptor)?;
            Some(descriptor)
        }
    };
    Ok(SessionSupervisorResult { descriptor })
}

fn write_ensure_project_session_request(
    request: &EnsureProjectSessionRequest,
    mut builder: daemon_capnp::ensure_project_session_request::Builder<'_>,
) -> Result<(), Error> {
    validate_ensure_project_session_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_name(&request.session_name);
    Ok(())
}

fn read_ensure_project_session_request(
    reader: daemon_capnp::ensure_project_session_request::Reader<'_>,
) -> Result<EnsureProjectSessionRequest, Error> {
    let request = EnsureProjectSessionRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_name: reader.get_session_name()?.to_string()?,
    };
    validate_ensure_project_session_request_for_transport(&request)?;
    Ok(request)
}

fn write_project_session_result(
    result: &ProjectSessionResult,
    mut builder: daemon_capnp::project_session_result::Builder<'_>,
) -> Result<(), Error> {
    (result.manifest).to_capnp(builder.reborrow().init_manifest())?;
    builder.set_created(result.created);
    Ok(())
}

fn read_project_session_result(
    reader: daemon_capnp::project_session_result::Reader<'_>,
) -> Result<ProjectSessionResult, Error> {
    Ok(ProjectSessionResult {
        manifest: session::SessionManifest::from_capnp(reader.get_manifest()?)?,
        created: reader.get_created(),
    })
}

fn write_prepare_project_session_services_request(
    request: &PrepareProjectSessionServicesRequest,
    mut builder: daemon_capnp::prepare_project_session_services_request::Builder<'_>,
) -> Result<(), Error> {
    validate_prepare_project_session_services_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_slug(&request.session_slug);
    builder.set_endpoint_kind(request.endpoint_kind.to_capnp());
    builder.set_skip_build(request.skip_build);
    let mut service_names = builder
        .reborrow()
        .init_service_names(capnp_list_index(request.service_names.len())?);
    for (index, service_name) in request.service_names.iter().enumerate() {
        service_names.set(capnp_list_index(index)?, service_name);
    }
    builder.set_otlp_endpoint(request.otlp_endpoint.as_deref().unwrap_or(""));
    builder.set_recover(request.recover);
    Ok(())
}

fn read_prepare_project_session_services_request(
    reader: daemon_capnp::prepare_project_session_services_request::Reader<'_>,
) -> Result<PrepareProjectSessionServicesRequest, Error> {
    let request = PrepareProjectSessionServicesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
        endpoint_kind: core::EndpointKind::from_capnp(reader.get_endpoint_kind()?),
        skip_build: reader.get_skip_build(),
        service_names: reader
            .get_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        otlp_endpoint: {
            let value = reader.get_otlp_endpoint()?.to_string()?;
            (!value.is_empty()).then_some(value)
        },
        recover: reader.get_recover(),
    };
    validate_prepare_project_session_services_request_for_transport(&request)?;
    Ok(request)
}

fn write_project_session_services_result(
    result: &ProjectSessionServicesResult,
    mut builder: daemon_capnp::project_session_services_result::Builder<'_>,
) -> Result<(), Error> {
    (result.manifest).to_capnp(builder.reborrow().init_manifest())?;
    let mut services = builder
        .reborrow()
        .init_service_names(capnp_list_index(result.service_names.len())?);
    for (index, service_name) in result.service_names.iter().enumerate() {
        services.set(capnp_list_index(index)?, service_name);
    }
    builder.set_build_command_count(result.build_command_count);
    builder.set_prepared_process_count(result.prepared_process_count);
    builder.set_built(result.built);
    Ok(())
}

fn read_project_session_services_result(
    reader: daemon_capnp::project_session_services_result::Reader<'_>,
) -> Result<ProjectSessionServicesResult, Error> {
    Ok(ProjectSessionServicesResult {
        manifest: session::SessionManifest::from_capnp(reader.get_manifest()?)?,
        service_names: reader
            .get_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        build_command_count: reader.get_build_command_count(),
        prepared_process_count: reader.get_prepared_process_count(),
        built: reader.get_built(),
    })
}

fn write_ensure_project_session_services_request(
    request: &EnsureProjectSessionServicesRequest,
    mut builder: daemon_capnp::ensure_project_session_services_request::Builder<'_>,
) -> Result<(), Error> {
    validate_ensure_project_session_services_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_name(&request.session_name);
    builder.set_endpoint_kind(request.endpoint_kind.to_capnp());
    builder.set_skip_build(request.skip_build);
    let mut service_names = builder
        .reborrow()
        .init_start_service_names(capnp_list_index(request.start_service_names.len())?);
    for (index, service_name) in request.start_service_names.iter().enumerate() {
        service_names.set(capnp_list_index(index)?, service_name);
    }
    builder.set_timeout_ms(request.timeout_ms);
    core::Endpoint::to_capnp(
        &request.daemon_endpoint,
        builder.reborrow().init_daemon_endpoint(),
    )?;
    Ok(())
}

fn read_ensure_project_session_services_request(
    reader: daemon_capnp::ensure_project_session_services_request::Reader<'_>,
) -> Result<EnsureProjectSessionServicesRequest, Error> {
    let request = EnsureProjectSessionServicesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_name: reader.get_session_name()?.to_string()?,
        endpoint_kind: core::EndpointKind::from_capnp(reader.get_endpoint_kind()?),
        skip_build: reader.get_skip_build(),
        start_service_names: reader
            .get_start_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        timeout_ms: reader.get_timeout_ms(),
        daemon_endpoint: core::Endpoint::from_capnp(reader.get_daemon_endpoint()?)?,
    };
    validate_ensure_project_session_services_request_for_transport(&request)?;
    Ok(request)
}

fn write_project_open_progress_event(
    event: &ProjectOpenProgressEvent,
    mut builder: daemon_capnp::project_open_progress_event::Builder<'_>,
) {
    builder.set_seq(event.seq);
    builder.set_phase(event.phase.to_capnp());
    builder.set_done_bp(event.done_bp);
    builder.set_phase_done(event.phase_done);
    builder.set_phase_total(event.phase_total);
    builder.set_message(&event.message);
}

fn read_project_open_progress_event(
    reader: daemon_capnp::project_open_progress_event::Reader<'_>,
) -> Result<ProjectOpenProgressEvent, Error> {
    Ok(ProjectOpenProgressEvent {
        seq: reader.get_seq(),
        phase: OpenProjectPhase::from_capnp(reader.get_phase()?),
        done_bp: reader.get_done_bp(),
        phase_done: reader.get_phase_done(),
        phase_total: reader.get_phase_total(),
        message: reader.get_message()?.to_string()?,
    })
}

fn write_project_session_services_start_result(
    result: &ProjectSessionServicesStartResult,
    mut builder: daemon_capnp::project_session_services_start_result::Builder<'_>,
) -> Result<(), Error> {
    (result.manifest).to_capnp(builder.reborrow().init_manifest())?;
    let mut services = builder
        .reborrow()
        .init_service_names(capnp_list_index(result.service_names.len())?);
    for (index, service_name) in result.service_names.iter().enumerate() {
        services.set(capnp_list_index(index)?, service_name);
    }
    let mut running = builder
        .reborrow()
        .init_running_service_names(capnp_list_index(result.running_service_names.len())?);
    for (index, service_name) in result.running_service_names.iter().enumerate() {
        running.set(capnp_list_index(index)?, service_name);
    }
    builder.set_build_command_count(result.build_command_count);
    builder.set_prepared_process_count(result.prepared_process_count);
    builder.set_built(result.built);
    builder.set_session_created(result.session_created);
    core::ServiceDescriptor::to_capnp(&result.supervisor, builder.reborrow().init_supervisor())?;
    builder.set_reused_supervisor(result.reused_supervisor);
    builder.set_start_requested(result.start_requested);
    builder.set_sessiond_pid(result.sessiond_pid);
    builder.set_log_path(&result.log_path);
    Ok(())
}

fn read_project_session_services_start_result(
    reader: daemon_capnp::project_session_services_start_result::Reader<'_>,
) -> Result<ProjectSessionServicesStartResult, Error> {
    Ok(ProjectSessionServicesStartResult {
        manifest: session::SessionManifest::from_capnp(reader.get_manifest()?)?,
        service_names: reader
            .get_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        running_service_names: reader
            .get_running_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        build_command_count: reader.get_build_command_count(),
        prepared_process_count: reader.get_prepared_process_count(),
        built: reader.get_built(),
        session_created: reader.get_session_created(),
        supervisor: core::ServiceDescriptor::from_capnp(reader.get_supervisor()?)?,
        reused_supervisor: reader.get_reused_supervisor(),
        start_requested: reader.get_start_requested(),
        sessiond_pid: reader.get_sessiond_pid(),
        log_path: reader.get_log_path()?.to_string()?,
    })
}

fn write_plan_project_build_request(
    request: &PlanProjectBuildRequest,
    mut builder: daemon_capnp::plan_project_build_request::Builder<'_>,
) -> Result<(), Error> {
    validate_plan_project_build_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_profile(&request.profile);
    builder.set_target_triple(request.target_triple.as_deref().unwrap_or(""));
    let mut selectors = builder
        .reborrow()
        .init_package_selectors(capnp_list_index(request.package_selectors.len())?);
    for (index, selector) in request.package_selectors.iter().enumerate() {
        selectors.set(capnp_list_index(index)?, selector);
    }
    Ok(())
}

fn read_plan_project_build_request(
    reader: daemon_capnp::plan_project_build_request::Reader<'_>,
) -> Result<PlanProjectBuildRequest, Error> {
    let target_triple = reader.get_target_triple()?.to_string()?;
    let request = PlanProjectBuildRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        profile: reader.get_profile()?.to_string()?,
        target_triple: if target_triple.is_empty() {
            None
        } else {
            Some(target_triple)
        },
        package_selectors: reader
            .get_package_selectors()?
            .iter()
            .map(|selector| Ok(selector?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_plan_project_build_request_for_transport(&request)?;
    Ok(request)
}

fn write_execute_project_build_request(
    request: &ExecuteProjectBuildRequest,
    mut builder: daemon_capnp::execute_project_build_request::Builder<'_>,
) -> Result<(), Error> {
    validate_execute_project_build_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_profile(&request.profile);
    builder.set_target_triple(request.target_triple.as_deref().unwrap_or(""));
    let mut selectors = builder
        .reborrow()
        .init_package_selectors(capnp_list_index(request.package_selectors.len())?);
    for (index, selector) in request.package_selectors.iter().enumerate() {
        selectors.set(capnp_list_index(index)?, selector);
    }
    Ok(())
}

fn read_execute_project_build_request(
    reader: daemon_capnp::execute_project_build_request::Reader<'_>,
) -> Result<ExecuteProjectBuildRequest, Error> {
    let target_triple = reader.get_target_triple()?.to_string()?;
    let request = ExecuteProjectBuildRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        profile: reader.get_profile()?.to_string()?,
        target_triple: if target_triple.is_empty() {
            None
        } else {
            Some(target_triple)
        },
        package_selectors: reader
            .get_package_selectors()?
            .iter()
            .map(|selector| Ok(selector?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_execute_project_build_request_for_transport(&request)?;
    Ok(request)
}

fn write_project_build_command(
    command: &ProjectBuildCommand,
    mut builder: daemon_capnp::project_build_command::Builder<'_>,
) -> Result<(), Error> {
    validate_project_build_command_for_transport(command)?;
    builder.set_owner_id(&command.owner_id);
    builder.set_owner_root(&command.owner_root);
    builder.set_target_name(&command.target_name);
    builder.set_program(&command.program);
    builder.set_cwd(&command.cwd);
    let mut args = builder
        .reborrow()
        .init_args(capnp_list_index(command.args.len())?);
    for (index, arg) in command.args.iter().enumerate() {
        args.set(capnp_list_index(index)?, arg);
    }
    write_optional_text(
        command.cargo_target_dir.as_deref(),
        builder.reborrow().init_cargo_target_dir(),
    );
    Ok(())
}

fn read_project_build_command(
    reader: daemon_capnp::project_build_command::Reader<'_>,
) -> Result<ProjectBuildCommand, Error> {
    let args = reader
        .get_args()?
        .iter()
        .map(|arg| Ok(arg?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;
    let command = ProjectBuildCommand {
        owner_id: reader.get_owner_id()?.to_string()?,
        owner_root: reader.get_owner_root()?.to_string()?,
        target_name: reader.get_target_name()?.to_string()?,
        program: reader.get_program()?.to_string()?,
        cwd: reader.get_cwd()?.to_string()?,
        args,
        cargo_target_dir: read_optional_text(reader.get_cargo_target_dir()?)?,
    };
    validate_project_build_command_for_transport(&command)?;
    Ok(command)
}

fn write_optional_project_build_command(
    command: Option<&ProjectBuildCommand>,
    mut builder: daemon_capnp::optional_project_build_command::Builder<'_>,
) -> Result<(), Error> {
    if let Some(command) = command {
        (command).to_capnp(builder.init_value())
    } else {
        builder.set_none(());
        Ok(())
    }
}

fn read_optional_project_build_command(
    reader: daemon_capnp::optional_project_build_command::Reader<'_>,
) -> Result<Option<ProjectBuildCommand>, Error> {
    match reader.which()? {
        daemon_capnp::optional_project_build_command::Which::None(()) => Ok(None),
        daemon_capnp::optional_project_build_command::Which::Value(value) => {
            Ok(Some(ProjectBuildCommand::from_capnp(value?)?))
        }
    }
}

fn write_project_build_package_profile(
    profile: &ProjectBuildPackageProfile,
    mut builder: daemon_capnp::project_build_package_profile::Builder<'_>,
) -> Result<(), Error> {
    validate_project_build_package_profile_for_transport(profile)?;
    builder.set_name(&profile.name);
    builder.set_asset_platform(&profile.asset_platform);
    builder.set_cargo_profile(&profile.cargo_profile);
    builder.set_container(&profile.container);
    builder.set_compression(&profile.compression);
    write_optional_text(
        profile.oodle_compressor.as_deref(),
        builder.reborrow().init_oodle_compressor(),
    );
    write_optional_text(
        profile.oodle_effort.as_deref(),
        builder.reborrow().init_oodle_effort(),
    );
    Ok(())
}

fn read_project_build_package_profile(
    reader: daemon_capnp::project_build_package_profile::Reader<'_>,
) -> Result<ProjectBuildPackageProfile, Error> {
    let profile = ProjectBuildPackageProfile {
        name: reader.get_name()?.to_string()?,
        asset_platform: reader.get_asset_platform()?.to_string()?,
        cargo_profile: reader.get_cargo_profile()?.to_string()?,
        container: reader.get_container()?.to_string()?,
        compression: reader.get_compression()?.to_string()?,
        oodle_compressor: read_optional_text(reader.get_oodle_compressor()?)?,
        oodle_effort: read_optional_text(reader.get_oodle_effort()?)?,
    };
    validate_project_build_package_profile_for_transport(&profile)?;
    Ok(profile)
}

fn write_optional_project_build_package_profile(
    profile: Option<&ProjectBuildPackageProfile>,
    mut builder: daemon_capnp::optional_project_build_package_profile::Builder<'_>,
) -> Result<(), Error> {
    if let Some(profile) = profile {
        (profile).to_capnp(builder.init_value())
    } else {
        builder.set_none(());
        Ok(())
    }
}

fn read_optional_project_build_package_profile(
    reader: daemon_capnp::optional_project_build_package_profile::Reader<'_>,
) -> Result<Option<ProjectBuildPackageProfile>, Error> {
    match reader.which()? {
        daemon_capnp::optional_project_build_package_profile::Which::None(()) => Ok(None),
        daemon_capnp::optional_project_build_package_profile::Which::Value(value) => {
            Ok(Some(ProjectBuildPackageProfile::from_capnp(value?)?))
        }
    }
}

fn write_optional_text(
    value: Option<&str>,
    mut builder: core::core_capnp::optional_text::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_text(
    reader: core::core_capnp::optional_text::Reader<'_>,
) -> Result<Option<String>, Error> {
    match reader.which()? {
        core::core_capnp::optional_text::Which::None(()) => Ok(None),
        core::core_capnp::optional_text::Which::Value(value) => Ok(Some(value?.to_string()?)),
    }
}

fn write_project_build_plan(
    plan: &ProjectBuildPlan,
    mut builder: daemon_capnp::project_build_plan::Builder<'_>,
) -> Result<(), Error> {
    validate_project_build_plan_for_transport(plan)?;
    let mut commands = builder
        .reborrow()
        .init_commands(capnp_list_index(plan.commands.len())?);
    for (index, command) in plan.commands.iter().enumerate() {
        (command).to_capnp(commands.reborrow().get(capnp_list_index(index)?))?;
    }
    write_optional_project_build_package_profile(
        plan.package_profile.as_ref(),
        builder.init_package_profile(),
    )?;
    Ok(())
}

fn read_project_build_plan(
    reader: daemon_capnp::project_build_plan::Reader<'_>,
) -> Result<ProjectBuildPlan, Error> {
    let commands = reader
        .get_commands()?
        .iter()
        .map(read_project_build_command)
        .collect::<Result<Vec<_>, Error>>()?;
    let plan = ProjectBuildPlan {
        commands,
        package_profile: read_optional_project_build_package_profile(
            reader.get_package_profile()?,
        )?,
    };
    validate_project_build_plan_for_transport(&plan)?;
    Ok(plan)
}

fn write_project_build_execution_result(
    result: &ProjectBuildExecutionResult,
    mut builder: daemon_capnp::project_build_execution_result::Builder<'_>,
) -> Result<(), Error> {
    validate_project_build_execution_result_for_transport(result)?;
    builder.set_success(result.success);
    (result.plan).to_capnp(builder.reborrow().init_plan())?;
    builder.set_completed_command_count(result.completed_command_count);
    write_optional_project_build_command(
        result.failing_command.as_ref(),
        builder.reborrow().init_failing_command(),
    )?;
    builder.set_diagnostic_headline(&result.diagnostic_headline);
    builder.set_diagnostic_tail(&result.diagnostic_tail);
    Ok(())
}

fn read_project_build_execution_result(
    reader: daemon_capnp::project_build_execution_result::Reader<'_>,
) -> Result<ProjectBuildExecutionResult, Error> {
    let result = ProjectBuildExecutionResult {
        success: reader.get_success(),
        plan: ProjectBuildPlan::from_capnp(reader.get_plan()?)?,
        completed_command_count: reader.get_completed_command_count(),
        failing_command: read_optional_project_build_command(reader.get_failing_command()?)?,
        diagnostic_headline: reader.get_diagnostic_headline()?.to_string()?,
        diagnostic_tail: reader.get_diagnostic_tail()?.to_string()?,
    };
    validate_project_build_execution_result_for_transport(&result)?;
    Ok(result)
}

fn write_project_build_progress_event(
    event: &ProjectBuildProgressEvent,
    mut builder: daemon_capnp::project_build_progress_event::Builder<'_>,
) -> Result<(), Error> {
    validate_project_build_progress_event_for_transport(event)?;
    builder.set_seq(event.seq);
    builder.set_command_index(event.command_index);
    builder.set_command_count(event.command_count);
    builder.set_target_name(&event.target_name);
    builder.set_done_bp(event.done_bp);
    builder.set_command_done(event.command_done);
    builder.set_command_total(event.command_total);
    builder.set_message(&event.message);
    Ok(())
}

fn read_project_build_progress_event(
    reader: daemon_capnp::project_build_progress_event::Reader<'_>,
) -> Result<ProjectBuildProgressEvent, Error> {
    let event = ProjectBuildProgressEvent {
        seq: reader.get_seq(),
        command_index: reader.get_command_index(),
        command_count: reader.get_command_count(),
        target_name: reader.get_target_name()?.to_string()?,
        done_bp: reader.get_done_bp(),
        command_done: reader.get_command_done(),
        command_total: reader.get_command_total(),
        message: reader.get_message()?.to_string()?,
    };
    validate_project_build_progress_event_for_transport(&event)?;
    Ok(event)
}

fn write_plan_project_services_request(
    request: &PlanProjectServicesRequest,
    mut builder: daemon_capnp::plan_project_services_request::Builder<'_>,
) -> Result<(), Error> {
    validate_plan_project_services_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_project_id(&request.project_id);
    builder.set_session_slug(&request.session_slug);
    builder.set_endpoint_kind(request.endpoint_kind.to_capnp());
    if let Some(workspace_root) = request.workspace_root.as_deref() {
        builder.set_workspace_root(workspace_root);
    }
    let mut service_names = builder
        .reborrow()
        .init_service_names(capnp_list_index(request.service_names.len())?);
    for (index, service_name) in request.service_names.iter().enumerate() {
        service_names.set(capnp_list_index(index)?, service_name);
    }
    Ok(())
}

fn read_plan_project_services_request(
    reader: daemon_capnp::plan_project_services_request::Reader<'_>,
) -> Result<PlanProjectServicesRequest, Error> {
    let workspace_root = reader.get_workspace_root()?.to_string()?;
    let workspace_root = if workspace_root.trim().is_empty() {
        None
    } else {
        Some(workspace_root)
    };
    let request = PlanProjectServicesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        project_id: reader.get_project_id()?.to_string()?,
        session_slug: reader.get_session_slug()?.to_string()?,
        endpoint_kind: core::EndpointKind::from_capnp(reader.get_endpoint_kind()?),
        workspace_root,
        service_names: reader
            .get_service_names()?
            .iter()
            .map(|name| Ok(name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_plan_project_services_request_for_transport(&request)?;
    Ok(request)
}

fn write_project_service_command(
    command: &ProjectServiceCommand,
    mut builder: daemon_capnp::project_service_command::Builder<'_>,
) -> Result<(), Error> {
    validate_project_service_command_for_transport(command)?;
    builder.set_owner_id(&command.owner_id);
    builder.set_owner_root(&command.owner_root);
    builder.set_build_output_root(&command.build_output_root);
    builder.set_service_name(&command.service_name);
    builder.set_role(command.role.to_capnp());
    core::Endpoint::to_capnp(&command.endpoint, builder.reborrow().init_endpoint())?;
    builder.set_program(&command.program);
    builder.set_cwd(&command.cwd);
    let mut args = builder
        .reborrow()
        .init_args(capnp_list_index(command.args.len())?);
    for (index, arg) in command.args.iter().enumerate() {
        args.set(capnp_list_index(index)?, arg);
    }
    Ok(())
}

fn read_project_service_command(
    reader: daemon_capnp::project_service_command::Reader<'_>,
) -> Result<ProjectServiceCommand, Error> {
    let args = reader
        .get_args()?
        .iter()
        .map(|arg| Ok(arg?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;
    let command = ProjectServiceCommand {
        owner_id: reader.get_owner_id()?.to_string()?,
        owner_root: reader.get_owner_root()?.to_string()?,
        build_output_root: reader.get_build_output_root()?.to_string()?,
        service_name: reader.get_service_name()?.to_string()?,
        role: core::ServiceRole::from_capnp(reader.get_role()?),
        endpoint: core::Endpoint::from_capnp(reader.get_endpoint()?)?,
        program: reader.get_program()?.to_string()?,
        cwd: reader.get_cwd()?.to_string()?,
        args,
    };
    validate_project_service_command_for_transport(&command)?;
    Ok(command)
}

fn write_project_service_plan(
    plan: &ProjectServicePlan,
    mut builder: daemon_capnp::project_service_plan::Builder<'_>,
) -> Result<(), Error> {
    validate_project_service_plan_for_transport(plan)?;
    let mut commands = builder
        .reborrow()
        .init_commands(capnp_list_index(plan.commands.len())?);
    for (index, command) in plan.commands.iter().enumerate() {
        (command).to_capnp(commands.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut build_commands = builder
        .reborrow()
        .init_build_commands(capnp_list_index(plan.build_commands.len())?);
    for (index, command) in plan.build_commands.iter().enumerate() {
        (command).to_capnp(build_commands.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_project_service_plan(
    reader: daemon_capnp::project_service_plan::Reader<'_>,
) -> Result<ProjectServicePlan, Error> {
    let commands = reader
        .get_commands()?
        .iter()
        .map(read_project_service_command)
        .collect::<Result<Vec<_>, Error>>()?;
    let build_commands = reader
        .get_build_commands()?
        .iter()
        .map(read_project_build_command)
        .collect::<Result<Vec<_>, Error>>()?;
    let plan = ProjectServicePlan {
        build_commands,
        commands,
    };
    validate_project_service_plan_for_transport(&plan)?;
    Ok(plan)
}

fn write_shutdown_daemon_request(
    request: &ShutdownDaemonRequest,
    mut builder: daemon_capnp::shutdown_daemon_request::Builder<'_>,
) -> Result<(), Error> {
    validate_shutdown_daemon_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_reason(&request.reason);
    Ok(())
}

fn read_shutdown_daemon_request(
    reader: daemon_capnp::shutdown_daemon_request::Reader<'_>,
) -> Result<ShutdownDaemonRequest, Error> {
    let request = ShutdownDaemonRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        reason: reader.get_reason()?.to_string()?,
    };
    validate_shutdown_daemon_request_for_transport(&request)?;
    Ok(request)
}

fn write_shutdown_daemon_result(
    result: &ShutdownDaemonResult,
    mut builder: daemon_capnp::shutdown_daemon_result::Builder<'_>,
) {
    builder.set_accepted(result.accepted);
    builder.set_reason(&result.reason);
}

fn read_shutdown_daemon_result(
    reader: daemon_capnp::shutdown_daemon_result::Reader<'_>,
) -> Result<ShutdownDaemonResult, Error> {
    Ok(ShutdownDaemonResult {
        accepted: reader.get_accepted(),
        reason: reader.get_reason()?.to_string()?,
    })
}

fn write_touch_editor_lease_request(
    request: &TouchEditorLeaseRequest,
    mut builder: daemon_capnp::touch_editor_lease_request::Builder<'_>,
) -> Result<(), Error> {
    validate_touch_editor_lease_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_lease_id(&request.lease_id);
    let mut owner_process = builder.reborrow().init_owner_process();
    owner_process.set_process_id(request.owner_process.process_id);
    owner_process.set_process_start_time(request.owner_process.process_start_time);
    builder.set_purpose(&request.purpose);
    Ok(())
}

fn read_touch_editor_lease_request(
    reader: daemon_capnp::touch_editor_lease_request::Reader<'_>,
) -> Result<TouchEditorLeaseRequest, Error> {
    let owner_process = reader.get_owner_process()?;
    let request = TouchEditorLeaseRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        lease_id: reader.get_lease_id()?.to_string()?,
        owner_process: ProcessIdentity {
            process_id: owner_process.get_process_id(),
            process_start_time: owner_process.get_process_start_time(),
        },
        purpose: reader.get_purpose()?.to_string()?,
    };
    validate_touch_editor_lease_request_for_transport(&request)?;
    Ok(request)
}

fn write_touch_editor_lease_result(
    result: &TouchEditorLeaseResult,
    mut builder: daemon_capnp::touch_editor_lease_result::Builder<'_>,
) -> Result<(), Error> {
    validate_touch_editor_lease_result_for_transport(result)?;
    builder.set_accepted(result.accepted);
    builder.set_lease_id(&result.lease_id);
    builder.set_active_lease_count(result.active_lease_count);
    Ok(())
}

fn read_touch_editor_lease_result(
    reader: daemon_capnp::touch_editor_lease_result::Reader<'_>,
) -> Result<TouchEditorLeaseResult, Error> {
    let result = TouchEditorLeaseResult {
        accepted: reader.get_accepted(),
        lease_id: reader.get_lease_id()?.to_string()?,
        active_lease_count: reader.get_active_lease_count(),
    };
    validate_touch_editor_lease_result_for_transport(&result)?;
    Ok(result)
}

impl ProjectRecord {
    pub fn to_capnp(&self, builder: daemon_capnp::project_record::Builder<'_>) {
        write_project_record(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project record is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: daemon_capnp::project_record::Reader<'_>) -> Result<Self, Error> {
        read_project_record(reader)
    }
}

impl ProjectResult {
    pub fn to_capnp(&self, builder: daemon_capnp::project_result::Builder<'_>) {
        write_project_result(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project result is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: daemon_capnp::project_result::Reader<'_>) -> Result<Self, Error> {
        read_project_result(reader)
    }
}

impl RegisterProjectRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the register project
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::register_project_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_register_project_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the register project request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::register_project_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_register_project_request(reader)
    }
}

impl RegisterProjectRootRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the register project root
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::register_project_root_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_register_project_root_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the register project root request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::register_project_root_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_register_project_root_request(reader)
    }
}

impl ResolveProjectRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the resolve project request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::resolve_project_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_resolve_project_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the resolve project request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::resolve_project_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_resolve_project_request(reader)
    }
}

impl ListProjectsRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the list projects request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::list_projects_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_list_projects_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the list projects request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::list_projects_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_projects_request(reader)
    }
}

impl ListProjectsResult {
    /// # Panics
    ///
    /// Panics if the project list is longer than a Cap'n Proto list can address
    /// (`u32::MAX` entries). This method is infallible in the RPC surface that
    /// calls it, and the list comes from the machine-local project registry, so
    /// the bound is not reachable in practice. The conversion is still checked
    /// rather than a truncating cast, so an oversized list fails loudly instead
    /// of silently encoding the wrong element count.
    pub fn to_capnp(&self, builder: daemon_capnp::list_projects_result::Builder<'_>) {
        write_list_projects_result(self, builder)
            .expect("machine-local project registry fits in a Cap'n Proto list");
    }

    /// # Errors
    ///
    /// Returns an error if a field of the list projects result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::list_projects_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_projects_result(reader)
    }
}

impl RegisterSessionSupervisorRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the register session
    /// supervisor request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::register_session_supervisor_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_register_session_supervisor_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the register session supervisor request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::register_session_supervisor_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_register_session_supervisor_request(reader)
    }
}

impl UnregisterSessionSupervisorRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the unregister session
    /// supervisor request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::unregister_session_supervisor_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_unregister_session_supervisor_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the unregister session supervisor request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::unregister_session_supervisor_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_unregister_session_supervisor_request(reader)
    }
}

impl UnregisterSessionSupervisorResult {
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::unregister_session_supervisor_result::Builder<'_>,
    ) {
        write_unregister_session_supervisor_result(self, builder);
    }

    /// # Errors
    ///
    /// Never returns an error today: the result carries a single `Bool` that
    /// decodes without a fallible read. The `Result` is kept for signature
    /// parity with the other wire types.
    pub fn from_capnp(
        reader: daemon_capnp::unregister_session_supervisor_result::Reader<'_>,
    ) -> Result<Self, Error> {
        Ok(read_unregister_session_supervisor_result(reader))
    }
}

impl ResolveSessionSupervisorRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the resolve session
    /// supervisor request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::resolve_session_supervisor_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_resolve_session_supervisor_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the resolve session supervisor request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::resolve_session_supervisor_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_resolve_session_supervisor_request(reader)
    }
}

impl ListSessionSupervisorsRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the list session supervisors
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::list_session_supervisors_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_list_session_supervisors_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the list session supervisors request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::list_session_supervisors_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_session_supervisors_request(reader)
    }
}

impl SessionSupervisorDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::session_supervisor_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor descriptor is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::session_supervisor_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_descriptor(reader)
    }
}

impl ListSessionSupervisorsResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the list session supervisors
    /// result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::list_session_supervisors_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_list_session_supervisors_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the list session supervisors result is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::list_session_supervisors_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_session_supervisors_result(reader)
    }
}

impl SessionSupervisorResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor
    /// result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::session_supervisor_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::session_supervisor_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_result(reader)
    }
}

impl EnsureProjectSessionRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the ensure project session
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::ensure_project_session_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_ensure_project_session_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the ensure project session request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::ensure_project_session_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_ensure_project_session_request(reader)
    }
}

impl ProjectSessionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project session result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_session_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_session_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project session result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_session_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_session_result(reader)
    }
}

impl PrepareProjectSessionServicesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the prepare project session
    /// services request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::prepare_project_session_services_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_prepare_project_session_services_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the prepare project session services request is absent from
    /// the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::prepare_project_session_services_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_prepare_project_session_services_request(reader)
    }
}

impl ProjectSessionServicesResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project session services
    /// result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_session_services_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_session_services_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project session services result is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_session_services_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_session_services_result(reader)
    }
}

impl EnsureProjectSessionServicesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the ensure project session
    /// services request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::ensure_project_session_services_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_ensure_project_session_services_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the ensure project session services request is absent from
    /// the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::ensure_project_session_services_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_ensure_project_session_services_request(reader)
    }
}

impl ProjectOpenProgressEvent {
    pub fn to_capnp(&self, builder: daemon_capnp::project_open_progress_event::Builder<'_>) {
        write_project_open_progress_event(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project open progress event is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_open_progress_event::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_open_progress_event(reader)
    }
}

impl ProjectSessionServicesStartResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project session services
    /// start result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_session_services_start_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_session_services_start_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project session services start result is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_session_services_start_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_session_services_start_result(reader)
    }
}

impl PlanProjectBuildRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the plan project build
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::plan_project_build_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_plan_project_build_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the plan project build request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::plan_project_build_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_plan_project_build_request(reader)
    }
}

impl ExecuteProjectBuildRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the execute project build
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::execute_project_build_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_execute_project_build_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the execute project build request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::execute_project_build_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_execute_project_build_request(reader)
    }
}

impl ProjectBuildCommand {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project build command.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_build_command::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_build_command(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project build command is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_build_command::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_build_command(reader)
    }
}

impl ProjectBuildPackageProfile {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project build package
    /// profile.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_build_package_profile::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_build_package_profile(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project build package profile is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_build_package_profile::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_build_package_profile(reader)
    }
}

impl ProjectBuildPlan {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project build plan.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_build_plan::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_build_plan(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project build plan is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: daemon_capnp::project_build_plan::Reader<'_>) -> Result<Self, Error> {
        read_project_build_plan(reader)
    }
}

impl ProjectBuildExecutionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project build execution
    /// result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_build_execution_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_build_execution_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project build execution result is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_build_execution_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_build_execution_result(reader)
    }
}

impl ProjectBuildProgressEvent {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project build progress
    /// event.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_build_progress_event::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_build_progress_event(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project build progress event is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_build_progress_event::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_build_progress_event(reader)
    }
}

impl PlanProjectServicesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the plan project services
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::plan_project_services_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_plan_project_services_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the plan project services request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::plan_project_services_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_plan_project_services_request(reader)
    }
}

impl ProjectServiceCommand {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project service command.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_service_command::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_service_command(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project service command is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_service_command::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_service_command(reader)
    }
}

impl ProjectServicePlan {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the project service plan.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::project_service_plan::Builder<'_>,
    ) -> Result<(), Error> {
        write_project_service_plan(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the project service plan is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::project_service_plan::Reader<'_>,
    ) -> Result<Self, Error> {
        read_project_service_plan(reader)
    }
}

impl ShutdownDaemonRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the shutdown daemon request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::shutdown_daemon_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_shutdown_daemon_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the shutdown daemon request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::shutdown_daemon_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_shutdown_daemon_request(reader)
    }
}

impl ShutdownDaemonResult {
    pub fn to_capnp(&self, builder: daemon_capnp::shutdown_daemon_result::Builder<'_>) {
        write_shutdown_daemon_result(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the shutdown daemon result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::shutdown_daemon_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_shutdown_daemon_result(reader)
    }
}

impl TouchEditorLeaseRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the touch editor lease
    /// request.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::touch_editor_lease_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_touch_editor_lease_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the touch editor lease request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::touch_editor_lease_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_touch_editor_lease_request(reader)
    }
}

impl TouchEditorLeaseResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the touch editor lease
    /// result.
    pub fn to_capnp(
        &self,
        builder: daemon_capnp::touch_editor_lease_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_touch_editor_lease_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the touch editor lease result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: daemon_capnp::touch_editor_lease_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_touch_editor_lease_result(reader)
    }
}

#[cfg(test)]
mod tests {
    use capnp::message;

    use super::*;

    fn capability(permission: &'static str) -> core::Capability {
        core::Capability::new(
            core::ServiceId::new("azoth", "editor"),
            core::ServiceRole::Editor,
        )
        .with_audience(DAEMON_AUDIENCE)
        .with_permissions([permission])
    }

    fn brokered_capability() -> core::Capability {
        capability(DAEMON_READ_PERMISSION).with_token_hash(vec![0x42; 32])
    }

    fn session_supervisor_descriptor() -> core::ServiceDescriptor {
        core::ServiceDescriptor::new(
            core::ServiceId::new("azoth", "session-supervisor"),
            core::ServiceRole::SessionSupervisor,
            core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37613"),
        )
        .with_run(uuid::Uuid::now_v7())
        .with_capability(brokered_capability())
    }

    fn project() -> ProjectRecord {
        ProjectRecord {
            project_id: "local.example".to_string(),
            name: "Example".to_string(),
            root: "projects/example".to_string(),
            manifest_path: "projects/example/azoth.toml".to_string(),
            engine_version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn project_registry_round_trips_project_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.projects.toml");
        let project = project();

        write_project_registry(&path, std::slice::from_ref(&project)).unwrap();

        assert_eq!(read_project_registry(&path).unwrap(), vec![project]);
    }

    #[test]
    fn project_registry_forget_is_idempotent_and_preserves_other_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.projects.toml");
        let removed = project();
        let mut retained = project();
        retained.project_id = "local.retained".to_string();
        retained.name = "Retained".to_string();
        write_project_registry(&path, &[removed.clone(), retained.clone()]).unwrap();

        assert_eq!(
            forget_project_registration(&path, &removed.project_id).unwrap(),
            Some(removed.clone())
        );
        assert_eq!(read_project_registry(&path).unwrap(), vec![retained]);
        assert_eq!(
            forget_project_registration(&path, &removed.project_id).unwrap(),
            None
        );
    }

    #[test]
    fn project_open_progress_event_round_trips() {
        for expected in [
            ProjectOpenProgressEvent {
                seq: 7,
                phase: OpenProjectPhase::Build,
                done_bp: 4_200,
                phase_done: 12,
                phase_total: 30,
                message: "compiling az-runtime-host".to_string(),
            },
            ProjectOpenProgressEvent {
                seq: 1,
                phase: OpenProjectPhase::StartServices,
                done_bp: 9_200,
                phase_done: 2,
                phase_total: 0, // unknown total
                message: String::new(),
            },
            ProjectOpenProgressEvent {
                seq: 99,
                phase: OpenProjectPhase::LoadCatalogs,
                done_bp: 10_000,
                phase_done: 3,
                phase_total: 3,
                message: "catalogs loaded".to_string(),
            },
        ] {
            let mut message = message::Builder::new_default();
            (expected).to_capnp(
                message.init_root::<daemon_capnp::project_open_progress_event::Builder<'_>>(),
            );
            let reader = message
                .get_root_as_reader::<daemon_capnp::project_open_progress_event::Reader<'_>>()
                .unwrap();
            assert_eq!(
                ProjectOpenProgressEvent::from_capnp(reader).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn project_result_round_trips_optional_record() {
        let expected = ProjectResult {
            project: Some(project()),
        };
        let mut message = message::Builder::new_default();
        (expected).to_capnp(message.init_root::<daemon_capnp::project_result::Builder<'_>>());

        let reader = message
            .get_root_as_reader::<daemon_capnp::project_result::Reader<'_>>()
            .unwrap();
        assert_eq!(ProjectResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn list_projects_result_round_trips_records() {
        let expected = ListProjectsResult {
            projects: vec![project()],
            protocol_version: core::ProtocolVersion::CURRENT,
        };
        let mut message = message::Builder::new_default();
        (expected).to_capnp(message.init_root::<daemon_capnp::list_projects_result::Builder<'_>>());

        let reader = message
            .get_root_as_reader::<daemon_capnp::list_projects_result::Reader<'_>>()
            .unwrap();
        assert_eq!(ListProjectsResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn legacy_list_projects_result_decodes_as_outdated_protocol() {
        let mut message = message::Builder::new_default();
        message
            .init_root::<daemon_capnp::list_projects_result::Builder<'_>>()
            .init_projects(0);

        let reader = message
            .get_root_as_reader::<daemon_capnp::list_projects_result::Reader<'_>>()
            .unwrap();
        let result = ListProjectsResult::from_capnp(reader).unwrap();

        assert!(
            result
                .protocol_version
                .require(core::ProtocolVersion::CURRENT)
                .is_err()
        );
    }

    #[test]
    fn register_project_root_request_round_trips_root() {
        let expected = RegisterProjectRootRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            root: "projects/example".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<daemon_capnp::register_project_root_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::register_project_root_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            RegisterProjectRootRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn register_project_root_request_rejects_read_capability() {
        let request = RegisterProjectRootRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            root: "projects/example".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<daemon_capnp::register_project_root_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(DAEMON_PROJECTS_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn list_projects_request_rejects_wrong_audience() {
        let request = ListProjectsRequest {
            capability: capability(DAEMON_READ_PERMISSION).with_audience("project-host"),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<daemon_capnp::list_projects_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains(DAEMON_AUDIENCE), "{error}");
    }

    #[test]
    fn resolve_project_request_accepts_projects_capability_for_read() {
        let expected = ResolveProjectRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<daemon_capnp::resolve_project_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::resolve_project_request::Reader<'_>>()
            .unwrap();
        assert_eq!(ResolveProjectRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn register_session_supervisor_request_round_trips_descriptor() {
        let descriptor = session_supervisor_descriptor();
        let expected = RegisterSessionSupervisorRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_slug: "lighting".to_string(),
            descriptor,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message
                    .init_root::<daemon_capnp::register_session_supervisor_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::register_session_supervisor_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            RegisterSessionSupervisorRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn unregister_session_supervisor_round_trips_request_and_result() {
        let descriptor = session_supervisor_descriptor();
        let request = UnregisterSessionSupervisorRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_slug: "lighting".to_string(),
            descriptor,
        };
        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message
                    .init_root::<daemon_capnp::unregister_session_supervisor_request::Builder<'_>>(
                    ),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<daemon_capnp::unregister_session_supervisor_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            UnregisterSessionSupervisorRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = UnregisterSessionSupervisorResult { removed: true };
        let mut result_message = message::Builder::new_default();
        (result).to_capnp(
            result_message
                .init_root::<daemon_capnp::unregister_session_supervisor_result::Builder<'_>>(),
        );
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::unregister_session_supervisor_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            UnregisterSessionSupervisorResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn list_session_supervisors_round_trips_descriptors() {
        let descriptor = session_supervisor_descriptor();
        let request = ListSessionSupervisorsRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            project_id: "local.example".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message
                    .init_root::<daemon_capnp::list_session_supervisors_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<daemon_capnp::list_session_supervisors_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ListSessionSupervisorsRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ListSessionSupervisorsResult {
            supervisors: vec![SessionSupervisorDescriptor {
                session_slug: "lighting".to_string(),
                descriptor,
            }],
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message
                    .init_root::<daemon_capnp::list_session_supervisors_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::list_session_supervisors_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ListSessionSupervisorsResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn ensure_project_session_round_trips_request_and_result() {
        let request = EnsureProjectSessionRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_name: "main".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message
                    .init_root::<daemon_capnp::ensure_project_session_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<daemon_capnp::ensure_project_session_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            EnsureProjectSessionRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ProjectSessionResult {
            manifest: session::SessionManifest::new(
                uuid::Uuid::from_bytes([0x31; 16]),
                "local.example",
                "main",
                "projects/example",
                "projects/example",
                "projects/example/.azoth/sessions/31313131-3131-3131-3131-313131313131",
                session::SessionState::Active,
            ),
            created: true,
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message.init_root::<daemon_capnp::project_session_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::project_session_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ProjectSessionResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn ensure_project_session_request_rejects_project_capability() {
        let request = EnsureProjectSessionRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
            session_name: "main".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<daemon_capnp::ensure_project_session_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(DAEMON_SESSIONS_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn prepare_project_session_services_round_trips_request_and_result() {
        let request = PrepareProjectSessionServicesRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_slug: "main".to_string(),
            endpoint_kind: core::EndpointKind::Tcp,
            skip_build: true,
            service_names: vec!["asset-worker".to_string()],
            otlp_endpoint: Some("http://127.0.0.1:4317".to_string()),
            recover: true,
        };
        let mut request_message = message::Builder::new_default();
        (request).to_capnp(request_message
                .init_root::<daemon_capnp::prepare_project_session_services_request::Builder<'_>>())
        .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<
                daemon_capnp::prepare_project_session_services_request::Reader<'_>,
            >()
            .unwrap();
        assert_eq!(
            PrepareProjectSessionServicesRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ProjectSessionServicesResult {
            manifest: session::SessionManifest::new(
                uuid::Uuid::from_bytes([0x32; 16]),
                "local.example",
                "main",
                "projects/example",
                "projects/example",
                "projects/example/.azoth/sessions/32323232-3232-3232-3232-323232323232",
                session::SessionState::Active,
            ),
            service_names: vec!["project-host".to_string(), "asset-processor".to_string()],
            build_command_count: 2,
            prepared_process_count: 2,
            built: false,
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message
                    .init_root::<daemon_capnp::project_session_services_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::project_session_services_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ProjectSessionServicesResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn prepare_project_session_services_rejects_in_process_endpoint_kind() {
        let request = PrepareProjectSessionServicesRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_slug: "main".to_string(),
            endpoint_kind: core::EndpointKind::InProcess,
            skip_build: false,
            service_names: Vec::new(),
            otlp_endpoint: None,
            recover: false,
        };
        let mut message = message::Builder::new_default();
        let error = (request).to_capnp(message
                .init_root::<daemon_capnp::prepare_project_session_services_request::Builder<'_>>())
        .unwrap_err();

        assert!(
            error.to_string().contains("in-process endpoints"),
            "{error}"
        );
    }

    #[test]
    fn ensure_project_session_services_round_trips_request_and_result() {
        let request = EnsureProjectSessionServicesRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_name: "main".to_string(),
            endpoint_kind: core::EndpointKind::Tcp,
            skip_build: true,
            start_service_names: vec!["project-host".to_string(), "asset-processor".to_string()],
            timeout_ms: 30_000,
            daemon_endpoint: core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37612"),
        };
        let mut request_message = message::Builder::new_default();
        (request).to_capnp(request_message
                .init_root::<daemon_capnp::ensure_project_session_services_request::Builder<'_>>())
        .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<
                daemon_capnp::ensure_project_session_services_request::Reader<'_>,
            >()
            .unwrap();
        assert_eq!(
            EnsureProjectSessionServicesRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ProjectSessionServicesStartResult {
            manifest: session::SessionManifest::new(
                uuid::Uuid::from_bytes([0x33; 16]),
                "local.example",
                "main",
                "projects/example",
                "projects/example",
                "projects/example/.azoth/sessions/33333333-3333-3333-3333-333333333333",
                session::SessionState::Active,
            ),
            service_names: vec!["project-host".to_string(), "asset-processor".to_string()],
            running_service_names: vec!["project-host".to_string(), "asset-processor".to_string()],
            build_command_count: 2,
            prepared_process_count: 2,
            built: false,
            session_created: true,
            supervisor: session_supervisor_descriptor(),
            reused_supervisor: false,
            start_requested: true,
            sessiond_pid: 42,
            log_path: "projects/example/.azoth/sessions/33333333-3333-3333-3333-333333333333/az-sessiond.log".to_string(),
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message
                    .init_root::<daemon_capnp::project_session_services_start_result::Builder<'_>>(
                    ),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::project_session_services_start_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ProjectSessionServicesStartResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn ensure_project_session_services_rejects_in_process_daemon_endpoint() {
        let request = EnsureProjectSessionServicesRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: "local.example".to_string(),
            session_name: "main".to_string(),
            endpoint_kind: core::EndpointKind::Tcp,
            skip_build: false,
            start_service_names: Vec::new(),
            timeout_ms: 30_000,
            daemon_endpoint: core::Endpoint::in_process("azd:test"),
        };
        let mut message = message::Builder::new_default();
        let error = (request).to_capnp(message
                .init_root::<daemon_capnp::ensure_project_session_services_request::Builder<'_>>())
        .unwrap_err();

        assert!(
            error.to_string().contains("in-process endpoints"),
            "{error}"
        );
    }

    #[test]
    fn project_build_plan_round_trips_commands() {
        let expected = ProjectBuildPlan {
            package_profile: Some(ProjectBuildPackageProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "release".to_string(),
                container: "azpack".to_string(),
                compression: "oodle".to_string(),
                oodle_compressor: Some("kraken".to_string()),
                oodle_effort: Some("normal".to_string()),
            }),
            commands: vec![ProjectBuildCommand {
                owner_id: "local.example".to_string(),
                owner_root: "projects/example".to_string(),
                target_name: "game".to_string(),
                program: "cargo".to_string(),
                cwd: "projects/example".to_string(),
                args: vec![
                    "build".to_string(),
                    "-p".to_string(),
                    "example".to_string(),
                    "--release".to_string(),
                ],
                cargo_target_dir: Some("projects/example/target".to_string()),
            }],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<daemon_capnp::project_build_plan::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::project_build_plan::Reader<'_>>()
            .unwrap();
        assert_eq!(ProjectBuildPlan::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn execute_project_build_request_round_trips() {
        let expected = ExecuteProjectBuildRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
            profile: "pc-dev".to_string(),
            target_triple: Some("x86_64-pc-windows-msvc".to_string()),
            package_selectors: vec!["server".to_string(), "azoth-target-server".to_string()],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<daemon_capnp::execute_project_build_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::execute_project_build_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ExecuteProjectBuildRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn plan_project_build_request_round_trips_package_selectors() {
        let expected = PlanProjectBuildRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
            profile: "pc-dev".to_string(),
            target_triple: None,
            package_selectors: vec!["server".to_string()],
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(message.init_root::<daemon_capnp::plan_project_build_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::plan_project_build_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            PlanProjectBuildRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn project_build_execution_result_round_trips_failure() {
        let failing_command = ProjectBuildCommand {
            owner_id: "local.example".to_string(),
            owner_root: "projects/example".to_string(),
            target_name: "game".to_string(),
            program: "cargo".to_string(),
            cwd: "projects/example".to_string(),
            args: vec!["build".to_string(), "-p".to_string(), "example".to_string()],
            cargo_target_dir: None,
        };
        let expected = ProjectBuildExecutionResult {
            success: false,
            plan: ProjectBuildPlan {
                package_profile: None,
                commands: vec![failing_command.clone()],
            },
            completed_command_count: 0,
            failing_command: Some(failing_command),
            diagnostic_headline: "error[E0432]: unresolved import".to_string(),
            diagnostic_tail: "Cargo diagnostics:\nerror[E0432]: unresolved import".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<daemon_capnp::project_build_execution_result::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::project_build_execution_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ProjectBuildExecutionResult::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn project_build_progress_event_round_trips() {
        let expected = ProjectBuildProgressEvent {
            seq: 1,
            command_index: 0,
            command_count: 2,
            target_name: "game".to_string(),
            done_bp: 2_500,
            command_done: 5,
            command_total: 10,
            message: "compiling game".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<daemon_capnp::project_build_progress_event::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::project_build_progress_event::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ProjectBuildProgressEvent::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn project_service_plan_request_round_trips_workspace_root() {
        let expected = PlanProjectServicesRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
            session_slug: "editor-work".to_string(),
            endpoint_kind: core::EndpointKind::WindowsNamedPipe,
            workspace_root: Some("projects/example/.azoth/workspaces/editor-work".to_string()),
            service_names: vec!["asset-worker".to_string()],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<daemon_capnp::plan_project_services_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::plan_project_services_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            PlanProjectServicesRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn plan_project_build_request_rejects_empty_profile() {
        let request = PlanProjectBuildRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.example".to_string(),
            profile: " ".to_string(),
            target_triple: None,
            package_selectors: Vec::new(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<daemon_capnp::plan_project_build_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error.to_string().contains("profile cannot be empty"),
            "{error}"
        );
    }

    #[test]
    fn project_service_plan_round_trips_commands() {
        let expected = ProjectServicePlan {
            build_commands: vec![ProjectBuildCommand {
                owner_id: "local.example".to_string(),
                owner_root: "projects/example".to_string(),
                target_name: "project-host".to_string(),
                program: "cargo".to_string(),
                cwd: "projects/example".to_string(),
                args: vec![
                    "build".to_string(),
                    "-p".to_string(),
                    "example".to_string(),
                    "--bin".to_string(),
                    "project-host".to_string(),
                ],
                cargo_target_dir: None,
            }],
            commands: vec![ProjectServiceCommand {
                owner_id: "local.example".to_string(),
                owner_root: "projects/example".to_string(),
                build_output_root: "projects/example/target".to_string(),
                service_name: "project-host".to_string(),
                role: core::ServiceRole::ProjectHost,
                endpoint: core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:37614"),
                program: "projects/example/target/debug/project-host".to_string(),
                cwd: "projects/example".to_string(),
                args: vec!["--endpoint-kind".to_string(), "tcp".to_string()],
            }],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<daemon_capnp::project_service_plan::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<daemon_capnp::project_service_plan::Reader<'_>>()
            .unwrap();
        assert_eq!(ProjectServicePlan::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn shutdown_daemon_request_rejects_empty_reason() {
        let request = ShutdownDaemonRequest {
            capability: capability(DAEMON_CONTROL_PERMISSION),
            reason: " ".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<daemon_capnp::shutdown_daemon_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error.to_string().contains("reason cannot be empty"),
            "{error}"
        );
    }

    #[test]
    fn shutdown_daemon_round_trips_request_and_result() {
        let request = ShutdownDaemonRequest {
            capability: capability(DAEMON_CONTROL_PERMISSION),
            reason: "operator requested stop".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message.init_root::<daemon_capnp::shutdown_daemon_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<daemon_capnp::shutdown_daemon_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ShutdownDaemonRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ShutdownDaemonResult {
            accepted: true,
            reason: "operator requested stop".to_string(),
        };
        let mut result_message = message::Builder::new_default();
        (result).to_capnp(
            result_message.init_root::<daemon_capnp::shutdown_daemon_result::Builder<'_>>(),
        );
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::shutdown_daemon_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ShutdownDaemonResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn touch_editor_lease_round_trips_request_and_result() {
        let owner_process = ProcessIdentity {
            process_id: 42,
            process_start_time: 9_001,
        };
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: editor_process_lease_id(owner_process),
            owner_process,
            purpose: "editor sidecar owner".to_string(),
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message
                    .init_root::<daemon_capnp::touch_editor_lease_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<daemon_capnp::touch_editor_lease_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            TouchEditorLeaseRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = TouchEditorLeaseResult {
            accepted: true,
            lease_id: editor_process_lease_id(owner_process),
            active_lease_count: 2,
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message.init_root::<daemon_capnp::touch_editor_lease_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<daemon_capnp::touch_editor_lease_result::Reader<'_>>()
            .unwrap();

        assert_eq!(
            TouchEditorLeaseResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn touch_editor_lease_rejects_zero_owner_pid() {
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: "editor-process:0:9001".to_string(),
            owner_process: ProcessIdentity {
                process_id: 0,
                process_start_time: 9_001,
            },
            purpose: "editor sidecar owner".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<daemon_capnp::touch_editor_lease_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("owner process id"), "{error}");
    }

    #[test]
    fn touch_editor_lease_rejects_missing_start_identity() {
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: "editor-process:42:0".to_string(),
            owner_process: ProcessIdentity {
                process_id: 42,
                process_start_time: 0,
            },
            purpose: "editor sidecar owner".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<daemon_capnp::touch_editor_lease_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("start time"), "{error}");
    }
}
