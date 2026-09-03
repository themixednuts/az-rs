//! Cap'n Proto protocol types for `az-sessiond`.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod session_capnp, "azoth/session_capnp.rs");

use std::collections::BTreeSet;

use capnp::Error;
use uuid::Uuid;

pub use az_proto_asset as asset;
pub use az_proto_core as core;
pub use az_proto_project as project;
pub use az_proto_runtime as runtime;
use project::{FromCapnp as _, ToCapnp as _};

pub const SESSION_SUPERVISOR_NAMESPACE: &str = "azoth";
pub const SESSION_SUPERVISOR_SERVICE_NAME: &str = "session-supervisor";
pub const SESSION_SUPERVISOR_AUDIENCE: &str = "session-supervisor";
pub const SESSION_READ_PERMISSION: &str = "session.read";
pub const SESSION_MANAGE_PERMISSION: &str = "session.manage";
pub const SESSION_SAVE_PERMISSION: &str = "session.save";
pub const SESSION_EXEC_PERMISSION: &str = "session.exec";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest {
    pub name: String,
}

impl CreateSessionRequest {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn to_capnp(&self, mut builder: session_capnp::create_session_request::Builder<'_>) {
        builder.set_name(&self.name);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the create session request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::create_session_request::Reader<'_>,
    ) -> Result<Self, Error> {
        Ok(Self {
            name: reader.get_name()?.to_string()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Preparing,
    Active,
    FailedPreserved,
    Removed,
}

impl SessionState {
    #[must_use]
    pub const fn to_capnp(self) -> session_capnp::SessionState {
        match self {
            Self::Preparing => session_capnp::SessionState::Preparing,
            Self::Active => session_capnp::SessionState::Active,
            Self::FailedPreserved => session_capnp::SessionState::FailedPreserved,
            Self::Removed => session_capnp::SessionState::Removed,
        }
    }

    #[must_use]
    pub const fn from_capnp(state: session_capnp::SessionState) -> Self {
        match state {
            session_capnp::SessionState::Preparing => Self::Preparing,
            session_capnp::SessionState::Active => Self::Active,
            session_capnp::SessionState::FailedPreserved => Self::FailedPreserved,
            session_capnp::SessionState::Removed => Self::Removed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceProcessState {
    Planned,
    Starting,
    Running,
    Exited,
    Failed,
}

impl ServiceProcessState {
    #[must_use]
    pub const fn to_capnp(self) -> session_capnp::ServiceProcessState {
        match self {
            Self::Planned => session_capnp::ServiceProcessState::Planned,
            Self::Starting => session_capnp::ServiceProcessState::Starting,
            Self::Running => session_capnp::ServiceProcessState::Running,
            Self::Exited => session_capnp::ServiceProcessState::Exited,
            Self::Failed => session_capnp::ServiceProcessState::Failed,
        }
    }

    #[must_use]
    pub const fn from_capnp(state: session_capnp::ServiceProcessState) -> Self {
        match state {
            session_capnp::ServiceProcessState::Planned => Self::Planned,
            session_capnp::ServiceProcessState::Starting => Self::Starting,
            session_capnp::ServiceProcessState::Running => Self::Running,
            session_capnp::ServiceProcessState::Exited => Self::Exited,
            session_capnp::ServiceProcessState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLogStream {
    Stdout,
    Stderr,
    Structured,
}

impl ServiceLogStream {
    #[must_use]
    pub const fn to_capnp(self) -> session_capnp::ServiceLogStream {
        match self {
            Self::Stdout => session_capnp::ServiceLogStream::Stdout,
            Self::Stderr => session_capnp::ServiceLogStream::Stderr,
            Self::Structured => session_capnp::ServiceLogStream::Structured,
        }
    }

    #[must_use]
    pub const fn from_capnp(stream: session_capnp::ServiceLogStream) -> Self {
        match stream {
            session_capnp::ServiceLogStream::Stdout => Self::Stdout,
            session_capnp::ServiceLogStream::Stderr => Self::Stderr,
            session_capnp::ServiceLogStream::Structured => Self::Structured,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProgramArtifact {
    pub byte_length: u64,
    pub modified_unix_ns: u64,
    pub file_system_id: Option<u64>,
    pub file_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProcessRecord {
    pub owner_id: String,
    pub owner_root: String,
    pub service_name: String,
    pub role: core::ServiceRole,
    /// `UUIDv7` label for this launch. It is observational, never a control key.
    pub run: Uuid,
    pub previous_run: Option<Uuid>,
    pub endpoint: core::Endpoint,
    pub program: String,
    pub program_artifact: Option<ServiceProgramArtifact>,
    pub cwd: String,
    pub args: Vec<String>,
    pub stdout_log: String,
    pub stderr_log: String,
    pub structured_log: String,
    pub state: ServiceProcessState,
    pub pid: Option<u32>,
    pub process_start_time: Option<u64>,
    pub exit_code: Option<i32>,
    pub failure: Option<String>,
    pub planned_unix_ms: u64,
    pub updated_unix_ms: u64,
    pub started_unix_ms: Option<u64>,
    pub exited_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionManifest {
    pub id: Uuid,
    pub project_id: String,
    pub slug: String,
    pub project_root: String,
    pub workspace_root: String,
    pub run_dir: String,
    pub state: SessionState,
    pub services: Vec<core::ServiceDescriptor>,
    pub processes: Vec<ServiceProcessRecord>,
}

impl SessionManifest {
    #[must_use]
    pub fn new(
        id: Uuid,
        project_id: impl Into<String>,
        slug: impl Into<String>,
        project_root: impl Into<String>,
        workspace_root: impl Into<String>,
        run_dir: impl Into<String>,
        state: SessionState,
    ) -> Self {
        Self {
            id,
            project_id: project_id.into(),
            slug: slug.into(),
            project_root: project_root.into(),
            workspace_root: workspace_root.into(),
            run_dir: run_dir.into(),
            state,
            services: Vec::new(),
            processes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWorkspaceStatus {
    pub manifest: SessionManifest,
    pub failure_reason: Option<String>,
}

/// Immutable identity echoed by a running session-supervisor for lease challenges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorIdentity {
    pub process_id: u32,
    /// Platform process-start token used together with `process_id` to reject PID reuse.
    pub process_start_time: u64,
    pub descriptor: core::ServiceDescriptor,
}

fn write_session_supervisor_identity(
    identity: &SessionSupervisorIdentity,
    mut builder: session_capnp::session_supervisor_identity::Builder<'_>,
) -> Result<(), Error> {
    builder.set_process_id(identity.process_id);
    builder.set_process_start_time(identity.process_start_time);
    core::ServiceDescriptor::to_capnp(&identity.descriptor, builder.init_descriptor())
}

fn read_session_supervisor_identity(
    reader: session_capnp::session_supervisor_identity::Reader<'_>,
) -> Result<SessionSupervisorIdentity, Error> {
    Ok(SessionSupervisorIdentity {
        process_id: reader.get_process_id(),
        process_start_time: reader.get_process_start_time(),
        descriptor: core::ServiceDescriptor::from_capnp(reader.get_descriptor()?)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCapabilityRequest {
    pub capability: core::Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSlugRequest {
    pub capability: core::Capability,
    pub slug: String,
}

trait SessionSlugToCapnp<Builder> {
    fn convert_to_capnp(&self, builder: Builder) -> Result<(), Error>;
}

trait SessionSlugFromCapnp<Reader>: Sized {
    fn convert_from_capnp(reader: Reader) -> Result<Self, Error>;
}

impl SessionSlugRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session slug request.
    #[allow(private_bounds)]
    pub fn to_capnp<Builder>(&self, builder: Builder) -> Result<(), Error>
    where
        Self: SessionSlugToCapnp<Builder>,
    {
        <Self as SessionSlugToCapnp<Builder>>::convert_to_capnp(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session slug request is absent from the message or is not
    /// valid UTF-8.
    #[allow(private_bounds)]
    pub fn from_capnp<Reader>(reader: Reader) -> Result<Self, Error>
    where
        Self: SessionSlugFromCapnp<Reader>,
    {
        <Self as SessionSlugFromCapnp<Reader>>::convert_from_capnp(reader)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterServiceRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub descriptor: core::ServiceDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveServiceRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub id: core::ServiceId,
    pub role: core::ServiceRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverSessionRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveGraphDocumentRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub document_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveGraphDocumentResult {
    pub saved: project::SavedDocument,
    pub asset_record: asset::SourceAssetRecordResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopServicesRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopServicesResult {
    pub status: SessionWorkspaceStatus,
    pub stopped: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartServicesRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub reason: String,
    pub service_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartServicesResult {
    pub status: SessionWorkspaceStatus,
    pub started: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorEvent {
    pub sequence: u64,
    pub status: SessionWorkspaceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorEventSubscriptionRequest {
    pub capability: core::Capability,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSupervisorEventSubscriptionResult {
    pub subscribed: bool,
    pub initial_sequence: u64,
    pub initial_status: SessionWorkspaceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLogRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub service_name: String,
    pub run: Option<Uuid>,
    pub stream: ServiceLogStream,
    pub all: bool,
    pub tail_lines: u32,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLogResult {
    pub session_slug: String,
    pub service_name: String,
    pub run: Uuid,
    pub stream: ServiceLogStream,
    pub path: String,
    pub lines: Vec<String>,
    pub next_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecCommandRequest {
    pub capability: core::Capability,
    pub slug: String,
    pub program: String,
    pub args: Vec<String>,
    pub max_output_bytes: u32,
}

// The four bools mirror four separately-numbered `Bool` fields in
// session.capnp (`success @0`, `exited @1`, `stdoutTruncated @5`,
// `stderrTruncated @6`); collapsing them into a flags type would break the
// field-for-field correspondence the write/read pair is audited against.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecCommandResult {
    pub success: bool,
    pub exited: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAssetPackageRootsRequest {
    pub capability: core::Capability,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAssetPackageRootsResult {
    pub roots: Vec<runtime::RuntimeAssetPackageRoot>,
}

fn write_session_manifest(
    manifest: &SessionManifest,
    mut builder: session_capnp::session_manifest::Builder<'_>,
) -> Result<(), Error> {
    validate_session_manifest_for_transport(manifest)?;
    core::SessionId::from(manifest.id).to_capnp(builder.reborrow().init_id());
    builder.set_project_id(&manifest.project_id);
    builder.set_slug(&manifest.slug);
    builder.set_project_root(&manifest.project_root);
    builder.set_workspace_root(&manifest.workspace_root);
    builder.set_run_dir(&manifest.run_dir);
    builder.set_state(manifest.state.to_capnp());
    let mut services = builder
        .reborrow()
        .init_services(capnp_list_index(manifest.services.len())?);
    for (index, service) in manifest.services.iter().enumerate() {
        core::ServiceDescriptor::to_capnp(
            service,
            services.reborrow().get(capnp_list_index(index)?),
        )?;
    }
    let mut processes = builder
        .reborrow()
        .init_processes(capnp_list_index(manifest.processes.len())?);
    for (index, process) in manifest.processes.iter().enumerate() {
        (process).to_capnp(processes.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_session_manifest(
    reader: session_capnp::session_manifest::Reader<'_>,
) -> Result<SessionManifest, Error> {
    let state = reader
        .get_state()
        .map(SessionState::from_capnp)
        .map_err(|error| Error::failed(format!("unknown session state: {error:?}")))?;
    let services = reader
        .get_services()?
        .iter()
        .map(core::ServiceDescriptor::from_capnp)
        .collect::<Result<Vec<_>, Error>>()?;
    let processes = reader
        .get_processes()?
        .iter()
        .map(read_service_process_record)
        .collect::<Result<Vec<_>, Error>>()?;

    let manifest = SessionManifest {
        id: core::SessionId::from_capnp(reader.get_id()?).map(core::SessionId::into_uuid)?,
        project_id: reader.get_project_id()?.to_string()?,
        slug: reader.get_slug()?.to_string()?,
        project_root: reader.get_project_root()?.to_string()?,
        workspace_root: reader.get_workspace_root()?.to_string()?,
        run_dir: reader.get_run_dir()?.to_string()?,
        state,
        services,
        processes,
    };
    validate_session_manifest_for_transport(&manifest)?;
    Ok(manifest)
}

fn validate_session_manifest_for_transport(manifest: &SessionManifest) -> Result<(), Error> {
    if manifest.id == Uuid::nil() {
        return Err(invalid_session_manifest(
            manifest,
            "session id cannot be nil",
        ));
    }
    if manifest.project_id.trim().is_empty()
        || manifest.slug.trim().is_empty()
        || manifest.project_root.trim().is_empty()
        || manifest.workspace_root.trim().is_empty()
        || manifest.run_dir.trim().is_empty()
    {
        return Err(invalid_session_manifest(
            manifest,
            "identity/path fields cannot be empty",
        ));
    }

    let mut seen_descriptors = BTreeSet::new();
    for descriptor in &manifest.services {
        if descriptor.id.namespace.trim().is_empty()
            || descriptor.id.name.trim().is_empty()
            || descriptor.run == Uuid::nil()
        {
            return Err(invalid_session_manifest(
                manifest,
                "service descriptors must carry a service id and non-nil run",
            ));
        }
        if !seen_descriptors.insert((
            descriptor.id.namespace.as_str(),
            descriptor.id.name.as_str(),
            descriptor.role,
        )) {
            return Err(invalid_session_manifest(
                manifest,
                format!(
                    "duplicate descriptor `{}`/`{}` role {:?}",
                    descriptor.id.namespace, descriptor.id.name, descriptor.role
                ),
            ));
        }
    }

    let mut seen_processes = BTreeSet::new();
    for process in &manifest.processes {
        validate_service_process_record_for_transport(manifest, process)?;
        if !seen_processes.insert((process.service_name.as_str(), process.role)) {
            return Err(invalid_session_manifest(
                manifest,
                format!(
                    "duplicate process `{}` role {:?}",
                    process.service_name, process.role
                ),
            ));
        }
    }

    Ok(())
}

fn validate_service_process_record_for_transport(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
) -> Result<(), Error> {
    validate_service_process_record_shape(process)
        .map_err(|reason| invalid_session_manifest(manifest, reason))
}

fn validate_service_process_record_shape(process: &ServiceProcessRecord) -> Result<(), String> {
    if process.owner_id.trim().is_empty()
        || process.owner_root.trim().is_empty()
        || process.service_name.trim().is_empty()
        || process.run == Uuid::nil()
        || process.program.trim().is_empty()
        || process.cwd.trim().is_empty()
        || process.stdout_log.trim().is_empty()
        || process.stderr_log.trim().is_empty()
    {
        return Err(format!(
            "process `{}` identity/path fields cannot be empty",
            process.service_name
        ));
    }
    if process.updated_unix_ms < process.planned_unix_ms {
        return Err(format!(
            "process `{}` updated timestamp precedes planned timestamp",
            process.service_name
        ));
    }
    if process
        .program_artifact
        .as_ref()
        .is_some_and(|artifact| artifact.file_system_id.is_some() != artifact.file_id.is_some())
    {
        return Err(format!(
            "process `{}` program artifact must carry both filesystem and file identities or neither",
            process.service_name
        ));
    }

    match process.state {
        ServiceProcessState::Planned | ServiceProcessState::Starting => {
            validate_unstarted_process_state(process)
        }
        ServiceProcessState::Running => validate_running_process_state(process),
        ServiceProcessState::Exited => validate_exited_process_state(process),
        ServiceProcessState::Failed => validate_failed_process_state(process),
    }
}

fn validate_unstarted_process_state(process: &ServiceProcessRecord) -> Result<(), String> {
    if process.pid.is_some()
        || process.exit_code.is_some()
        || process.failure.is_some()
        || process.started_unix_ms.is_some()
        || process.exited_unix_ms.is_some()
    {
        return Err(format!(
            "{:?} process `{}` cannot carry pid, exit, failure, started, or exited state",
            process.state, process.service_name
        ));
    }
    Ok(())
}

fn validate_running_process_state(process: &ServiceProcessRecord) -> Result<(), String> {
    if process.pid.is_none()
        || process.started_unix_ms.is_none()
        || process.exit_code.is_some()
        || process.failure.is_some()
        || process.exited_unix_ms.is_some()
    {
        return Err(format!(
            "running process `{}` must carry pid and started timestamp without exit or failure state",
            process.service_name
        ));
    }
    Ok(())
}

fn validate_exited_process_state(process: &ServiceProcessRecord) -> Result<(), String> {
    if process.pid.is_some()
        || process.failure.is_some()
        || process.exited_unix_ms.is_none()
        || process.exit_code.is_some_and(|code| code != 0)
    {
        return Err(format!(
            "exited process `{}` cannot carry pid, failure, or non-zero exit state and must carry an exited timestamp",
            process.service_name
        ));
    }
    Ok(())
}

fn validate_failed_process_state(process: &ServiceProcessRecord) -> Result<(), String> {
    if process.pid.is_some() || process.exited_unix_ms.is_none() {
        return Err(format!(
            "failed process `{}` cannot carry a live pid and must carry an exited timestamp",
            process.service_name
        ));
    }

    let has_failure_text = process
        .failure
        .as_deref()
        .is_some_and(|failure| !failure.trim().is_empty());
    let has_non_zero_exit = process.exit_code.is_some_and(|code| code != 0);
    if !has_failure_text && !has_non_zero_exit {
        return Err(format!(
            "failed process `{}` must carry failure text or a non-zero exit code",
            process.service_name
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

fn invalid_session_manifest(manifest: &SessionManifest, reason: impl Into<String>) -> Error {
    Error::failed(format!(
        "invalid session manifest `{}`: {}",
        manifest.slug,
        reason.into()
    ))
}

fn write_optional_i64(
    value: Option<i64>,
    mut builder: core::core_capnp::optional_i64::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_i64(
    reader: core::core_capnp::optional_i64::Reader<'_>,
) -> Result<Option<i64>, Error> {
    match reader.which()? {
        core::core_capnp::optional_i64::Which::None(()) => Ok(None),
        core::core_capnp::optional_i64::Which::Value(value) => Ok(Some(value)),
    }
}

fn write_optional_u64(
    value: Option<u64>,
    mut builder: core::core_capnp::optional_u64::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_u64(
    reader: core::core_capnp::optional_u64::Reader<'_>,
) -> Result<Option<u64>, Error> {
    match reader.which()? {
        core::core_capnp::optional_u64::Which::None(()) => Ok(None),
        core::core_capnp::optional_u64::Which::Value(value) => Ok(Some(value)),
    }
}

fn write_optional_uuid(
    value: Option<Uuid>,
    mut builder: core::core_capnp::optional_uuid::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value.as_bytes()),
        None => builder.set_none(()),
    }
}

fn read_optional_uuid(
    reader: core::core_capnp::optional_uuid::Reader<'_>,
) -> Result<Option<Uuid>, Error> {
    match reader.which()? {
        core::core_capnp::optional_uuid::Which::None(()) => Ok(None),
        core::core_capnp::optional_uuid::Which::Value(value) => Uuid::from_slice(value?)
            .map(Some)
            .map_err(|error| Error::failed(format!("invalid optional UUID: {error}"))),
    }
}

fn read_run(bytes: &[u8], context: &str) -> Result<Uuid, Error> {
    let run = Uuid::from_slice(bytes)
        .map_err(|error| Error::failed(format!("invalid {context} UUID: {error}")))?;
    if run == Uuid::nil() {
        return Err(Error::failed(format!("{context} must not be nil")));
    }
    Ok(run)
}

fn write_service_process_record(
    process: &ServiceProcessRecord,
    mut builder: session_capnp::service_process_record::Builder<'_>,
) -> Result<(), Error> {
    validate_service_process_record_shape(process)
        .map_err(|reason| invalid_session_protocol_value("service process record", reason))?;
    builder.set_owner_id(&process.owner_id);
    builder.set_owner_root(&process.owner_root);
    builder.set_service_name(&process.service_name);
    builder.set_role(process.role.to_capnp());
    builder.set_run(process.run.as_bytes());
    write_optional_uuid(process.previous_run, builder.reborrow().init_previous_run());
    core::Endpoint::to_capnp(&process.endpoint, builder.reborrow().init_endpoint())?;
    builder.set_program(&process.program);
    if let Some(artifact) = process.program_artifact.as_ref() {
        let mut artifact_builder = builder.reborrow().init_program_artifact();
        artifact_builder.set_byte_length(artifact.byte_length);
        artifact_builder.set_modified_unix_ns(artifact.modified_unix_ns);
        if let (Some(file_system_id), Some(file_id)) = (artifact.file_system_id, artifact.file_id) {
            artifact_builder.set_has_file_identity(true);
            artifact_builder.set_file_system_id(file_system_id);
            artifact_builder.set_file_id(file_id);
        }
    }
    builder.set_cwd(&process.cwd);
    let mut args = builder
        .reborrow()
        .init_args(capnp_list_index(process.args.len())?);
    for (index, arg) in process.args.iter().enumerate() {
        args.set(capnp_list_index(index)?, arg);
    }
    builder.set_stdout_log(&process.stdout_log);
    builder.set_stderr_log(&process.stderr_log);
    builder.set_structured_log(&process.structured_log);
    builder.set_state(process.state.to_capnp());
    builder.set_pid(process.pid.unwrap_or(0));
    builder.set_process_start_time(process.process_start_time.unwrap_or(0));
    write_optional_i64(
        process.exit_code.map(i64::from),
        builder.reborrow().init_exit_code(),
    );
    builder.set_failure(process.failure.as_deref().unwrap_or(""));
    builder.set_planned_unix_ms(process.planned_unix_ms);
    builder.set_updated_unix_ms(process.updated_unix_ms);
    builder.set_started_unix_ms(process.started_unix_ms.unwrap_or(0));
    builder.set_exited_unix_ms(process.exited_unix_ms.unwrap_or(0));
    Ok(())
}

fn read_service_process_record(
    reader: session_capnp::service_process_record::Reader<'_>,
) -> Result<ServiceProcessRecord, Error> {
    let args = reader
        .get_args()?
        .iter()
        .map(|arg| Ok(arg?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;
    let state = reader
        .get_state()
        .map(ServiceProcessState::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service process state: {error:?}")))?;
    let failure = reader.get_failure()?.to_string()?;
    let program_artifact = if reader.has_program_artifact() {
        let artifact = reader.get_program_artifact()?;
        let file_identity = artifact
            .get_has_file_identity()
            .then_some((artifact.get_file_system_id(), artifact.get_file_id()));
        Some(ServiceProgramArtifact {
            byte_length: artifact.get_byte_length(),
            modified_unix_ns: artifact.get_modified_unix_ns(),
            file_system_id: file_identity.map(|identity| identity.0),
            file_id: file_identity.map(|identity| identity.1),
        })
    } else {
        None
    };
    let process = ServiceProcessRecord {
        owner_id: reader.get_owner_id()?.to_string()?,
        owner_root: reader.get_owner_root()?.to_string()?,
        service_name: reader.get_service_name()?.to_string()?,
        role: core::ServiceRole::from_capnp(reader.get_role()?),
        run: read_run(reader.get_run()?, "service process run")?,
        previous_run: read_optional_uuid(reader.get_previous_run()?)?,
        endpoint: core::Endpoint::from_capnp(reader.get_endpoint()?)?,
        program: reader.get_program()?.to_string()?,
        program_artifact,
        cwd: reader.get_cwd()?.to_string()?,
        args,
        stdout_log: reader.get_stdout_log()?.to_string()?,
        stderr_log: reader.get_stderr_log()?.to_string()?,
        structured_log: reader.get_structured_log()?.to_string()?,
        state,
        pid: (reader.get_pid() != 0).then_some(reader.get_pid()),
        process_start_time: (reader.get_process_start_time() != 0)
            .then_some(reader.get_process_start_time()),
        exit_code: read_optional_i64(reader.get_exit_code()?)?
            .map(i32::try_from)
            .transpose()
            .map_err(|error| {
                invalid_session_protocol_value(
                    "service process record",
                    format!("exit code is outside the i32 range: {error}"),
                )
            })?,
        failure: (!failure.is_empty()).then_some(failure),
        planned_unix_ms: reader.get_planned_unix_ms(),
        updated_unix_ms: reader.get_updated_unix_ms(),
        started_unix_ms: (reader.get_started_unix_ms() != 0)
            .then_some(reader.get_started_unix_ms()),
        exited_unix_ms: (reader.get_exited_unix_ms() != 0).then_some(reader.get_exited_unix_ms()),
    };
    validate_service_process_record_shape(&process)
        .map_err(|reason| invalid_session_protocol_value("service process record", reason))?;
    Ok(process)
}

fn write_session_workspace_status(
    status: &SessionWorkspaceStatus,
    mut builder: session_capnp::session_workspace_status::Builder<'_>,
) -> Result<(), Error> {
    validate_session_workspace_status_for_transport(status)?;
    (status.manifest).to_capnp(builder.reborrow().init_manifest())?;
    builder.set_failure_reason(status.failure_reason.as_deref().unwrap_or(""));
    Ok(())
}

fn read_session_workspace_status(
    reader: session_capnp::session_workspace_status::Reader<'_>,
) -> Result<SessionWorkspaceStatus, Error> {
    let failure_reason = reader.get_failure_reason()?.to_string()?;
    let status = SessionWorkspaceStatus {
        manifest: SessionManifest::from_capnp(reader.get_manifest()?)?,
        failure_reason: (!failure_reason.is_empty()).then_some(failure_reason),
    };
    validate_session_workspace_status_for_transport(&status)?;
    Ok(status)
}

fn validate_session_workspace_status_for_transport(
    status: &SessionWorkspaceStatus,
) -> Result<(), Error> {
    if status.manifest.state == SessionState::FailedPreserved
        && status
            .failure_reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(invalid_session_protocol_value(
            "session workspace status",
            format!(
                "failed-preserved session `{}` must include a failure reason",
                status.manifest.slug
            ),
        ));
    }
    Ok(())
}

fn write_list_sessions_request(
    request: &SessionCapabilityRequest,
    mut builder: session_capnp::session_supervisor::list_params::Builder<'_>,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "list sessions request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_list_sessions_request(
    reader: session_capnp::session_supervisor::list_params::Reader<'_>,
) -> Result<SessionCapabilityRequest, Error> {
    let request = SessionCapabilityRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_session_capability_request_for_transport(
        "list sessions request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    Ok(request)
}

fn write_status_request(
    request: &SessionSlugRequest,
    mut builder: session_capnp::session_supervisor::status_params::Builder<'_>,
) -> Result<(), Error> {
    validate_session_slug_request_for_transport(
        "status request",
        SESSION_READ_PERMISSION,
        request,
    )?;
    builder.set_slug(&request.slug);
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_status_request(
    reader: session_capnp::session_supervisor::status_params::Reader<'_>,
) -> Result<SessionSlugRequest, Error> {
    let request = SessionSlugRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
    };
    validate_session_slug_request_for_transport(
        "status request",
        SESSION_READ_PERMISSION,
        &request,
    )?;
    Ok(request)
}

fn write_register_service_request(
    request: &RegisterServiceRequest,
    mut builder: session_capnp::session_supervisor::register_service_params::Builder<'_>,
) -> Result<(), Error> {
    validate_register_service_request_for_transport(request)?;
    builder.set_slug(&request.slug);
    core::ServiceDescriptor::to_capnp(&request.descriptor, builder.reborrow().init_descriptor())?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_register_service_request(
    reader: session_capnp::session_supervisor::register_service_params::Reader<'_>,
) -> Result<RegisterServiceRequest, Error> {
    let request = RegisterServiceRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        descriptor: core::ServiceDescriptor::from_capnp(reader.get_descriptor()?)?,
    };
    validate_register_service_request_for_transport(&request)?;
    Ok(request)
}

fn write_resolve_service_request(
    request: &ResolveServiceRequest,
    mut builder: session_capnp::session_supervisor::resolve_service_params::Builder<'_>,
) -> Result<(), Error> {
    validate_resolve_service_request_for_transport(request)?;
    builder.set_slug(&request.slug);
    request.id.to_capnp(builder.reborrow().init_id());
    builder.set_role(request.role.to_capnp());
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_resolve_service_request(
    reader: session_capnp::session_supervisor::resolve_service_params::Reader<'_>,
) -> Result<ResolveServiceRequest, Error> {
    let role = reader
        .get_role()
        .map(core::ServiceRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service role: {error:?}")))?;
    let request = ResolveServiceRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        id: core::ServiceId::from_capnp(reader.get_id()?)?,
        role,
    };
    validate_resolve_service_request_for_transport(&request)?;
    Ok(request)
}

fn write_recover_session_request(
    request: &RecoverSessionRequest,
    mut builder: session_capnp::session_supervisor::recover_params::Builder<'_>,
) -> Result<(), Error> {
    validate_recover_session_request_for_transport(request)?;
    builder.set_slug(&request.slug);
    builder.set_force(request.force);
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_recover_session_request(
    reader: session_capnp::session_supervisor::recover_params::Reader<'_>,
) -> Result<RecoverSessionRequest, Error> {
    let request = RecoverSessionRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        force: reader.get_force(),
    };
    validate_recover_session_request_for_transport(&request)?;
    Ok(request)
}

impl SessionSlugToCapnp<session_capnp::session_supervisor::status_params::Builder<'_>>
    for SessionSlugRequest
{
    fn convert_to_capnp(
        &self,
        builder: session_capnp::session_supervisor::status_params::Builder<'_>,
    ) -> Result<(), Error> {
        write_status_request(self, builder)
    }
}

impl SessionSlugFromCapnp<session_capnp::session_supervisor::status_params::Reader<'_>>
    for SessionSlugRequest
{
    fn convert_from_capnp(
        reader: session_capnp::session_supervisor::status_params::Reader<'_>,
    ) -> Result<Self, Error> {
        read_status_request(reader)
    }
}

impl SessionSlugToCapnp<session_capnp::session_supervisor::retire_params::Builder<'_>>
    for SessionSlugRequest
{
    fn convert_to_capnp(
        &self,
        mut builder: session_capnp::session_supervisor::retire_params::Builder<'_>,
    ) -> Result<(), Error> {
        validate_session_slug_request_for_transport(
            "retire session request",
            SESSION_MANAGE_PERMISSION,
            self,
        )?;
        builder.set_slug(&self.slug);
        core::Capability::to_capnp(&self.capability, builder.reborrow().init_capability())
    }
}

impl SessionSlugFromCapnp<session_capnp::session_supervisor::retire_params::Reader<'_>>
    for SessionSlugRequest
{
    fn convert_from_capnp(
        reader: session_capnp::session_supervisor::retire_params::Reader<'_>,
    ) -> Result<Self, Error> {
        let request = Self {
            capability: core::Capability::from_capnp(reader.get_capability()?)?,
            slug: reader.get_slug()?.to_string()?,
        };
        validate_session_slug_request_for_transport(
            "retire session request",
            SESSION_MANAGE_PERMISSION,
            &request,
        )?;
        Ok(request)
    }
}

fn write_save_graph_document_request(
    request: &SaveGraphDocumentRequest,
    mut builder: session_capnp::save_graph_document_request::Builder<'_>,
) -> Result<(), Error> {
    validate_save_graph_document_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    project::DocumentId::new(&request.document_id).to_capnp(builder.reborrow().init_document());
    Ok(())
}

fn read_save_graph_document_request(
    reader: session_capnp::save_graph_document_request::Reader<'_>,
) -> Result<SaveGraphDocumentRequest, Error> {
    let request = SaveGraphDocumentRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        document_id: project::DocumentId::from_capnp(reader.get_document()?)?
            .as_str()
            .to_owned(),
    };
    validate_save_graph_document_request_for_transport(&request)?;
    Ok(request)
}

fn write_save_graph_document_result(
    result: &SaveGraphDocumentResult,
    mut builder: session_capnp::save_graph_document_result::Builder<'_>,
) -> Result<(), Error> {
    result.saved.to_capnp(builder.reborrow().init_saved())?;
    result
        .asset_record
        .to_capnp(builder.reborrow().init_asset_record())?;
    Ok(())
}

fn read_save_graph_document_result(
    reader: session_capnp::save_graph_document_result::Reader<'_>,
) -> Result<SaveGraphDocumentResult, Error> {
    Ok(SaveGraphDocumentResult {
        saved: project::SavedDocument::from_capnp(reader.get_saved()?)?,
        asset_record: asset::SourceAssetRecordResult::from_capnp(reader.get_asset_record()?)?,
    })
}

fn write_stop_services_request(
    request: &StopServicesRequest,
    mut builder: session_capnp::stop_services_request::Builder<'_>,
) -> Result<(), Error> {
    validate_stop_services_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    builder.set_reason(&request.reason);
    Ok(())
}

fn read_stop_services_request(
    reader: session_capnp::stop_services_request::Reader<'_>,
) -> Result<StopServicesRequest, Error> {
    let request = StopServicesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        reason: reader.get_reason()?.to_string()?,
    };
    validate_stop_services_request_for_transport(&request)?;
    Ok(request)
}

fn write_stop_services_result(
    result: &StopServicesResult,
    mut builder: session_capnp::stop_services_result::Builder<'_>,
) -> Result<(), Error> {
    (result.status).to_capnp(builder.reborrow().init_status())?;
    let mut stopped = builder
        .reborrow()
        .init_stopped(capnp_list_index(result.stopped.len())?);
    for (index, service) in result.stopped.iter().enumerate() {
        stopped.set(capnp_list_index(index)?, service);
    }
    let mut skipped = builder
        .reborrow()
        .init_skipped(capnp_list_index(result.skipped.len())?);
    for (index, service) in result.skipped.iter().enumerate() {
        skipped.set(capnp_list_index(index)?, service);
    }
    Ok(())
}

fn read_stop_services_result(
    reader: session_capnp::stop_services_result::Reader<'_>,
) -> Result<StopServicesResult, Error> {
    Ok(StopServicesResult {
        status: SessionWorkspaceStatus::from_capnp(reader.get_status()?)?,
        stopped: reader
            .get_stopped()?
            .iter()
            .map(|service| Ok(service?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        skipped: reader
            .get_skipped()?
            .iter()
            .map(|service| Ok(service?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    })
}

fn write_start_services_request(
    request: &StartServicesRequest,
    mut builder: session_capnp::start_services_request::Builder<'_>,
) -> Result<(), Error> {
    validate_start_services_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    builder.set_reason(&request.reason);
    let mut service_names = builder
        .reborrow()
        .init_service_names(capnp_list_index(request.service_names.len())?);
    for (index, service_name) in request.service_names.iter().enumerate() {
        service_names.set(capnp_list_index(index)?, service_name);
    }
    Ok(())
}

fn read_start_services_request(
    reader: session_capnp::start_services_request::Reader<'_>,
) -> Result<StartServicesRequest, Error> {
    let request = StartServicesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        reason: reader.get_reason()?.to_string()?,
        service_names: reader
            .get_service_names()?
            .iter()
            .map(|service_name| Ok(service_name?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_start_services_request_for_transport(&request)?;
    Ok(request)
}

fn write_start_services_result(
    result: &StartServicesResult,
    mut builder: session_capnp::start_services_result::Builder<'_>,
) -> Result<(), Error> {
    (result.status).to_capnp(builder.reborrow().init_status())?;
    let mut started = builder
        .reborrow()
        .init_started(capnp_list_index(result.started.len())?);
    for (index, service) in result.started.iter().enumerate() {
        started.set(capnp_list_index(index)?, service);
    }
    let mut skipped = builder
        .reborrow()
        .init_skipped(capnp_list_index(result.skipped.len())?);
    for (index, service) in result.skipped.iter().enumerate() {
        skipped.set(capnp_list_index(index)?, service);
    }
    Ok(())
}

fn read_start_services_result(
    reader: session_capnp::start_services_result::Reader<'_>,
) -> Result<StartServicesResult, Error> {
    Ok(StartServicesResult {
        status: SessionWorkspaceStatus::from_capnp(reader.get_status()?)?,
        started: reader
            .get_started()?
            .iter()
            .map(|service| Ok(service?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        skipped: reader
            .get_skipped()?
            .iter()
            .map(|service| Ok(service?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    })
}

fn write_session_supervisor_event(
    event: &SessionSupervisorEvent,
    mut builder: session_capnp::session_supervisor_event::Builder<'_>,
) -> Result<(), Error> {
    builder.set_sequence(event.sequence);
    event.status.to_capnp(builder.reborrow().init_status())
}

fn read_session_supervisor_event(
    reader: session_capnp::session_supervisor_event::Reader<'_>,
) -> Result<SessionSupervisorEvent, Error> {
    Ok(SessionSupervisorEvent {
        sequence: reader.get_sequence(),
        status: SessionWorkspaceStatus::from_capnp(reader.get_status()?)?,
    })
}

fn write_session_supervisor_event_subscription_request(
    request: &SessionSupervisorEventSubscriptionRequest,
    mut builder: session_capnp::session_supervisor_event_subscription_request::Builder<'_>,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "session-supervisor event subscription request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    validate_session_slug_for_transport(
        "session-supervisor event subscription request",
        &request.slug,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    Ok(())
}

fn read_session_supervisor_event_subscription_request(
    reader: session_capnp::session_supervisor_event_subscription_request::Reader<'_>,
) -> Result<SessionSupervisorEventSubscriptionRequest, Error> {
    let request = SessionSupervisorEventSubscriptionRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
    };
    validate_session_capability_request_for_transport(
        "session-supervisor event subscription request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    validate_session_slug_for_transport(
        "session-supervisor event subscription request",
        &request.slug,
    )?;
    Ok(request)
}

fn write_session_supervisor_event_subscription_result(
    result: &SessionSupervisorEventSubscriptionResult,
    mut builder: session_capnp::session_supervisor_event_subscription_result::Builder<'_>,
) -> Result<(), Error> {
    builder.set_subscribed(result.subscribed);
    builder.set_initial_sequence(result.initial_sequence);
    result
        .initial_status
        .to_capnp(builder.reborrow().init_initial_status())
}

fn read_session_supervisor_event_subscription_result(
    reader: session_capnp::session_supervisor_event_subscription_result::Reader<'_>,
) -> Result<SessionSupervisorEventSubscriptionResult, Error> {
    Ok(SessionSupervisorEventSubscriptionResult {
        subscribed: reader.get_subscribed(),
        initial_sequence: reader.get_initial_sequence(),
        initial_status: SessionWorkspaceStatus::from_capnp(reader.get_initial_status()?)?,
    })
}

fn write_service_log_request(
    request: &ServiceLogRequest,
    mut builder: session_capnp::service_log_request::Builder<'_>,
) -> Result<(), Error> {
    validate_service_log_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    builder.set_service_name(&request.service_name);
    write_optional_uuid(request.run, builder.reborrow().init_run());
    builder.set_stream(request.stream.to_capnp());
    builder.set_all(request.all);
    builder.set_tail_lines(request.tail_lines);
    write_optional_u64(request.offset, builder.reborrow().init_offset());
    Ok(())
}

fn read_service_log_request(
    reader: session_capnp::service_log_request::Reader<'_>,
) -> Result<ServiceLogRequest, Error> {
    let stream = reader
        .get_stream()
        .map(ServiceLogStream::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service log stream: {error:?}")))?;
    let request = ServiceLogRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        service_name: reader.get_service_name()?.to_string()?,
        run: read_optional_uuid(reader.get_run()?)?,
        stream,
        all: reader.get_all(),
        tail_lines: reader.get_tail_lines(),
        offset: read_optional_u64(reader.get_offset()?)?,
    };
    validate_service_log_request_for_transport(&request)?;
    Ok(request)
}

fn write_service_log_result(
    result: &ServiceLogResult,
    mut builder: session_capnp::service_log_result::Builder<'_>,
) -> Result<(), Error> {
    validate_service_log_result_for_transport(result)?;
    builder.set_session_slug(&result.session_slug);
    builder.set_service_name(&result.service_name);
    builder.set_run(result.run.as_bytes());
    builder.set_stream(result.stream.to_capnp());
    builder.set_path(&result.path);
    let mut lines = builder
        .reborrow()
        .init_lines(capnp_list_index(result.lines.len())?);
    for (index, line) in result.lines.iter().enumerate() {
        lines.set(capnp_list_index(index)?, line);
    }
    builder.set_next_offset(result.next_offset);
    Ok(())
}

fn read_service_log_result(
    reader: session_capnp::service_log_result::Reader<'_>,
) -> Result<ServiceLogResult, Error> {
    let stream = reader
        .get_stream()
        .map(ServiceLogStream::from_capnp)
        .map_err(|error| Error::failed(format!("unknown service log stream: {error:?}")))?;
    let lines = reader
        .get_lines()?
        .iter()
        .map(|line| Ok(line?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;
    let result = ServiceLogResult {
        session_slug: reader.get_session_slug()?.to_string()?,
        service_name: reader.get_service_name()?.to_string()?,
        run: read_run(reader.get_run()?, "service log run")?,
        stream,
        path: reader.get_path()?.to_string()?,
        lines,
        next_offset: reader.get_next_offset(),
    };
    validate_service_log_result_for_transport(&result)?;
    Ok(result)
}

fn validate_service_log_result_for_transport(result: &ServiceLogResult) -> Result<(), Error> {
    if result.session_slug.trim().is_empty()
        || result.service_name.trim().is_empty()
        || result.path.trim().is_empty()
    {
        return Err(invalid_session_protocol_value(
            "service log result",
            "session, service, and path fields cannot be empty",
        ));
    }
    if result.run == Uuid::nil() {
        return Err(invalid_session_protocol_value(
            "service log result",
            format!("service `{}` must carry a non-nil run", result.service_name),
        ));
    }
    Ok(())
}

fn write_exec_command_request(
    request: &ExecCommandRequest,
    mut builder: session_capnp::exec_command_request::Builder<'_>,
) -> Result<(), Error> {
    validate_exec_command_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    builder.set_program(&request.program);
    let mut args = builder
        .reborrow()
        .init_args(capnp_list_index(request.args.len())?);
    for (index, arg) in request.args.iter().enumerate() {
        args.set(capnp_list_index(index)?, arg);
    }
    builder.set_max_output_bytes(request.max_output_bytes);
    Ok(())
}

fn read_exec_command_request(
    reader: session_capnp::exec_command_request::Reader<'_>,
) -> Result<ExecCommandRequest, Error> {
    let args = reader
        .get_args()?
        .iter()
        .map(|arg| Ok(arg?.to_string()?))
        .collect::<Result<Vec<_>, Error>>()?;
    let request = ExecCommandRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
        program: reader.get_program()?.to_string()?,
        args,
        max_output_bytes: reader.get_max_output_bytes(),
    };
    validate_exec_command_request_for_transport(&request)?;
    Ok(request)
}

fn write_exec_command_result(
    result: &ExecCommandResult,
    mut builder: session_capnp::exec_command_result::Builder<'_>,
) -> Result<(), Error> {
    validate_exec_command_result_for_transport(result)?;
    builder.set_success(result.success);
    builder.set_exited(result.exited);
    builder.set_exit_code(result.exit_code);
    builder.set_stdout(&result.stdout);
    builder.set_stderr(&result.stderr);
    builder.set_stdout_truncated(result.stdout_truncated);
    builder.set_stderr_truncated(result.stderr_truncated);
    Ok(())
}

fn read_exec_command_result(
    reader: session_capnp::exec_command_result::Reader<'_>,
) -> Result<ExecCommandResult, Error> {
    let result = ExecCommandResult {
        success: reader.get_success(),
        exited: reader.get_exited(),
        exit_code: reader.get_exit_code(),
        stdout: reader.get_stdout()?.to_string()?,
        stderr: reader.get_stderr()?.to_string()?,
        stdout_truncated: reader.get_stdout_truncated(),
        stderr_truncated: reader.get_stderr_truncated(),
    };
    validate_exec_command_result_for_transport(&result)?;
    Ok(result)
}

fn validate_exec_command_result_for_transport(result: &ExecCommandResult) -> Result<(), Error> {
    if result.success && !result.exited {
        return Err(invalid_session_protocol_value(
            "exec command result",
            "successful command result must report an exited process",
        ));
    }
    if result.success && result.exit_code != 0 {
        return Err(invalid_session_protocol_value(
            "exec command result",
            format!(
                "successful command result must report exit code 0, got {}",
                result.exit_code
            ),
        ));
    }
    if !result.exited && result.exit_code != 0 {
        return Err(invalid_session_protocol_value(
            "exec command result",
            format!(
                "non-exited command result cannot carry exit code {}",
                result.exit_code
            ),
        ));
    }
    Ok(())
}

fn validate_save_graph_document_request_for_transport(
    request: &SaveGraphDocumentRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "save graph document request",
        &request.capability,
        SESSION_SAVE_PERMISSION,
    )?;
    validate_session_slug_for_transport("save graph document request", &request.slug)?;
    validate_project_relative_path_for_transport(
        "save graph document request",
        "document id",
        &request.document_id,
    )?;
    Ok(())
}

fn validate_stop_services_request_for_transport(
    request: &StopServicesRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "stop services request",
        &request.capability,
        SESSION_MANAGE_PERMISSION,
    )?;
    validate_session_slug_for_transport("stop services request", &request.slug)?;
    validate_non_empty_text_for_transport("stop services request", "reason", &request.reason)
}

fn validate_start_services_request_for_transport(
    request: &StartServicesRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "start services request",
        &request.capability,
        SESSION_MANAGE_PERMISSION,
    )?;
    validate_session_slug_for_transport("start services request", &request.slug)?;
    validate_non_empty_text_for_transport("start services request", "reason", &request.reason)?;
    let mut seen = std::collections::BTreeSet::new();
    for service_name in &request.service_names {
        validate_non_empty_text_for_transport(
            "start services request",
            "service name",
            service_name,
        )?;
        if !seen.insert(service_name) {
            return Err(invalid_session_protocol_value(
                "start services request",
                format!("duplicate service name `{service_name}`"),
            ));
        }
    }
    Ok(())
}

fn validate_service_log_request_for_transport(request: &ServiceLogRequest) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "service log request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    validate_session_slug_for_transport("service log request", &request.slug)?;
    validate_non_empty_text_for_transport(
        "service log request",
        "service name",
        &request.service_name,
    )?;
    if let Some(run) = request.run
        && run == Uuid::nil()
    {
        return Err(invalid_session_protocol_value(
            "service log request",
            "run must be non-nil when present",
        ));
    }
    Ok(())
}

fn validate_exec_command_request_for_transport(request: &ExecCommandRequest) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "exec command request",
        &request.capability,
        SESSION_EXEC_PERMISSION,
    )?;
    validate_session_slug_for_transport("exec command request", &request.slug)?;
    validate_non_empty_text_for_transport("exec command request", "program", &request.program)
}

fn validate_session_capability_request_for_transport(
    kind: &'static str,
    capability: &core::Capability,
    required_permission: &'static str,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_session_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != SESSION_SUPERVISOR_AUDIENCE {
        return Err(invalid_session_protocol_value(
            kind,
            format!(
                "capability audience must be `{SESSION_SUPERVISOR_AUDIENCE}`, got `{}`",
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
        return Err(invalid_session_protocol_value(
            kind,
            format!("capability role {:?} is not allowed", capability.role),
        ));
    }
    let has_required = capability.has_permissions(&[required_permission]);
    let has_manage = capability.has_permissions(&[SESSION_MANAGE_PERMISSION]);
    if !(has_required || (required_permission == SESSION_READ_PERMISSION && has_manage)) {
        return Err(invalid_session_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    if capability.session.is_none() {
        return Err(invalid_session_protocol_value(
            kind,
            format!("`{required_permission}` capability must be session-scoped"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_session_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_session_slug_request_for_transport(
    kind: &'static str,
    required_permission: &'static str,
    request: &SessionSlugRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        kind,
        &request.capability,
        required_permission,
    )?;
    validate_session_slug_for_transport(kind, &request.slug)
}

fn validate_register_service_request_for_transport(
    request: &RegisterServiceRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "register service request",
        &request.capability,
        SESSION_MANAGE_PERMISSION,
    )?;
    validate_session_slug_for_transport("register service request", &request.slug)?;
    let mut message = capnp::message::Builder::new_default();
    core::ServiceDescriptor::to_capnp(
        &request.descriptor,
        message.init_root::<core::core_capnp::service_descriptor::Builder<'_>>(),
    )
}

fn validate_resolve_service_request_for_transport(
    request: &ResolveServiceRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "resolve service request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    validate_session_slug_for_transport("resolve service request", &request.slug)?;
    if request.id.is_empty() {
        return Err(invalid_session_protocol_value(
            "resolve service request",
            "service id cannot be empty",
        ));
    }
    if request.role == core::ServiceRole::Unknown {
        return Err(invalid_session_protocol_value(
            "resolve service request",
            "service role cannot be Unknown",
        ));
    }
    Ok(())
}

fn validate_recover_session_request_for_transport(
    request: &RecoverSessionRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "recover session request",
        &request.capability,
        SESSION_MANAGE_PERMISSION,
    )?;
    validate_session_slug_for_transport("recover session request", &request.slug)
}

fn validate_session_slug_for_transport(kind: &'static str, slug: &str) -> Result<(), Error> {
    validate_non_empty_text_for_transport(kind, "slug", slug)
}

fn validate_non_empty_text_for_transport(
    kind: &'static str,
    label: &str,
    value: &str,
) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(invalid_session_protocol_value(
            kind,
            format!("{label} cannot be empty"),
        ));
    }
    Ok(())
}

fn validate_project_relative_path_for_transport(
    kind: &'static str,
    label: &str,
    value: &str,
) -> Result<(), Error> {
    if !is_safe_project_relative_path(value) {
        return Err(invalid_session_protocol_value(
            kind,
            format!("{label} `{value}` must be a project-relative source path"),
        ));
    }
    Ok(())
}

fn is_safe_project_relative_path(value: &str) -> bool {
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.trim() != value
    {
        return false;
    }

    let mut has_component = false;
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return false;
        }
        has_component = true;
    }
    has_component
}

fn invalid_session_protocol_value(kind: &'static str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

fn write_runtime_asset_package_roots_request(
    request: &RuntimeAssetPackageRootsRequest,
    mut builder: session_capnp::runtime_asset_package_roots_request::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_asset_package_roots_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_slug(&request.slug);
    Ok(())
}

fn read_runtime_asset_package_roots_request(
    reader: session_capnp::runtime_asset_package_roots_request::Reader<'_>,
) -> Result<RuntimeAssetPackageRootsRequest, Error> {
    let request = RuntimeAssetPackageRootsRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        slug: reader.get_slug()?.to_string()?,
    };
    validate_runtime_asset_package_roots_request_for_transport(&request)?;
    Ok(request)
}

fn write_runtime_asset_package_roots_result(
    result: &RuntimeAssetPackageRootsResult,
    mut builder: session_capnp::runtime_asset_package_roots_result::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_asset_package_roots_result_for_transport(result)?;
    let mut roots = builder
        .reborrow()
        .init_roots(capnp_list_index(result.roots.len())?);
    for (index, root) in result.roots.iter().enumerate() {
        (root).to_capnp(roots.reborrow().get(capnp_list_index(index)?));
    }
    Ok(())
}

fn read_runtime_asset_package_roots_result(
    reader: session_capnp::runtime_asset_package_roots_result::Reader<'_>,
) -> Result<RuntimeAssetPackageRootsResult, Error> {
    let result = RuntimeAssetPackageRootsResult {
        roots: reader
            .get_roots()?
            .iter()
            .map(runtime::RuntimeAssetPackageRoot::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_runtime_asset_package_roots_result_for_transport(&result)?;
    Ok(result)
}

fn validate_runtime_asset_package_roots_request_for_transport(
    request: &RuntimeAssetPackageRootsRequest,
) -> Result<(), Error> {
    validate_session_capability_request_for_transport(
        "runtime asset package roots request",
        &request.capability,
        SESSION_READ_PERMISSION,
    )?;
    validate_session_slug_for_transport("runtime asset package roots request", &request.slug)
}

fn validate_runtime_asset_package_roots_result_for_transport(
    result: &RuntimeAssetPackageRootsResult,
) -> Result<(), Error> {
    runtime::validate_runtime_asset_package_roots(&result.roots).map_err(|error| {
        invalid_session_protocol_value("runtime asset package roots result", error.to_string())
    })
}

impl SessionSupervisorIdentity {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor
    /// identity.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor_identity::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_identity(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor identity is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor_identity::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_identity(reader)
    }
}

impl SessionManifest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session manifest.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_manifest::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_manifest(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session manifest is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: session_capnp::session_manifest::Reader<'_>) -> Result<Self, Error> {
        read_session_manifest(reader)
    }
}

impl ServiceProcessRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the service process record.
    pub fn to_capnp(
        &self,
        builder: session_capnp::service_process_record::Builder<'_>,
    ) -> Result<(), Error> {
        write_service_process_record(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the service process record is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::service_process_record::Reader<'_>,
    ) -> Result<Self, Error> {
        read_service_process_record(reader)
    }
}

impl SessionWorkspaceStatus {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session workspace
    /// status.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_workspace_status::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_workspace_status(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session workspace status is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_workspace_status::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_workspace_status(reader)
    }
}

impl SessionCapabilityRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session capability
    /// request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor::list_params::Builder<'_>,
    ) -> Result<(), Error> {
        write_list_sessions_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session capability request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor::list_params::Reader<'_>,
    ) -> Result<Self, Error> {
        read_list_sessions_request(reader)
    }
}

impl RegisterServiceRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the register service
    /// request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor::register_service_params::Builder<'_>,
    ) -> Result<(), Error> {
        write_register_service_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the register service request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor::register_service_params::Reader<'_>,
    ) -> Result<Self, Error> {
        read_register_service_request(reader)
    }
}

impl ResolveServiceRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the resolve service request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor::resolve_service_params::Builder<'_>,
    ) -> Result<(), Error> {
        write_resolve_service_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the resolve service request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor::resolve_service_params::Reader<'_>,
    ) -> Result<Self, Error> {
        read_resolve_service_request(reader)
    }
}

impl RecoverSessionRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the recover session request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor::recover_params::Builder<'_>,
    ) -> Result<(), Error> {
        write_recover_session_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the recover session request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor::recover_params::Reader<'_>,
    ) -> Result<Self, Error> {
        read_recover_session_request(reader)
    }
}

impl SaveGraphDocumentRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the save graph document
    /// request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::save_graph_document_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_save_graph_document_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the save graph document request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::save_graph_document_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_save_graph_document_request(reader)
    }
}

impl SaveGraphDocumentResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the save graph document
    /// result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::save_graph_document_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_save_graph_document_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the save graph document result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::save_graph_document_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_save_graph_document_result(reader)
    }
}

impl StopServicesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the stop services request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::stop_services_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_stop_services_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the stop services request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::stop_services_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_stop_services_request(reader)
    }
}

impl StopServicesResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the stop services result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::stop_services_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_stop_services_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the stop services result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::stop_services_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_stop_services_result(reader)
    }
}

impl StartServicesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the start services request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::start_services_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_start_services_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the start services request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::start_services_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_start_services_request(reader)
    }
}

impl StartServicesResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the start services result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::start_services_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_start_services_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the start services result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::start_services_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_start_services_result(reader)
    }
}

impl SessionSupervisorEvent {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor
    /// event.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor_event::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_event(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor event is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor_event::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_event(reader)
    }
}

impl SessionSupervisorEventSubscriptionRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor event
    /// subscription request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor_event_subscription_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_event_subscription_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor event subscription request is absent
    /// from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor_event_subscription_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_event_subscription_request(reader)
    }
}

impl SessionSupervisorEventSubscriptionResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the session supervisor event
    /// subscription result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::session_supervisor_event_subscription_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_session_supervisor_event_subscription_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the session supervisor event subscription result is absent
    /// from the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::session_supervisor_event_subscription_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_session_supervisor_event_subscription_result(reader)
    }
}

impl ServiceLogRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the service log request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::service_log_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_service_log_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the service log request is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::service_log_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_service_log_request(reader)
    }
}

impl ServiceLogResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the service log result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::service_log_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_service_log_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the service log result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::service_log_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_service_log_result(reader)
    }
}

impl ExecCommandRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the exec command request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::exec_command_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_exec_command_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the exec command request is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::exec_command_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_exec_command_request(reader)
    }
}

impl ExecCommandResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the exec command result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::exec_command_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_exec_command_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the exec command result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::exec_command_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_exec_command_result(reader)
    }
}

impl RuntimeAssetPackageRootsRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime asset package
    /// roots request.
    pub fn to_capnp(
        &self,
        builder: session_capnp::runtime_asset_package_roots_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_asset_package_roots_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime asset package roots request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::runtime_asset_package_roots_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_asset_package_roots_request(reader)
    }
}

impl RuntimeAssetPackageRootsResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the runtime asset package
    /// roots result.
    pub fn to_capnp(
        &self,
        builder: session_capnp::runtime_asset_package_roots_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_runtime_asset_package_roots_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the runtime asset package roots result is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: session_capnp::runtime_asset_package_roots_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_runtime_asset_package_roots_result(reader)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use capnp::{message, serialize_packed};

    use super::*;

    fn transport_manifest() -> SessionManifest {
        let mut manifest = SessionManifest::new(
            Uuid::from_bytes([4; 16]),
            "local.lighting",
            "lighting",
            "project",
            "project",
            "project/.azoth/sessions/lighting",
            SessionState::Active,
        );
        manifest.services.push(
            core::ServiceDescriptor::new(
                core::ServiceId::new("azoth", "project-host"),
                core::ServiceRole::ProjectHost,
                core::Endpoint::in_process("project-host:lighting"),
            )
            .with_run(Uuid::now_v7())
            .with_capability(
                core::Capability::new(
                    core::ServiceId::new("azoth", "editor"),
                    core::ServiceRole::Editor,
                )
                .with_session(Uuid::from_bytes([4; 16]))
                .with_audience("project-host")
                .with_permissions(["project.schema"])
                .with_token_hash([0x42]),
            ),
        );
        manifest.processes.push(ServiceProcessRecord {
            owner_id: "local.lighting".to_string(),
            owner_root: "project".to_string(),
            service_name: "project-host".to_string(),
            role: core::ServiceRole::ProjectHost,
            run: Uuid::now_v7(),
            previous_run: None,
            endpoint: core::Endpoint::in_process("project-host:lighting"),
            program: "cargo".to_string(),
            program_artifact: Some(ServiceProgramArtifact {
                byte_length: 42,
                modified_unix_ns: 123,
                file_system_id: Some(7),
                file_id: Some(11),
            }),
            cwd: "project".to_string(),
            args: vec![
                "run".to_string(),
                "--bin".to_string(),
                "project-host".to_string(),
            ],
            stdout_log: "project/.azoth/sessions/lighting/logs/project-host.stdout.log".to_string(),
            stderr_log: "project/.azoth/sessions/lighting/logs/project-host.stderr.log".to_string(),
            structured_log: "project/.azoth/sessions/lighting/logs/project-host.capnp.log"
                .to_string(),
            state: ServiceProcessState::Running,
            pid: Some(42),
            process_start_time: Some(9_001),
            exit_code: None,
            failure: None,
            planned_unix_ms: 10,
            updated_unix_ms: 20,
            started_unix_ms: Some(20),
            exited_unix_ms: None,
        });
        manifest
    }

    fn round_trip_manifest(manifest: &SessionManifest) -> Result<SessionManifest, Error> {
        let mut message = message::Builder::new_default();
        let root = message.init_root::<session_capnp::session_manifest::Builder<'_>>();
        (manifest).to_capnp(root)?;

        let reader = message
            .get_root_as_reader::<session_capnp::session_manifest::Reader<'_>>()
            .unwrap();
        SessionManifest::from_capnp(reader)
    }

    fn round_trip_session_workspace_status(
        status: &SessionWorkspaceStatus,
    ) -> Result<SessionWorkspaceStatus, Error> {
        let mut message = message::Builder::new_default();
        let root = message.init_root::<session_capnp::session_workspace_status::Builder<'_>>();
        (status).to_capnp(root)?;

        let reader = message
            .get_root_as_reader::<session_capnp::session_workspace_status::Reader<'_>>()
            .unwrap();
        SessionWorkspaceStatus::from_capnp(reader)
    }

    fn round_trip_service_log_result(result: &ServiceLogResult) -> Result<ServiceLogResult, Error> {
        let mut message = message::Builder::new_default();
        (result).to_capnp(message.init_root::<session_capnp::service_log_result::Builder<'_>>())?;
        let reader = message
            .get_root_as_reader::<session_capnp::service_log_result::Reader<'_>>()
            .unwrap();
        ServiceLogResult::from_capnp(reader)
    }

    fn round_trip_exec_command_result(
        result: &ExecCommandResult,
    ) -> Result<ExecCommandResult, Error> {
        let mut message = message::Builder::new_default();
        (result)
            .to_capnp(message.init_root::<session_capnp::exec_command_result::Builder<'_>>())?;
        let reader = message
            .get_root_as_reader::<session_capnp::exec_command_result::Reader<'_>>()
            .unwrap();
        ExecCommandResult::from_capnp(reader)
    }

    fn test_session_id() -> uuid::Uuid {
        uuid::Uuid::from_bytes([0x31; 16])
    }

    fn session_capability(permission: &'static str) -> core::Capability {
        core::Capability::new(
            core::ServiceId::new("azoth", "editor"),
            core::ServiceRole::Editor,
        )
        .with_session(test_session_id())
        .with_audience(SESSION_SUPERVISOR_AUDIENCE)
        .with_permissions([permission])
    }

    #[test]
    fn create_session_request_round_trips_through_generated_type() {
        let expected = CreateSessionRequest::new("lighting pass");
        let mut message = message::Builder::new_default();
        let root = message.init_root::<session_capnp::create_session_request::Builder<'_>>();
        expected.to_capnp(root);

        let mut bytes = Vec::new();
        serialize_packed::write_message(&mut bytes, &message).unwrap();

        let reader =
            serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())
                .unwrap();
        let root = reader
            .get_root::<session_capnp::create_session_request::Reader<'_>>()
            .unwrap();

        assert_eq!(CreateSessionRequest::from_capnp(root).unwrap(), expected);
    }

    #[test]
    fn generated_session_manifest_uses_core_session_id() {
        let mut message = message::Builder::new_default();
        let mut root = message.init_root::<session_capnp::session_manifest::Builder<'_>>();
        root.set_slug("project-host-edit");
        root.init_id()
            .set_uuid(uuid::Uuid::nil().as_bytes().as_slice());

        let reader = message
            .get_root_as_reader::<session_capnp::session_manifest::Reader<'_>>()
            .unwrap();

        assert_eq!(
            reader.get_slug().unwrap().to_string().unwrap(),
            "project-host-edit"
        );
        assert_eq!(
            core::SessionId::from_capnp(reader.get_id().unwrap())
                .map(core::SessionId::into_uuid)
                .unwrap(),
            uuid::Uuid::nil()
        );
    }

    #[test]
    fn session_supervisor_identity_round_trips_process_and_descriptor() {
        let expected = SessionSupervisorIdentity {
            process_id: 42,
            process_start_time: 9_001,
            descriptor: core::ServiceDescriptor::new(
                core::ServiceId::new("azoth", "session-supervisor"),
                core::ServiceRole::SessionSupervisor,
                core::Endpoint::new(core::EndpointKind::Tcp, "127.0.0.1:43123"),
            )
            .with_run(Uuid::now_v7())
            .with_capability(
                core::Capability::new(
                    core::ServiceId::new("azoth", "editor"),
                    core::ServiceRole::Editor,
                )
                .with_session(Uuid::from_bytes([4; 16]))
                .with_audience(SESSION_SUPERVISOR_AUDIENCE)
                .with_permissions([SESSION_READ_PERMISSION])
                .with_token_hash([0x42]),
            ),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<session_capnp::session_supervisor_identity::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<session_capnp::session_supervisor_identity::Reader<'_>>()
            .unwrap();

        assert_eq!(
            SessionSupervisorIdentity::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn status_request_round_trips_typed_session_boundary() {
        let expected = SessionSlugRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(
                message
                    .init_root::<session_capnp::session_supervisor::status_params::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<session_capnp::session_supervisor::status_params::Reader<'_>>()
            .unwrap();

        assert_eq!(SessionSlugRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn list_sessions_request_rejects_wrong_permission() {
        let request = SessionCapabilityRequest {
            capability: session_capability(SESSION_EXEC_PERMISSION),
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(
                message.init_root::<session_capnp::session_supervisor::list_params::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(SESSION_READ_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn list_sessions_request_accepts_manage_capability_for_read() {
        let expected = SessionCapabilityRequest {
            capability: session_capability(SESSION_MANAGE_PERMISSION),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<session_capnp::session_supervisor::list_params::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<session_capnp::session_supervisor::list_params::Reader<'_>>()
            .unwrap();

        assert_eq!(
            SessionCapabilityRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn register_service_request_rejects_empty_slug() {
        let request = RegisterServiceRequest {
            capability: session_capability(SESSION_MANAGE_PERMISSION),
            slug: " ".to_string(),
            descriptor: transport_manifest().services[0].clone(),
        };
        let mut message = message::Builder::new_default();
        let error = (request).to_capnp(message
                .init_root::<session_capnp::session_supervisor::register_service_params::Builder<'_>>(
                ))
        .unwrap_err();

        assert!(
            error.to_string().contains("slug cannot be empty"),
            "{error}"
        );
    }

    #[test]
    fn resolve_service_request_rejects_unknown_role() {
        let request = ResolveServiceRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
            id: core::ServiceId::new("azoth", "project-host"),
            role: core::ServiceRole::Unknown,
        };
        let mut message = message::Builder::new_default();
        let error = (request).to_capnp(message
                .init_root::<session_capnp::session_supervisor::resolve_service_params::Builder<'_>>(
                ))
        .unwrap_err();

        assert!(
            error.to_string().contains("service role cannot be Unknown"),
            "{error}"
        );
    }

    #[test]
    fn retire_session_request_requires_manage_permission() {
        let request = SessionSlugRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
        };
        let mut message = message::Builder::new_default();
        let error = request
            .to_capnp(
                message
                    .init_root::<session_capnp::session_supervisor::retire_params::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(SESSION_MANAGE_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn session_manifest_round_trips_all_stable_fields() {
        let expected = transport_manifest();
        let actual = round_trip_manifest(&expected).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn session_manifest_encode_rejects_duplicate_process_identity() {
        let mut manifest = transport_manifest();
        manifest.processes.push(manifest.processes[0].clone());

        let error = round_trip_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("duplicate process"), "{error}");
    }

    #[test]
    fn session_manifest_encode_rejects_incoherent_running_process() {
        let mut manifest = transport_manifest();
        manifest.processes[0].pid = None;

        let error = round_trip_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("running process"), "{error}");
    }

    #[test]
    fn session_manifest_encode_rejects_unexplained_failed_process() {
        let mut manifest = transport_manifest();
        let process = &mut manifest.processes[0];
        process.state = ServiceProcessState::Failed;
        process.pid = None;
        process.exit_code = Some(0);
        process.failure = None;
        process.exited_unix_ms = Some(30);

        let error = round_trip_manifest(&manifest).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failure text or a non-zero exit code"),
            "{error}"
        );
    }

    #[test]
    fn session_workspace_status_round_trips_failure_reason() {
        let manifest = SessionManifest::new(
            Uuid::from_bytes([5; 16]),
            "local.dirty",
            "dirty",
            "project",
            "project",
            "project/.azoth/sessions/dirty",
            SessionState::Active,
        );
        let expected = SessionWorkspaceStatus {
            manifest,
            failure_reason: Some("service process failed".to_string()),
        };

        let actual = round_trip_session_workspace_status(&expected).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn session_workspace_status_encode_requires_failed_preserved_reason() {
        let mut status = SessionWorkspaceStatus {
            manifest: transport_manifest(),
            failure_reason: Some("   ".to_string()),
        };
        status.manifest.state = SessionState::FailedPreserved;
        status.manifest.processes.clear();

        let error = round_trip_session_workspace_status(&status).unwrap_err();

        assert!(
            error.to_string().contains("must include a failure reason"),
            "{error}"
        );
    }

    #[test]
    fn save_graph_document_request_and_result_round_trip_project_and_asset_records() {
        let request = SaveGraphDocumentRequest {
            capability: session_capability(SESSION_SAVE_PERMISSION),
            slug: "lighting".to_string(),
            document_id: "prefabs/light.prefab.ron".to_string(),
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message
                    .init_root::<session_capnp::save_graph_document_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<session_capnp::save_graph_document_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SaveGraphDocumentRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = SaveGraphDocumentResult {
            saved: project::SavedDocument {
                document_id: project::DocumentId::new("prefabs/light.prefab.ron"),
                revision: project::DocumentRevision::new(3),
                source_path: "prefabs/light.prefab.ron".to_string(),
                schema_type: "az.tests.LightPrefab".to_string(),
                content_hash: vec![9; 32],
                byte_length: 128,
            },
            asset_record: asset::SourceAssetRecordResult {
                asset_guid: Uuid::from_bytes([6; 16]),
                entry: asset::WorkspaceEntry {
                    entry_id: 11,
                    workspace_id: 22,
                    asset_guid: Uuid::from_bytes([6; 16]),
                    root_id: 44,
                    source_path: "prefabs/light.prefab.ron".to_string(),
                    schema_type: Some("az.tests.LightPrefab".to_string()),
                    content_hash:
                        "0909090909090909090909090909090909090909090909090909090909090909"
                            .to_string(),
                    diff: asset::WorkspaceEntryDiff::Clean,
                    diagnostics_count: 0,
                    updated_unix_ms: 55,
                    jobs: Vec::new(),
                },
            },
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message
                    .init_root::<session_capnp::save_graph_document_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<session_capnp::save_graph_document_result::Reader<'_>>()
            .unwrap();

        assert_eq!(
            SaveGraphDocumentResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn save_graph_document_request_encode_rejects_unsafe_document_id() {
        let request = SaveGraphDocumentRequest {
            capability: session_capability(SESSION_SAVE_PERMISSION),
            slug: "lighting".to_string(),
            document_id: "../outside.prefab.ron".to_string(),
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(
                message.init_root::<session_capnp::save_graph_document_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains("project-relative source path"),
            "{error}"
        );
    }

    #[test]
    fn stop_services_request_and_result_round_trip() {
        let request = StopServicesRequest {
            capability: session_capability(SESSION_MANAGE_PERMISSION),
            slug: "lighting".to_string(),
            reason: "operator requested shutdown".to_string(),
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message.init_root::<session_capnp::stop_services_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<session_capnp::stop_services_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            StopServicesRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = StopServicesResult {
            status: SessionWorkspaceStatus {
                manifest: transport_manifest(),
                failure_reason: None,
            },
            stopped: vec!["project-host".to_string()],
            skipped: vec!["asset-processor".to_string()],
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message.init_root::<session_capnp::stop_services_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<session_capnp::stop_services_result::Reader<'_>>()
            .unwrap();

        assert_eq!(
            StopServicesResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn start_services_request_and_result_round_trip() {
        let request = StartServicesRequest {
            capability: session_capability(SESSION_MANAGE_PERMISSION),
            slug: "lighting".to_string(),
            reason: "editor attach requires project services".to_string(),
            service_names: vec!["project-host".to_string(), "asset-processor".to_string()],
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message.init_root::<session_capnp::start_services_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<session_capnp::start_services_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            StartServicesRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = StartServicesResult {
            status: SessionWorkspaceStatus {
                manifest: transport_manifest(),
                failure_reason: None,
            },
            started: vec!["project-host".to_string()],
            skipped: vec!["asset-processor".to_string()],
        };
        let mut result_message = message::Builder::new_default();
        (result)
            .to_capnp(
                result_message.init_root::<session_capnp::start_services_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<session_capnp::start_services_result::Reader<'_>>()
            .unwrap();

        assert_eq!(
            StartServicesResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn service_log_request_and_result_round_trip() {
        assert_eq!(
            ServiceLogStream::from_capnp(ServiceLogStream::Structured.to_capnp()),
            ServiceLogStream::Structured
        );
        let request = ServiceLogRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
            service_name: "project-host".to_string(),
            run: Some(Uuid::now_v7()),
            stream: ServiceLogStream::Stderr,
            all: false,
            tail_lines: 50,
            offset: Some(128),
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message.init_root::<session_capnp::service_log_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<session_capnp::service_log_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ServiceLogRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ServiceLogResult {
            session_slug: "lighting".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stderr,
            path: "project/.azoth/sessions/lighting/logs/project-host.stderr.log".to_string(),
            lines: vec!["first".to_string(), "second".to_string()],
            next_offset: 256,
        };
        assert_eq!(round_trip_service_log_result(&result).unwrap(), result);
    }

    #[test]
    fn service_log_result_encode_rejects_missing_identity() {
        let result = ServiceLogResult {
            session_slug: "lighting".to_string(),
            service_name: String::new(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stdout,
            path: "project/.azoth/sessions/lighting/logs/project-host.stdout.log".to_string(),
            lines: Vec::new(),
            next_offset: 0,
        };

        let error = round_trip_service_log_result(&result).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("session, service, and path fields cannot be empty"),
            "{error}"
        );
    }

    #[test]
    fn service_log_result_encode_requires_non_nil_run() {
        let result = ServiceLogResult {
            session_slug: "lighting".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::nil(),
            stream: ServiceLogStream::Stdout,
            path: "project/.azoth/sessions/lighting/logs/project-host.stdout.log".to_string(),
            lines: Vec::new(),
            next_offset: 0,
        };

        let error = round_trip_service_log_result(&result).unwrap_err();

        assert!(error.to_string().contains("non-nil run"), "{error}");
    }

    #[test]
    fn service_log_request_encode_rejects_nil_run() {
        let request = ServiceLogRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
            service_name: "project-host".to_string(),
            run: Some(Uuid::nil()),
            stream: ServiceLogStream::Stdout,
            all: false,
            tail_lines: 0,
            offset: None,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<session_capnp::service_log_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("run must be non-nil"), "{error}");
    }

    #[test]
    fn exec_command_request_and_result_round_trip() {
        let request = ExecCommandRequest {
            capability: session_capability(SESSION_EXEC_PERMISSION),
            slug: "lighting".to_string(),
            program: "cargo".to_string(),
            args: vec![
                "check".to_string(),
                "-p".to_string(),
                "az-editor".to_string(),
            ],
            max_output_bytes: 4096,
        };

        let mut request_message = message::Builder::new_default();
        (request)
            .to_capnp(
                request_message.init_root::<session_capnp::exec_command_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<session_capnp::exec_command_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            ExecCommandRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = ExecCommandResult {
            success: false,
            exited: true,
            exit_code: 101,
            stdout: "checking".to_string(),
            stderr: "failed".to_string(),
            stdout_truncated: false,
            stderr_truncated: true,
        };
        assert_eq!(round_trip_exec_command_result(&result).unwrap(), result);
    }

    #[test]
    fn exec_command_request_encode_rejects_unscoped_capability() {
        let mut capability = session_capability(SESSION_EXEC_PERMISSION);
        capability.session = None;
        let request = ExecCommandRequest {
            capability,
            slug: "lighting".to_string(),
            program: "cargo".to_string(),
            args: vec!["check".to_string()],
            max_output_bytes: 0,
        };
        let mut message = message::Builder::new_default();

        let error = (request)
            .to_capnp(message.init_root::<session_capnp::exec_command_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("session-scoped"), "{error}");
    }

    #[test]
    fn exec_command_result_encode_rejects_success_without_clean_exit() {
        let result = ExecCommandResult {
            success: true,
            exited: true,
            exit_code: 101,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let error = round_trip_exec_command_result(&result).unwrap_err();

        assert!(error.to_string().contains("exit code 0"), "{error}");
    }

    #[test]
    fn exec_command_result_encode_rejects_non_exited_exit_code() {
        let result = ExecCommandResult {
            success: false,
            exited: false,
            exit_code: 9,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let error = round_trip_exec_command_result(&result).unwrap_err();

        assert!(error.to_string().contains("non-exited command"), "{error}");
    }

    #[test]
    fn session_supervisor_event_and_subscription_round_trip() {
        let status = SessionWorkspaceStatus {
            manifest: transport_manifest(),
            failure_reason: None,
        };
        let event = SessionSupervisorEvent {
            sequence: 42,
            status: status.clone(),
        };
        let mut event_message = message::Builder::new_default();
        event
            .to_capnp(
                event_message.init_root::<session_capnp::session_supervisor_event::Builder<'_>>(),
            )
            .unwrap();
        let event_reader = event_message
            .get_root_as_reader::<session_capnp::session_supervisor_event::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SessionSupervisorEvent::from_capnp(event_reader).unwrap(),
            event
        );

        let request = SessionSupervisorEventSubscriptionRequest {
            capability: session_capability(SESSION_READ_PERMISSION),
            slug: "lighting".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        request
            .to_capnp(request_message.init_root::<
                session_capnp::session_supervisor_event_subscription_request::Builder<'_>,
            >())
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<
                session_capnp::session_supervisor_event_subscription_request::Reader<'_>,
            >()
            .unwrap();
        assert_eq!(
            SessionSupervisorEventSubscriptionRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = SessionSupervisorEventSubscriptionResult {
            subscribed: true,
            initial_sequence: 41,
            initial_status: status,
        };
        let mut result_message = message::Builder::new_default();
        result
            .to_capnp(result_message.init_root::<
                session_capnp::session_supervisor_event_subscription_result::Builder<'_>,
            >())
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<
                session_capnp::session_supervisor_event_subscription_result::Reader<'_>,
            >()
            .unwrap();
        assert_eq!(
            SessionSupervisorEventSubscriptionResult::from_capnp(result_reader).unwrap(),
            result
        );
    }
}
