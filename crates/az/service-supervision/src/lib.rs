//! Scope-neutral service supervision primitives.
//!
//! Project instances and sessions own different service sets, but persist the
//! same descriptor, readiness, process, and child-lifecycle shapes. This crate
//! owns those shared shapes without deciding which scope owns a service.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::UNIX_EPOCH;

use az_filesystem::file_identity;
use az_proto_core::{
    Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor, ServiceId, ServiceRole,
};
use az_work::{OwnedSynchronousChild, OwnedSynchronousCommandTree};
use crossbeam_channel as channel;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod service_lifecycle;
pub use service_lifecycle::{
    ServiceLifecycleController, ServiceLifecycleShutdownError, request_service_lifecycle_shutdown,
};

pub const SERVICE_READY_SCHEMA_VERSION: u32 = 1;

// Serde's `skip_serializing_if` is handed `&FieldType`, so this predicate must
// take `&PathBuf`; `&Path` would not satisfy the two call sites below.
#[allow(clippy::ptr_arg)]
fn path_buf_is_empty(path: &PathBuf) -> bool {
    path.as_os_str().is_empty()
}

/// Stable, inexpensive identity for one executable artifact.
///
/// Service program artifacts describe a concrete launched program, not merely
/// a path. Cargo replaces binaries in place, so comparing the path alone can
/// silently keep an older mapped image alive after a successful build. The
/// filesystem identity catches replacement while size and modification time
/// cover filesystems that do not expose a stable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServiceProgramArtifact {
    pub byte_length: u64,
    pub modified_unix_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_system_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u64>,
}

impl ServiceProgramArtifact {
    /// Identify the executable currently at `path`.
    ///
    /// # Errors
    ///
    /// Returns the [`std::io::Error`] from reading `path`'s metadata or its
    /// modification time, or [`std::io::ErrorKind::InvalidInput`] if `path`
    /// names something other than a file.
    pub fn capture(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("service program `{}` is not a file", path.display()),
            ));
        }
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|source| std::io::Error::new(std::io::ErrorKind::InvalidData, source))?;
        let (file_system_id, file_id) = file_identity(path, &metadata)
            .map_or((None, None), |identity| {
                (Some(identity.file_system_id), Some(identity.file_id))
            });
        Ok(Self {
            byte_length: metadata.len(),
            modified_unix_ns: u64::try_from(modified.as_nanos()).unwrap_or(u64::MAX),
            file_system_id,
            file_id,
        })
    }
}

/// A running process and the platform token for the instant it started.
///
/// A pid on its own is not an identity: the operating system recycles pids, so
/// a live pid answers "something is running" and never "the process I launched
/// is running". The start token settles that. It is only meaningful compared
/// with another token read for the same pid, and it changes the moment the pid
/// is handed to a different process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub process_id: u32,
    /// Platform process-start token. Its unit is platform-specific and it is
    /// only meaningful when compared with another token for the same PID.
    pub process_start_time: u64,
}

/// An OS-owned, identity-bound wait for the process that was launched.
///
/// Binding verifies the persisted start token before returning.  The wait then
/// stays attached to the opened process object (Windows) or pidfd (Linux), so
/// a recycled numeric PID can never satisfy it.  Platforms without such an
/// object refuse to bind rather than falling back to PID polling.
pub struct ProcessExitWaiter {
    identity: ProcessIdentity,
    binding: ProcessExitBinding,
}

enum ProcessExitBinding {
    #[cfg(windows)]
    Windows(WindowsProcessExitHandle),
    #[cfg(target_os = "linux")]
    Linux(LinuxPidFd),
}

impl ProcessExitWaiter {
    /// Bind an exit wait to an already-captured launch identity.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::InvalidProcessIdentity`] if `identity`
    /// has a zero pid or start token,
    /// [`ServiceProcessError::ProcessExitIdentityMismatch`] if the pid has been
    /// recycled by a different process, or
    /// [`ServiceProcessError::ProcessExitBindingUnavailable`] if the platform
    /// refuses the exit handle.
    pub fn bind(identity: ProcessIdentity) -> Result<Self, ServiceProcessError> {
        if identity.process_id == 0 || identity.process_start_time == 0 {
            return Err(ServiceProcessError::InvalidProcessIdentity { identity });
        }
        platform_bind_process_exit_waiter(identity)
    }

    #[must_use]
    pub const fn identity(&self) -> ProcessIdentity {
        self.identity
    }

    /// Block until this exact process exits.
    ///
    /// This intentionally has no timeout. Callers combine it with their one
    /// semantic deadline at the orchestration boundary rather than waking on a
    /// cadence to ask whether the process is still alive.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::Io`] if the platform wait fails; the
    /// bound handle already pins the identity, so a recycled pid cannot be
    /// mistaken for an exit here.
    pub fn wait(self) -> Result<ProcessIdentity, ServiceProcessError> {
        platform_wait_for_process_exit(self)
    }

    fn wait_or_cancel(
        self,
        cancellation: &ProcessExitCancellation,
    ) -> Result<ProcessExitWaitOutcome, ServiceProcessError> {
        platform_wait_for_process_exit_or_cancel(self, cancellation)
    }
}

/// The terminal result of one identity-bound exit wait.
///
/// Cancellation only retires the observation. It never acts on the process;
/// the launch owner remains responsible for termination when that is wanted.
enum ProcessExitWaitOutcome {
    Exited(ProcessIdentity),
    Cancelled,
}

/// Native wake object paired with an exact process wait. Both supported
/// backends wait on the process object and this object in one OS call, so
/// cancellation has no timer or forwarding thread.
#[derive(Clone)]
struct ProcessExitCancellation {
    #[cfg(windows)]
    event: Arc<WindowsExitWaitCancellationHandle>,
    #[cfg(target_os = "linux")]
    event: Arc<LinuxExitWaitCancellationFd>,
}

impl ProcessExitCancellation {
    fn new(identity: ProcessIdentity) -> Result<Self, ServiceProcessError> {
        platform_new_process_exit_cancellation(identity)
    }

    fn cancel(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        platform_cancel_process_exit_wait(self, identity)
    }
}

impl ProcessIdentity {
    /// Read the identity of the calling process.
    ///
    /// # Errors
    ///
    /// Returns any [`std::io::Error`] [`Self::capture`] returns, or
    /// [`std::io::ErrorKind::NotFound`] in the degenerate case where the
    /// calling process's own start token cannot be read.
    pub fn current() -> std::io::Result<Self> {
        let process_id = std::process::id();
        Self::capture(process_id)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("current process {process_id} disappeared"),
            )
        })
    }

    /// Read the identity of `process_id`, or `None` when nothing is running
    /// under that pid.
    ///
    /// # Errors
    ///
    /// Returns the [`std::io::Error`] from the platform start-time probe. A pid
    /// that simply is not running is `Ok(None)`, not an error.
    pub fn capture(process_id: u32) -> std::io::Result<Option<Self>> {
        Ok(
            platform_process_start_time(process_id)?.map(|process_start_time| Self {
                process_id,
                process_start_time,
            }),
        )
    }

    /// Assess whether this exact process is still running.
    ///
    /// # Errors
    ///
    /// Returns any [`std::io::Error`] [`RecordedProcess::assess`] returns.
    pub fn assess(self) -> std::io::Result<RecordedProcess> {
        RecordedProcess::assess(Some(self.process_id), Some(self.process_start_time))
    }
}

/// What the process a record names actually is, right now.
///
/// Every question of the form "is the process I recorded still running" is
/// answered here, and none of them may reduce to a bare pid probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedProcess {
    /// The record names no pid at all.
    Unrecorded,
    /// The record names a pid without the start token that attributes it to a
    /// launch, so the pid cannot be trusted to be the launched process. Only
    /// records written before the token existed reach this state.
    Unattributable { process_id: u32 },
    /// Nothing is running under the recorded pid.
    Exited { process_id: u32 },
    /// Something is running under the recorded pid, but it started at a
    /// different instant, so it is a different process wearing a recycled pid.
    Reused {
        recorded: ProcessIdentity,
        actual_start_time: u64,
    },
    /// The recorded process is running.
    Live { recorded: ProcessIdentity },
}

impl RecordedProcess {
    /// Answer the liveness question for a recorded `(pid, start_time)` pair.
    ///
    /// # Errors
    ///
    /// Returns the [`std::io::Error`] from the platform start-time probe for
    /// `process_id`. Every "not running" and "recycled pid" answer is a
    /// successful [`RecordedProcess`] variant rather than an error.
    pub fn assess(
        process_id: Option<u32>,
        process_start_time: Option<u64>,
    ) -> std::io::Result<Self> {
        let Some(process_id) = process_id else {
            return Ok(Self::Unrecorded);
        };
        let Some(process_start_time) = process_start_time else {
            return Ok(Self::Unattributable { process_id });
        };
        let recorded = ProcessIdentity {
            process_id,
            process_start_time,
        };
        Ok(match ProcessIdentity::capture(process_id)? {
            None => Self::Exited { process_id },
            Some(actual) if actual != recorded => Self::Reused {
                recorded,
                actual_start_time: actual.process_start_time,
            },
            Some(_) => Self::Live { recorded },
        })
    }

    /// True only when the launched process itself is still running.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Live { .. })
    }
}

#[cfg(windows)]
fn platform_process_start_time(process_id: u32) -> std::io::Result<Option<u64>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: `OpenProcess` takes no pointers and returns an owned handle that
    // is closed on every path below.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
    {
        Ok(handle) => handle,
        // Low 16 bits of the HRESULT are the Win32 code; 87 is
        // ERROR_INVALID_PARAMETER, which `OpenProcess` returns for a pid that
        // has already been reaped.
        Err(error) if (error.code().0.cast_unsigned() & 0xffff) == 87 => return Ok(None),
        Err(error) => return Err(std::io::Error::other(error)),
    };
    let result = query_windows_process_start_time(handle);
    // SAFETY: `handle` is live and closed exactly once.
    let _ = unsafe { CloseHandle(handle) };
    result
}

#[cfg(windows)]
fn query_windows_process_start_time(
    handle: windows::Win32::Foundation::HANDLE,
) -> std::io::Result<Option<u64>> {
    use windows::Win32::Foundation::{FILETIME, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessTimes};

    let mut exit_code = 0;
    // SAFETY: `handle` is live and `exit_code` is a valid output slot.
    unsafe { GetExitCodeProcess(handle, &raw mut exit_code) }.map_err(std::io::Error::other)?;
    if exit_code != STILL_ACTIVE.0 as u32 {
        return Ok(None);
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `handle` is live and all four outputs are valid slots.
    unsafe {
        GetProcessTimes(
            handle,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    }
    .map_err(std::io::Error::other)?;
    Ok(Some(
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime),
    ))
}

#[cfg(target_os = "linux")]
fn platform_process_start_time(process_id: u32) -> std::io::Result<Option<u64>> {
    let path = Path::new("/proc").join(process_id.to_string()).join("stat");
    let stat = match fs::read_to_string(path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(after_name) = stat.rsplit_once(')').map(|(_, rest)| rest.trim()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("process {process_id} has malformed /proc stat"),
        ));
    };
    let fields = after_name.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"Z") {
        return Ok(None);
    }
    let start_time = fields
        .get(19)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("process {process_id} /proc stat has no start time"),
            )
        })?
        .parse::<u64>()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(start_time))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_process_start_time(process_id: u32) -> std::io::Result<Option<u64>> {
    // SAFETY: signal 0 performs the permission and existence check only.
    let alive = unsafe { libc::kill(process_id as i32, 0) == 0 };
    if !alive {
        return Ok(None);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process start-time queries are not implemented on this Unix target",
    ))
}

#[cfg(not(any(windows, unix)))]
fn platform_process_start_time(_process_id: u32) -> std::io::Result<Option<u64>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process start-time queries are not implemented on this platform",
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisedServiceRole {
    Editor,
    Daemon,
    SessionSupervisor,
    ProjectHost,
    AssetProcessor,
    RuntimeHost,
    Worker,
}

impl SupervisedServiceRole {
    #[must_use]
    pub const fn from_proto(role: ServiceRole) -> Option<Self> {
        match role {
            ServiceRole::Unknown => None,
            ServiceRole::Editor => Some(Self::Editor),
            ServiceRole::Daemon => Some(Self::Daemon),
            ServiceRole::SessionSupervisor => Some(Self::SessionSupervisor),
            ServiceRole::ProjectHost => Some(Self::ProjectHost),
            ServiceRole::AssetProcessor => Some(Self::AssetProcessor),
            ServiceRole::RuntimeHost => Some(Self::RuntimeHost),
            ServiceRole::Worker => Some(Self::Worker),
        }
    }

    #[must_use]
    pub const fn to_proto(self) -> ServiceRole {
        match self {
            Self::Editor => ServiceRole::Editor,
            Self::Daemon => ServiceRole::Daemon,
            Self::SessionSupervisor => ServiceRole::SessionSupervisor,
            Self::ProjectHost => ServiceRole::ProjectHost,
            Self::AssetProcessor => ServiceRole::AssetProcessor,
            Self::RuntimeHost => ServiceRole::RuntimeHost,
            Self::Worker => ServiceRole::Worker,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceEndpointKind {
    WindowsNamedPipe,
    UnixDomainSocket,
    Tcp,
    InProcess,
}

impl ServiceEndpointKind {
    #[must_use]
    pub const fn from_proto(kind: EndpointKind) -> Self {
        match kind {
            EndpointKind::WindowsNamedPipe => Self::WindowsNamedPipe,
            EndpointKind::UnixDomainSocket => Self::UnixDomainSocket,
            EndpointKind::Tcp => Self::Tcp,
            EndpointKind::InProcess => Self::InProcess,
        }
    }

    #[must_use]
    pub const fn to_proto(self) -> EndpointKind {
        match self {
            Self::WindowsNamedPipe => EndpointKind::WindowsNamedPipe,
            Self::UnixDomainSocket => EndpointKind::UnixDomainSocket,
            Self::Tcp => EndpointKind::Tcp,
            Self::InProcess => EndpointKind::InProcess,
        }
    }
}

/// Derive the dedicated observability-control listener from a service endpoint.
#[must_use]
pub fn observability_endpoint(endpoint: Endpoint) -> Endpoint {
    let address = match endpoint.kind {
        EndpointKind::WindowsNamedPipe => format!("{}-observability", endpoint.address),
        EndpointKind::UnixDomainSocket => {
            let path = Path::new(&endpoint.address);
            let file_name = path.file_name().and_then(|name| name.to_str());
            let scoped_name = match (
                path.file_stem().and_then(|stem| stem.to_str()),
                path.extension().and_then(|extension| extension.to_str()),
                file_name,
            ) {
                (Some(stem), Some(extension), _) => format!("{stem}-observability.{extension}"),
                (_, _, Some(file_name)) => format!("{file_name}-observability"),
                _ => format!("{}-observability", endpoint.address),
            };
            let mut scoped = PathBuf::from(&endpoint.address);
            scoped.set_file_name(scoped_name);
            scoped.to_string_lossy().into_owned()
        }
        EndpointKind::Tcp => endpoint
            .address
            .parse::<std::net::SocketAddr>()
            .map_or_else(
                |_| "127.0.0.1:0".to_string(),
                |mut address| {
                    address.set_port(0);
                    address.to_string()
                },
            ),
        EndpointKind::InProcess => endpoint.address,
    };
    Endpoint::new(endpoint.kind, address)
}

/// Derive the portable local lifecycle-control listener.
///
/// This endpoint is intentionally loopback TCP even when the role API is a
/// named pipe or Unix socket: the generic entrypoint owns one portable
/// listener, and capability validation is still mandatory. Publishing an IPC
/// kind the control server cannot bind would make readiness dishonest.
#[must_use]
pub fn service_lifecycle_endpoint(_endpoint: Endpoint) -> Endpoint {
    Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceProcessState {
    Planned,
    Starting,
    Running,
    Exited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCapabilityRecord {
    #[serde(default)]
    pub session: Option<uuid::Uuid>,
    pub service_namespace: String,
    pub service_name: String,
    pub role: SupervisedServiceRole,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub expires_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub token_hash: Vec<u8>,
}

impl ServiceCapabilityRecord {
    #[must_use]
    pub fn from_proto(capability: &Capability) -> Option<Self> {
        Some(Self {
            session: capability.session,
            service_namespace: capability.service.namespace.clone(),
            service_name: capability.service.name.clone(),
            role: SupervisedServiceRole::from_proto(capability.role)?,
            permissions: capability.permissions.clone(),
            audience: capability.audience.clone(),
            expires_unix_ms: capability.expires_unix_ms,
            token_hash: capability.token_hash.clone(),
        })
    }

    #[must_use]
    pub fn to_proto(&self) -> Capability {
        let mut capability = Capability::new(
            ServiceId::new(&self.service_namespace, &self.service_name),
            self.role.to_proto(),
        )
        .with_audience(&self.audience)
        .with_permissions(self.permissions.iter().cloned());
        capability.session = self.session;
        capability.expires_unix_ms = self.expires_unix_ms;
        capability.token_hash.clone_from(&self.token_hash);
        capability
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceReadyRecord {
    pub schema_version: u32,
    pub service_name: String,
    pub role: SupervisedServiceRole,
    pub run: uuid::Uuid,
    pub endpoint_kind: ServiceEndpointKind,
    pub endpoint_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability_endpoint_kind: Option<ServiceEndpointKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability_endpoint_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_endpoint_kind: Option<ServiceEndpointKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_endpoint_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub ready_unix_ms: u128,
}

impl ServiceReadyRecord {
    #[must_use]
    pub fn new(
        service_name: impl Into<String>,
        role: SupervisedServiceRole,
        run: uuid::Uuid,
        endpoint: &Endpoint,
        pid: Option<u32>,
        ready_unix_ms: u128,
    ) -> Self {
        Self {
            schema_version: SERVICE_READY_SCHEMA_VERSION,
            service_name: service_name.into(),
            role,
            run,
            endpoint_kind: ServiceEndpointKind::from_proto(endpoint.kind),
            endpoint_address: endpoint.address.clone(),
            observability_endpoint_kind: None,
            observability_endpoint_address: None,
            lifecycle_endpoint_kind: None,
            lifecycle_endpoint_address: None,
            pid,
            ready_unix_ms,
        }
    }

    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::new(self.endpoint_kind.to_proto(), &self.endpoint_address)
    }

    #[must_use]
    pub fn observability_endpoint(&self) -> Option<Endpoint> {
        Some(Endpoint::new(
            self.observability_endpoint_kind?.to_proto(),
            self.observability_endpoint_address.as_ref()?,
        ))
    }

    #[must_use]
    pub fn lifecycle_endpoint(&self) -> Option<Endpoint> {
        Some(Endpoint::new(
            self.lifecycle_endpoint_kind?.to_proto(),
            self.lifecycle_endpoint_address.as_ref()?,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRecord {
    pub namespace: String,
    pub name: String,
    pub role: SupervisedServiceRole,
    pub run: uuid::Uuid,
    #[serde(default)]
    pub protocol_major: u16,
    #[serde(default)]
    pub protocol_minor: u16,
    #[serde(default)]
    pub protocol_patch: u16,
    pub endpoint_kind: ServiceEndpointKind,
    pub endpoint_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability_endpoint_kind: Option<ServiceEndpointKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability_endpoint_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_endpoint_kind: Option<ServiceEndpointKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_endpoint_address: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ServiceCapabilityRecord>,
}

// A service row's launch label is published observation, not control identity.
impl PartialEq for ServiceRecord {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.name == other.name
            && self.role == other.role
            && self.protocol_major == other.protocol_major
            && self.protocol_minor == other.protocol_minor
            && self.protocol_patch == other.protocol_patch
            && self.endpoint_kind == other.endpoint_kind
            && self.endpoint_address == other.endpoint_address
            && self.observability_endpoint_kind == other.observability_endpoint_kind
            && self.observability_endpoint_address == other.observability_endpoint_address
            && self.lifecycle_endpoint_kind == other.lifecycle_endpoint_kind
            && self.lifecycle_endpoint_address == other.lifecycle_endpoint_address
            && self.capabilities == other.capabilities
    }
}

impl Eq for ServiceRecord {}

impl ServiceRecord {
    #[must_use]
    pub fn from_descriptor(descriptor: &ServiceDescriptor) -> Option<Self> {
        Some(Self {
            namespace: descriptor.id.namespace.clone(),
            name: descriptor.id.name.clone(),
            role: SupervisedServiceRole::from_proto(descriptor.role)?,
            run: descriptor.run,
            protocol_major: descriptor.protocol.major,
            protocol_minor: descriptor.protocol.minor,
            protocol_patch: descriptor.protocol.patch,
            endpoint_kind: ServiceEndpointKind::from_proto(descriptor.endpoint.kind),
            endpoint_address: descriptor.endpoint.address.clone(),
            observability_endpoint_kind: descriptor
                .observability_endpoint
                .as_ref()
                .map(|endpoint| ServiceEndpointKind::from_proto(endpoint.kind)),
            observability_endpoint_address: descriptor
                .observability_endpoint
                .as_ref()
                .map(|endpoint| endpoint.address.clone()),
            lifecycle_endpoint_kind: descriptor
                .lifecycle_endpoint
                .as_ref()
                .map(|endpoint| ServiceEndpointKind::from_proto(endpoint.kind)),
            lifecycle_endpoint_address: descriptor
                .lifecycle_endpoint
                .as_ref()
                .map(|endpoint| endpoint.address.clone()),
            capabilities: descriptor
                .capabilities
                .iter()
                .map(ServiceCapabilityRecord::from_proto)
                .collect::<Option<Vec<_>>>()?,
        })
    }

    #[must_use]
    pub fn to_descriptor(&self) -> ServiceDescriptor {
        ServiceDescriptor {
            id: ServiceId::new(&self.namespace, &self.name),
            role: self.role.to_proto(),
            run: self.run,
            protocol: ProtocolVersion {
                major: self.protocol_major,
                minor: self.protocol_minor,
                patch: self.protocol_patch,
            },
            endpoint: Endpoint::new(self.endpoint_kind.to_proto(), &self.endpoint_address),
            observability_endpoint: self.observability_endpoint(),
            lifecycle_endpoint: self.lifecycle_endpoint(),
            capabilities: self
                .capabilities
                .iter()
                .map(ServiceCapabilityRecord::to_proto)
                .collect(),
        }
    }

    #[must_use]
    pub fn matches(&self, id: &ServiceId, role: ServiceRole) -> bool {
        self.namespace == id.namespace && self.name == id.name && self.role.to_proto() == role
    }

    /// Whether serializing `other` would change this published observation.
    ///
    /// Normal equality intentionally excludes the observational launch label;
    /// publication idempotence is the one boundary that must also preserve it.
    #[must_use]
    pub fn same_durable_publication(&self, other: &Self) -> bool {
        self == other && self.run == other.run
    }

    #[must_use]
    pub fn observability_endpoint(&self) -> Option<Endpoint> {
        Some(Endpoint::new(
            self.observability_endpoint_kind?.to_proto(),
            self.observability_endpoint_address.as_ref()?,
        ))
    }

    #[must_use]
    pub fn lifecycle_endpoint(&self) -> Option<Endpoint> {
        Some(Endpoint::new(
            self.lifecycle_endpoint_kind?.to_proto(),
            self.lifecycle_endpoint_address.as_ref()?,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProcessRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner_id: String,
    #[serde(default, skip_serializing_if = "path_buf_is_empty")]
    pub owner_root: PathBuf,
    pub service_name: String,
    pub role: SupervisedServiceRole,
    /// `UUIDv7` launch label for attribution only; never a process key.
    pub run: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_run: Option<uuid::Uuid>,
    pub endpoint_kind: ServiceEndpointKind,
    pub endpoint_address: String,
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_artifact: Option<ServiceProgramArtifact>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    #[serde(default, skip_serializing_if = "path_buf_is_empty")]
    pub structured_log: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_file: Option<PathBuf>,
    pub state: ServiceProcessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Platform start token for [`Self::pid`], captured while the launcher
    /// still owned the spawned process handle. `pid` alone survives process
    /// death and is handed to unrelated processes, so no liveness question may
    /// be asked without it.
    /// Absent only in manifests written before the token existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_start_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    pub planned_unix_ms: u128,
    pub updated_unix_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_unix_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exited_unix_ms: Option<u128>,
}

// Process equality describes the controllable/persisted process state. The
// current and previous launch labels remain observations and cannot silently
// become liveness or current-selection predicates through whole-row equality.
impl PartialEq for ServiceProcessRecord {
    fn eq(&self, other: &Self) -> bool {
        self.owner_id == other.owner_id
            && self.owner_root == other.owner_root
            && self.service_name == other.service_name
            && self.role == other.role
            && self.endpoint_kind == other.endpoint_kind
            && self.endpoint_address == other.endpoint_address
            && self.program == other.program
            && self.program_artifact == other.program_artifact
            && self.cwd == other.cwd
            && self.args == other.args
            && self.stdout_log == other.stdout_log
            && self.stderr_log == other.stderr_log
            && self.structured_log == other.structured_log
            && self.ready_file == other.ready_file
            && self.state == other.state
            && self.pid == other.pid
            && self.process_start_time == other.process_start_time
            && self.exit_code == other.exit_code
            && self.failure == other.failure
            && self.planned_unix_ms == other.planned_unix_ms
            && self.updated_unix_ms == other.updated_unix_ms
            && self.started_unix_ms == other.started_unix_ms
            && self.exited_unix_ms == other.exited_unix_ms
    }
}

impl Eq for ServiceProcessRecord {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedServiceProcessCleanup {
    NoPid,
    NotRunning {
        pid: u32,
    },
    Terminated {
        pid: u32,
    },
    ImageMismatch {
        pid: u32,
        expected: String,
        actual: String,
    },
    Reused {
        pid: u32,
        recorded_start_time: u64,
        actual_start_time: u64,
    },
    Unattributable {
        pid: u32,
    },
    IdentityBindingUnavailable {
        pid: u32,
        reason: String,
    },
}

impl RecordedServiceProcessCleanup {
    /// Whether this verdict proves that the process named by the durable
    /// record is no longer live.
    ///
    /// A reused pid proves that the recorded process exited even though the
    /// unrelated process now holding that pid was deliberately not touched.
    /// Every refusal verdict preserves the active record so callers cannot
    /// launch a replacement alongside a process that may still be live.
    #[must_use]
    pub const fn proves_recorded_process_gone(&self) -> bool {
        matches!(
            self,
            Self::NotRunning { .. } | Self::Terminated { .. } | Self::Reused { .. }
        )
    }
}

/// Terminate the process a record names, and only that process.
///
/// The identity check runs before either platform path so that a recycled pid
/// is refused rather than killed. Records written before the start token
/// existed are likewise refused: an executable-image match does not prove that
/// the live process is the one this record launched.
///
/// # Errors
///
/// Returns [`ServiceProcessError::Io`] if the record's liveness cannot be
/// assessed or the platform kill fails. A pid that is already gone, recycled,
/// or unattributable is a successful [`RecordedServiceProcessCleanup`] variant,
/// not an error.
pub fn terminate_recorded_service_process(
    process: &ServiceProcessRecord,
) -> Result<RecordedServiceProcessCleanup, ServiceProcessError> {
    match process.process()? {
        RecordedProcess::Unrecorded => return Ok(RecordedServiceProcessCleanup::NoPid),
        RecordedProcess::Exited { process_id } => {
            return Ok(RecordedServiceProcessCleanup::NotRunning { pid: process_id });
        }
        RecordedProcess::Reused {
            recorded,
            actual_start_time,
        } => {
            return Ok(RecordedServiceProcessCleanup::Reused {
                pid: recorded.process_id,
                recorded_start_time: recorded.process_start_time,
                actual_start_time,
            });
        }
        RecordedProcess::Unattributable { process_id } => {
            return Ok(RecordedServiceProcessCleanup::Unattributable { pid: process_id });
        }
        RecordedProcess::Live { .. } => {}
    }
    platform_terminate_recorded_service_process(process)
}

#[cfg(windows)]
fn platform_terminate_recorded_service_process(
    process: &ServiceProcessRecord,
) -> Result<RecordedServiceProcessCleanup, ServiceProcessError> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    };

    let Some(pid) = process.pid else {
        return Ok(RecordedServiceProcessCleanup::NoPid);
    };
    let Ok(handle) = (unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }) else {
        return Ok(RecordedServiceProcessCleanup::NotRunning { pid });
    };

    let Some(recorded_start_time) = process.process_start_time else {
        let _ = unsafe { CloseHandle(handle) };
        return Ok(RecordedServiceProcessCleanup::Unattributable { pid });
    };
    let actual_start_time = match query_windows_process_start_time(handle) {
        Ok(Some(actual_start_time)) => actual_start_time,
        Ok(None) => {
            let _ = unsafe { CloseHandle(handle) };
            return Ok(RecordedServiceProcessCleanup::NotRunning { pid });
        }
        Err(error) => {
            let _ = unsafe { CloseHandle(handle) };
            return Err(error.into());
        }
    };
    if actual_start_time != recorded_start_time {
        let _ = unsafe { CloseHandle(handle) };
        return Ok(RecordedServiceProcessCleanup::Reused {
            pid,
            recorded_start_time,
            actual_start_time,
        });
    }

    let actual = query_windows_process_image(handle).map(PathBuf::from);
    let expected = canonical_process_program(&process.program);
    if let Ok(actual) = actual.as_ref()
        && !windows_paths_match(&expected, actual)
    {
        let _ = unsafe { CloseHandle(handle) };
        return Ok(RecordedServiceProcessCleanup::ImageMismatch {
            pid,
            expected: expected.display().to_string(),
            actual: actual.display().to_string(),
        });
    }

    if let Err(error) = unsafe { TerminateProcess(handle, 1) } {
        let _ = unsafe { CloseHandle(handle) };
        return Err(std::io::Error::other(error).into());
    }
    let wait = unsafe { WaitForSingleObject(handle, 5_000) };
    let _ = unsafe { CloseHandle(handle) };
    if wait == WAIT_TIMEOUT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("timed out waiting for recorded service pid {pid} to terminate"),
        )
        .into());
    }
    Ok(RecordedServiceProcessCleanup::Terminated { pid })
}

#[cfg(windows)]
fn query_windows_process_image(
    handle: windows::Win32::Foundation::HANDLE,
) -> std::io::Result<String> {
    use windows::Win32::System::Threading::{PROCESS_NAME_FORMAT, QueryFullProcessImageNameW};
    use windows::core::PWSTR;

    // One `u32` length feeds both the allocation and the in/out size argument,
    // so neither direction needs a fallible conversion.
    const BUFFER_LEN: u32 = 32_768;

    let mut buffer = vec![0u16; BUFFER_LEN as usize];
    let mut size = BUFFER_LEN;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT::default(),
            PWSTR(buffer.as_mut_ptr()),
            &raw mut size,
        )
    }
    .map_err(std::io::Error::other)?;
    buffer.truncate(size as usize);
    Ok(String::from_utf16_lossy(&buffer))
}

#[cfg(windows)]
fn windows_paths_match(expected: &Path, actual: &Path) -> bool {
    expected
        .to_string_lossy()
        .eq_ignore_ascii_case(&actual.to_string_lossy())
}

#[cfg(target_os = "linux")]
fn platform_terminate_recorded_service_process(
    process: &ServiceProcessRecord,
) -> Result<RecordedServiceProcessCleanup, ServiceProcessError> {
    let Some(pid) = process.pid else {
        return Ok(RecordedServiceProcessCleanup::NoPid);
    };
    let Some(recorded_start_time) = process.process_start_time else {
        return Ok(RecordedServiceProcessCleanup::Unattributable { pid });
    };
    let pidfd = match LinuxPidFd::open(pid) {
        Ok(Some(pidfd)) => pidfd,
        Ok(None) => return Ok(RecordedServiceProcessCleanup::NotRunning { pid }),
        Err(error)
            if error
                .raw_os_error()
                .is_some_and(|code| code == libc::ENOSYS || code == libc::EINVAL) =>
        {
            return Ok(RecordedServiceProcessCleanup::IdentityBindingUnavailable {
                pid,
                reason: error.to_string(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let Some(actual_start_time) = platform_process_start_time(pid)? else {
        return Ok(RecordedServiceProcessCleanup::NotRunning { pid });
    };
    if actual_start_time != recorded_start_time {
        return Ok(RecordedServiceProcessCleanup::Reused {
            pid,
            recorded_start_time,
            actual_start_time,
        });
    }
    let expected = canonical_process_program(&process.program);
    if let Ok(actual) = fs::read_link(format!("/proc/{pid}/exe"))
        && recorded_unix_process_program(&actual) != expected
    {
        return Ok(RecordedServiceProcessCleanup::ImageMismatch {
            pid,
            expected: expected.display().to_string(),
            actual: actual.display().to_string(),
        });
    }
    if !pidfd.send(libc::SIGTERM)? {
        return Ok(RecordedServiceProcessCleanup::NotRunning { pid });
    }
    if !pidfd.wait_until(std::time::Instant::now() + std::time::Duration::from_millis(250))? {
        let _ = pidfd.send(libc::SIGKILL)?;
    }
    Ok(RecordedServiceProcessCleanup::Terminated { pid })
}

#[cfg(target_os = "linux")]
fn recorded_unix_process_program(actual: &Path) -> PathBuf {
    const DELETED_SUFFIX: &str = " (deleted)";

    let text = actual.to_string_lossy();
    text.strip_suffix(DELETED_SUFFIX)
        .map_or_else(|| actual.to_path_buf(), PathBuf::from)
}

#[cfg(windows)]
struct WindowsProcessExitHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsProcessExitHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is the sole owner of the successfully opened
        // process handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
// SAFETY: ownership is unique and Windows process handles are kernel objects
// whose wait/close operations are valid from any thread. The wrapper is never
// cloned, so moving it transfers that sole close obligation.
unsafe impl Send for WindowsProcessExitHandle {}

/// A manual-reset event shared by the lifecycle owner and its exact-wait
/// worker. Windows permits concurrent signalling and waiting on this kernel
/// object; the Arc ensures it is closed only after both sides release it.
#[cfg(windows)]
struct WindowsExitWaitCancellationHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsExitWaitCancellationHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is the sole owner of the successfully created
        // event handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
// SAFETY: Windows manual-reset events are thread-safe kernel objects. The
// event remains open through the Arc while one thread waits and another sets
// it to cancel that wait.
unsafe impl Send for WindowsExitWaitCancellationHandle {}

#[cfg(windows)]
// SAFETY: concurrent `WaitForMultipleObjects` and `SetEvent` are the native
// synchronization contract for this object; no mutable Rust state is shared.
unsafe impl Sync for WindowsExitWaitCancellationHandle {}

#[cfg(windows)]
fn platform_bind_process_exit_waiter(
    identity: ProcessIdentity,
) -> Result<ProcessExitWaiter, ServiceProcessError> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            identity.process_id,
        )
    }
    .map_err(|error| ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: error.to_string(),
    })?;
    let handle = WindowsProcessExitHandle(handle);
    let actual_start_time = query_windows_process_start_time(handle.0)?.ok_or_else(|| {
        ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: "process exited while binding its wait handle".to_string(),
        }
    })?;
    if actual_start_time != identity.process_start_time {
        return Err(ServiceProcessError::ProcessExitIdentityMismatch {
            identity,
            actual_start_time,
        });
    }
    Ok(ProcessExitWaiter {
        identity,
        binding: ProcessExitBinding::Windows(handle),
    })
}

#[cfg(windows)]
fn platform_wait_for_process_exit(
    waiter: ProcessExitWaiter,
) -> Result<ProcessIdentity, ServiceProcessError> {
    use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};

    let ProcessExitBinding::Windows(handle) = waiter.binding;
    let wait = unsafe { WaitForSingleObject(handle.0, INFINITE) };
    // The wrapper owns and closes the exact handle after this wait returns.
    drop(handle);
    if wait == WAIT_OBJECT_0 {
        return Ok(waiter.identity);
    }
    let reason = if wait == WAIT_FAILED {
        std::io::Error::last_os_error().to_string()
    } else {
        format!("unexpected Windows process wait status {wait:?}")
    };
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason,
    })
}

#[cfg(windows)]
fn platform_new_process_exit_cancellation(
    identity: ProcessIdentity,
) -> Result<ProcessExitCancellation, ServiceProcessError> {
    use windows::Win32::System::Threading::CreateEventW;

    // SAFETY: no pointers are passed; the returned event handle is uniquely
    // owned by the Arc-backed wrapper below.
    let event = unsafe { CreateEventW(None, true, false, None) }.map_err(|error| {
        ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: error.to_string(),
        }
    })?;
    Ok(ProcessExitCancellation {
        event: Arc::new(WindowsExitWaitCancellationHandle(event)),
    })
}

#[cfg(windows)]
fn platform_cancel_process_exit_wait(
    cancellation: &ProcessExitCancellation,
    identity: ProcessIdentity,
) -> Result<(), ServiceProcessError> {
    use windows::Win32::System::Threading::SetEvent;

    // SAFETY: the Arc keeps the manual-reset event live for this call.
    unsafe { SetEvent(cancellation.event.0) }.map_err(|error| {
        ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: error.to_string(),
        }
    })
}

#[cfg(windows)]
fn platform_wait_for_process_exit_or_cancel(
    waiter: ProcessExitWaiter,
    cancellation: &ProcessExitCancellation,
) -> Result<ProcessExitWaitOutcome, ServiceProcessError> {
    use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};

    let ProcessExitBinding::Windows(handle) = waiter.binding;
    let handles = [handle.0, cancellation.event.0];
    // SAFETY: both handles remain valid for this call. The process handle is
    // exclusively owned by `handle`; the cancellation event is Arc-owned.
    let wait = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
    drop(handle);
    if wait == WAIT_OBJECT_0 {
        return Ok(ProcessExitWaitOutcome::Exited(waiter.identity));
    }
    if wait.0 == WAIT_OBJECT_0.0 + 1 {
        return Ok(ProcessExitWaitOutcome::Cancelled);
    }
    let reason = if wait == WAIT_FAILED {
        std::io::Error::last_os_error().to_string()
    } else {
        format!("unexpected Windows process/cancellation wait status {wait:?}")
    };
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason,
    })
}

#[cfg(target_os = "linux")]
fn platform_bind_process_exit_waiter(
    identity: ProcessIdentity,
) -> Result<ProcessExitWaiter, ServiceProcessError> {
    let pidfd = LinuxPidFd::open(identity.process_id)
        .map_err(|error| ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: error.to_string(),
        })?
        .ok_or_else(|| ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: "process exited while binding its pidfd".to_string(),
        })?;
    let actual_start_time = platform_process_start_time(identity.process_id)?.ok_or_else(|| {
        ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: "process exited while verifying its pidfd".to_string(),
        }
    })?;
    if actual_start_time != identity.process_start_time {
        return Err(ServiceProcessError::ProcessExitIdentityMismatch {
            identity,
            actual_start_time,
        });
    }
    Ok(ProcessExitWaiter {
        identity,
        binding: ProcessExitBinding::Linux(pidfd),
    })
}

#[cfg(target_os = "linux")]
fn platform_wait_for_process_exit(
    waiter: ProcessExitWaiter,
) -> Result<ProcessIdentity, ServiceProcessError> {
    let ProcessExitBinding::Linux(pidfd) = waiter.binding;
    let mut event = libc::pollfd {
        fd: pidfd.0,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut event, 1, -1) };
        if result > 0 {
            return Ok(waiter.identity);
        }
        if result == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(ServiceProcessError::ProcessExitBindingUnavailable {
            identity: waiter.identity,
            reason: error.to_string(),
        });
    }
}

#[cfg(target_os = "linux")]
fn platform_new_process_exit_cancellation(
    identity: ProcessIdentity,
) -> Result<ProcessExitCancellation, ServiceProcessError> {
    LinuxExitWaitCancellationFd::new()
        .map(|event| ProcessExitCancellation {
            event: Arc::new(event),
        })
        .map_err(|error| ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: error.to_string(),
        })
}

#[cfg(target_os = "linux")]
fn platform_cancel_process_exit_wait(
    cancellation: &ProcessExitCancellation,
    identity: ProcessIdentity,
) -> Result<(), ServiceProcessError> {
    cancellation.event.signal().map_err(|error| {
        ServiceProcessError::ProcessExitBindingUnavailable {
            identity,
            reason: error.to_string(),
        }
    })
}

#[cfg(target_os = "linux")]
fn platform_wait_for_process_exit_or_cancel(
    waiter: ProcessExitWaiter,
    cancellation: &ProcessExitCancellation,
) -> Result<ProcessExitWaitOutcome, ServiceProcessError> {
    let ProcessExitBinding::Linux(pidfd) = waiter.binding;
    let mut events = [
        libc::pollfd {
            fd: pidfd.0,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: cancellation.event.fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        let result = unsafe { libc::poll(events.as_mut_ptr(), events.len() as libc::nfds_t, -1) };
        if result > 0 {
            // Cancellation suppresses observation even if process exit became
            // ready concurrently, so a retired identity cannot publish a
            // stale transition into a later supervision operation.
            if events[1].revents & libc::POLLIN != 0 {
                return Ok(ProcessExitWaitOutcome::Cancelled);
            }
            if events[0].revents & libc::POLLIN != 0 {
                return Ok(ProcessExitWaitOutcome::Exited(waiter.identity));
            }
            continue;
        }
        if result == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(ServiceProcessError::ProcessExitBindingUnavailable {
            identity: waiter.identity,
            reason: error.to_string(),
        });
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_bind_process_exit_waiter(
    identity: ProcessIdentity,
) -> Result<ProcessExitWaiter, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this Unix target has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_wait_for_process_exit(
    waiter: ProcessExitWaiter,
) -> Result<ProcessIdentity, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason: "this Unix target has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_new_process_exit_cancellation(
    identity: ProcessIdentity,
) -> Result<ProcessExitCancellation, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this Unix target has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_cancel_process_exit_wait(
    _cancellation: &ProcessExitCancellation,
    identity: ProcessIdentity,
) -> Result<(), ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this Unix target has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_wait_for_process_exit_or_cancel(
    waiter: ProcessExitWaiter,
    _cancellation: &ProcessExitCancellation,
) -> Result<ProcessExitWaitOutcome, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason: "this Unix target has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_bind_process_exit_waiter(
    identity: ProcessIdentity,
) -> Result<ProcessExitWaiter, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this platform has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_wait_for_process_exit(
    waiter: ProcessExitWaiter,
) -> Result<ProcessIdentity, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason: "this platform has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_new_process_exit_cancellation(
    identity: ProcessIdentity,
) -> Result<ProcessExitCancellation, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this platform has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_cancel_process_exit_wait(
    _cancellation: &ProcessExitCancellation,
    identity: ProcessIdentity,
) -> Result<(), ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity,
        reason: "this platform has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_wait_for_process_exit_or_cancel(
    waiter: ProcessExitWaiter,
    _cancellation: &ProcessExitCancellation,
) -> Result<ProcessExitWaitOutcome, ServiceProcessError> {
    Err(ServiceProcessError::ProcessExitBindingUnavailable {
        identity: waiter.identity,
        reason: "this platform has no identity-bound process-exit wait".to_string(),
    })
}

#[cfg(target_os = "linux")]
struct LinuxPidFd(std::os::fd::RawFd);

#[cfg(target_os = "linux")]
impl LinuxPidFd {
    fn open(pid: u32) -> std::io::Result<Option<Self>> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as std::os::fd::RawFd;
        if fd >= 0 {
            return Ok(Some(Self(fd)));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        }
    }

    fn send(&self, signal: libc::c_int) -> std::io::Result<bool> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0,
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(false)
        } else {
            Err(error)
        }
    }

    fn wait_until(&self, deadline: std::time::Instant) -> std::io::Result<bool> {
        let mut event = libc::pollfd {
            fd: self.0,
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as libc::c_int;
            let result = unsafe { libc::poll(&mut event, 1, timeout_ms.max(1)) };
            if result > 0 {
                return Ok(true);
            }
            if result == 0 {
                return Ok(false);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxPidFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

#[cfg(target_os = "linux")]
struct LinuxExitWaitCancellationFd {
    fd: std::os::fd::RawFd,
    signalled: AtomicBool,
}

#[cfg(target_os = "linux")]
impl LinuxExitWaitCancellationFd {
    fn new() -> std::io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(Self {
                fd,
                signalled: AtomicBool::new(false),
            })
        }
    }

    fn signal(&self) -> std::io::Result<()> {
        if self
            .signalled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let value = 1_u64.to_ne_bytes();
        loop {
            let written =
                unsafe { libc::write(self.fd, value.as_ptr().cast::<libc::c_void>(), value.len()) };
            if written == value.len() as isize {
                return Ok(());
            }
            if written < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            return Err(std::io::Error::last_os_error());
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxExitWaitCancellationFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_terminate_recorded_service_process(
    process: &ServiceProcessRecord,
) -> Result<RecordedServiceProcessCleanup, ServiceProcessError> {
    Ok(match process.pid {
        Some(pid) => RecordedServiceProcessCleanup::IdentityBindingUnavailable {
            pid,
            reason: "this Unix target has no identity-bound termination primitive".to_string(),
        },
        None => RecordedServiceProcessCleanup::NoPid,
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_terminate_recorded_service_process(
    process: &ServiceProcessRecord,
) -> Result<RecordedServiceProcessCleanup, ServiceProcessError> {
    Ok(if let Some(pid) = process.pid {
        RecordedServiceProcessCleanup::IdentityBindingUnavailable {
            pid,
            reason: "this platform has no identity-bound termination primitive".to_string(),
        }
    } else {
        RecordedServiceProcessCleanup::NoPid
    })
}

fn canonical_process_program(program: &str) -> PathBuf {
    let path = Path::new(program);
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ServiceProcessRecord {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn planned(
        service_name: impl Into<String>,
        role: SupervisedServiceRole,
        run: uuid::Uuid,
        endpoint: &Endpoint,
        program: impl Into<String>,
        cwd: PathBuf,
        args: Vec<String>,
        stdout_log: PathBuf,
        stderr_log: PathBuf,
        structured_log: PathBuf,
        ready_file: Option<PathBuf>,
        now_unix_ms: u128,
    ) -> Self {
        Self {
            owner_id: String::new(),
            owner_root: PathBuf::new(),
            service_name: service_name.into(),
            role,
            run,
            previous_run: None,
            endpoint_kind: ServiceEndpointKind::from_proto(endpoint.kind),
            endpoint_address: endpoint.address.clone(),
            program: program.into(),
            program_artifact: None,
            cwd,
            args,
            stdout_log,
            stderr_log,
            structured_log,
            ready_file,
            state: ServiceProcessState::Planned,
            pid: None,
            process_start_time: None,
            exit_code: None,
            failure: None,
            planned_unix_ms: now_unix_ms,
            updated_unix_ms: now_unix_ms,
            started_unix_ms: None,
            exited_unix_ms: None,
        }
    }

    /// Record the identity of the executable this record will launch.
    ///
    /// # Errors
    ///
    /// Returns any [`std::io::Error`] [`ServiceProgramArtifact::capture`]
    /// returns for `self.program` — typically the binary being absent or not a
    /// file.
    pub fn capture_program_artifact(&mut self) -> std::io::Result<ServiceProgramArtifact> {
        let artifact = ServiceProgramArtifact::capture(Path::new(&self.program))?;
        self.program_artifact = Some(artifact);
        Ok(artifact)
    }

    /// Has the executable on disk changed since it was recorded?
    ///
    /// # Errors
    ///
    /// Returns any [`std::io::Error`] [`ServiceProgramArtifact::capture`]
    /// returns for `self.program`. A record with no captured artifact is
    /// `Ok(false)`.
    pub fn program_artifact_is_current(&self) -> std::io::Result<bool> {
        let Some(recorded) = self.program_artifact else {
            return Ok(false);
        };
        Ok(recorded == ServiceProgramArtifact::capture(Path::new(&self.program))?)
    }

    /// Persist the identity captured while the process handle was still owned
    /// by the launcher. No later PID lookup is allowed to replace that fact.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::InvalidProcessIdentity`] if `identity`
    /// has a zero pid or start token, which would record an unattributable
    /// process.
    pub fn mark_running(
        &mut self,
        identity: ProcessIdentity,
        now_unix_ms: u128,
    ) -> Result<(), ServiceProcessError> {
        if identity.process_id == 0 || identity.process_start_time == 0 {
            return Err(ServiceProcessError::InvalidProcessIdentity { identity });
        }
        self.state = ServiceProcessState::Running;
        self.pid = Some(identity.process_id);
        self.process_start_time = Some(identity.process_start_time);
        self.exit_code = None;
        self.failure = None;
        self.started_unix_ms = Some(now_unix_ms);
        self.exited_unix_ms = None;
        self.updated_unix_ms = now_unix_ms;
        Ok(())
    }

    /// Is the process this record names still the process that was launched?
    ///
    /// # Errors
    ///
    /// Returns any [`std::io::Error`] [`RecordedProcess::assess`] returns.
    pub fn process(&self) -> std::io::Result<RecordedProcess> {
        RecordedProcess::assess(self.pid, self.process_start_time)
    }

    pub fn mark_starting(&mut self, now_unix_ms: u128) {
        self.state = ServiceProcessState::Starting;
        self.pid = None;
        self.process_start_time = None;
        self.exit_code = None;
        self.failure = None;
        self.started_unix_ms = None;
        self.exited_unix_ms = None;
        self.updated_unix_ms = now_unix_ms;
    }

    pub fn mark_exited(
        &mut self,
        exit_code: Option<i32>,
        failure: Option<String>,
        now_unix_ms: u128,
    ) {
        self.state = if failure.is_some() || exit_code.is_some_and(|code| code != 0) {
            ServiceProcessState::Failed
        } else {
            ServiceProcessState::Exited
        };
        self.pid = None;
        self.process_start_time = None;
        self.exit_code = exit_code;
        self.failure = failure;
        self.exited_unix_ms = Some(now_unix_ms);
        self.updated_unix_ms = now_unix_ms;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedServiceProcess {
    pub service_name: String,
    pub role: SupervisedServiceRole,
    pub run: uuid::Uuid,
    pub identity: ProcessIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServiceProcessKey {
    pub service_name: String,
    pub role: SupervisedServiceRole,
}

impl ServiceProcessKey {
    #[must_use]
    pub fn new(service_name: impl Into<String>, role: SupervisedServiceRole) -> Self {
        Self {
            service_name: service_name.into(),
            role,
        }
    }

    #[must_use]
    pub fn from_proto(service_name: impl Into<String>, role: ServiceRole) -> Option<Self> {
        Some(Self::new(
            service_name,
            SupervisedServiceRole::from_proto(role)?,
        ))
    }

    #[must_use]
    pub fn from_process(process: &ServiceProcessRecord) -> Self {
        Self {
            service_name: process.service_name.clone(),
            role: process.role,
        }
    }

    #[must_use]
    pub fn from_spawned(process: &SpawnedServiceProcess) -> Self {
        Self {
            service_name: process.service_name.clone(),
            role: process.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProcessExit {
    pub service_name: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

pub trait ServiceProcessLauncher {
    type Error;

    /// Start the process `process` describes and take ownership of its handle.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the program cannot be spawned or its launch
    /// identity cannot be captured while the handle is still owned.
    fn spawn(&self, process: &ServiceProcessRecord) -> Result<SpawnedServiceProcess, Self::Error>;

    fn is_tracking(&self, process: &ServiceProcessKey) -> bool;

    /// Report an exit that has already happened, without blocking.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the launcher's process table is unavailable or
    /// the platform status probe fails. A process that is still running is
    /// `Ok(None)`.
    fn try_wait(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, Self::Error>;

    /// Stop the process, returning its exit if the launcher observed one.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the launcher's process table is unavailable or
    /// the platform kill fails. A process the launcher does not track is
    /// `Ok(None)`.
    fn terminate(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, Self::Error>;
}

#[derive(Debug, Error)]
pub enum ServiceProcessError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to capture the identity of process {pid}: {source}")]
    ProcessIdentityCapture {
        pid: u32,
        #[source]
        source: std::io::Error,
    },

    #[error("process {pid} exited before its start identity could be captured")]
    ProcessIdentityUnavailable { pid: u32 },

    #[error("process identity must have non-zero pid and start token, got {identity:?}")]
    InvalidProcessIdentity { identity: ProcessIdentity },

    #[error("cannot bind an exit event for process {identity:?}: {reason}")]
    ProcessExitBindingUnavailable {
        identity: ProcessIdentity,
        reason: String,
    },

    #[error(
        "cannot bind an exit event for recycled pid {identity:?}; its current start token is {actual_start_time}"
    )]
    ProcessExitIdentityMismatch {
        identity: ProcessIdentity,
        actual_start_time: u64,
    },

    #[error("failed to spawn service `{service}` with program `{program}`: {source}")]
    SpawnFailed {
        service: String,
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to watch service readiness directory `{path}`: {reason}")]
    ReadyWatch { path: PathBuf, reason: String },

    #[error("service lifecycle event source closed before the supervised operation completed")]
    LifecycleEventSourceClosed,

    #[error("service lifecycle worker registry was poisoned")]
    LifecycleWorkerRegistryPoisoned,

    #[error("service lifecycle {worker} worker panicked")]
    LifecycleWorkerPanicked { worker: &'static str },
}

pub struct ServiceReadyFileWatcher {
    _watcher: RecommendedWatcher,
    events: std::sync::mpsc::Receiver<ReadyWatchSignal>,
    wake: mpsc::Sender<ReadyWatchSignal>,
}

enum ReadyWatchSignal {
    Changed,
    Close,
}

impl ServiceReadyFileWatcher {
    /// Watch the parent directories of `paths` for readiness-file changes.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::ReadyWatch`] if the platform watcher
    /// cannot be created or a directory cannot be watched. Paths with no parent
    /// directory to watch yield `Ok(None)`.
    pub fn new<'a>(
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Option<Self>, ServiceProcessError> {
        let directories = paths
            .into_iter()
            .filter_map(Path::parent)
            .map(Path::to_path_buf)
            .collect::<std::collections::BTreeSet<_>>();
        if directories.is_empty() {
            return Ok(None);
        }
        let (sender, events) = std::sync::mpsc::channel();
        let event_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    let _ = event_sender.send(ReadyWatchSignal::Changed);
                }
            })
            .map_err(|error| ServiceProcessError::ReadyWatch {
                path: directories.iter().next().cloned().unwrap_or_default(),
                reason: error.to_string(),
            })?;
        for directory in &directories {
            watcher
                .watch(directory, RecursiveMode::NonRecursive)
                .map_err(|error| ServiceProcessError::ReadyWatch {
                    path: directory.clone(),
                    reason: error.to_string(),
                })?;
        }
        Ok(Some(Self {
            _watcher: watcher,
            events,
            wake: sender,
        }))
    }

    #[must_use]
    pub fn wait(&self) -> bool {
        matches!(self.events.recv(), Ok(ReadyWatchSignal::Changed))
    }

    fn close_handle(&self) -> mpsc::Sender<ReadyWatchSignal> {
        self.wake.clone()
    }
}

/// The two OS facts a service supervisor can wait for while startup is armed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceLifecycleEvent {
    ReadyFileChanged,
    ProcessExited(ProcessIdentity),
    ProcessExitWaitFailed {
        identity: ProcessIdentity,
        reason: String,
    },
}

impl ServiceLifecycleEvent {
    fn is_exit_for(&self, identity: ProcessIdentity) -> bool {
        matches!(
            self,
            Self::ProcessExited(actual)
                | Self::ProcessExitWaitFailed {
                    identity: actual,
                    ..
                } if *actual == identity
        )
    }
}

/// A single event source for readiness-file changes and exact child exits.
///
/// Every child wait is bound before its worker starts, and the ready watcher is
/// retained by its forwarding worker. The consumer therefore cannot miss an
/// exit merely because no readiness write follows it. `wait_until` is for the
/// one externally meaningful startup/grace deadline, not a retry cadence.
pub struct ServiceLifecycleEvents {
    sender: channel::Sender<ServiceLifecycleEvent>,
    events: channel::Receiver<ServiceLifecycleEvent>,
    closing: AtomicBool,
    exit_workers: std::sync::Mutex<BTreeMap<ProcessIdentity, ProcessExitSubscription>>,
}

/// Owned cancellation and join handle for one OS-bound exact-exit wait.
///
/// Dropping a join handle detaches a thread, so lifecycle ownership never
/// exposes that operation. A caller that no longer needs an observation must
/// cancel this native wait first and then join it.
struct ProcessExitSubscription {
    cancellation: ProcessExitCancellation,
    worker: thread::JoinHandle<()>,
}

impl std::fmt::Debug for ServiceLifecycleEvents {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceLifecycleEvents")
            .finish_non_exhaustive()
    }
}

/// One startup-only ready-file subscription on a permanent lifecycle hub.
///
/// The subscription owns both the OS watcher and its forwarding worker. It
/// must be closed when startup reaches a terminal outcome; closing wakes and
/// joins the worker without requiring a later filesystem event.
pub struct ServiceReadySubscription {
    wake: Option<mpsc::Sender<ReadyWatchSignal>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ServiceReadySubscription {
    /// Close the watch and join its forwarding worker.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleWorkerPanicked`] if the
    /// ready-file worker panicked, or
    /// [`ServiceProcessError::LifecycleEventSourceClosed`] if the wake channel
    /// was already gone. Calling this twice is not an error.
    pub fn finish(&mut self) -> Result<(), ServiceProcessError> {
        let wake_result = self.wake.take().map_or(Ok(()), |wake| {
            wake.send(ReadyWatchSignal::Close)
                .map_err(|_| ServiceProcessError::LifecycleEventSourceClosed)
        });
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| ServiceProcessError::LifecycleWorkerPanicked {
                    worker: "ready-file",
                })?;
        }
        wake_result
    }
}

impl Drop for ServiceReadySubscription {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl Default for ServiceLifecycleEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceLifecycleEvents {
    #[must_use]
    pub fn new() -> Self {
        let (sender, events) = channel::unbounded();
        Self {
            sender,
            events,
            closing: AtomicBool::new(false),
            exit_workers: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Attach an ephemeral readiness subscription to this permanent hub.
    /// Process-exit workers are never replaced by this operation.
    ///
    /// # Errors
    ///
    /// Returns any error [`ServiceReadyFileWatcher::new`] returns — in practice
    /// [`ServiceProcessError::ReadyWatch`] when the readiness directories
    /// cannot be watched.
    pub fn subscribe_ready<'a>(
        &self,
        ready_paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<ServiceReadySubscription, ServiceProcessError> {
        if let Some(watcher) = ServiceReadyFileWatcher::new(ready_paths)? {
            let wake = watcher.close_handle();
            let sender = self.sender.clone();
            let worker = thread::spawn(move || {
                while watcher.wait() {
                    if sender
                        .send(ServiceLifecycleEvent::ReadyFileChanged)
                        .is_err()
                    {
                        return;
                    }
                }
            });
            return Ok(ServiceReadySubscription {
                wake: Some(wake),
                worker: Some(worker),
            });
        }
        Ok(ServiceReadySubscription {
            wake: None,
            worker: None,
        })
    }

    /// Start observing `identity` for exit. Re-adding a tracked identity is a
    /// no-op.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleEventSourceClosed`] if the hub
    /// is shutting down, [`ServiceProcessError::LifecycleWorkerRegistryPoisoned`]
    /// if the worker registry lock is poisoned, or any error
    /// [`ProcessExitWaiter::bind`] returns for `identity`.
    pub fn add_identity(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(ServiceProcessError::LifecycleEventSourceClosed);
        }
        let mut workers = self
            .exit_workers
            .lock()
            .map_err(|_| ServiceProcessError::LifecycleWorkerRegistryPoisoned)?;
        if self.closing.load(Ordering::Acquire) {
            return Err(ServiceProcessError::LifecycleEventSourceClosed);
        }
        if workers.contains_key(&identity) {
            return Ok(());
        }
        let waiter = ProcessExitWaiter::bind(identity)?;
        let cancellation = ProcessExitCancellation::new(identity)?;
        let sender = self.sender.clone();
        let worker_cancellation = cancellation.clone();
        workers.insert(
            identity,
            ProcessExitSubscription {
                cancellation,
                worker: thread::spawn(move || {
                    let event = match waiter.wait_or_cancel(&worker_cancellation) {
                        Err(error) => ServiceLifecycleEvent::ProcessExitWaitFailed {
                            identity,
                            reason: error.to_string(),
                        },
                        Ok(ProcessExitWaitOutcome::Exited(identity)) => {
                            ServiceLifecycleEvent::ProcessExited(identity)
                        }
                        Ok(ProcessExitWaitOutcome::Cancelled) => return,
                    };
                    let _ = sender.send(event);
                }),
            },
        );
        // The registry lock guards a check-then-insert, so it has to span the
        // `contains_key` above through this insert; release it before returning.
        drop(workers);
        Ok(())
    }

    /// Retire the worker for an identity whose exit this hub already delivered.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleWorkerRegistryPoisoned`] if the
    /// worker registry lock is poisoned, or
    /// [`ServiceProcessError::LifecycleWorkerPanicked`] if the exit worker
    /// panicked.
    pub fn consume_exit(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        self.join_exit_worker(identity)
    }

    /// Stop observing one still-live identity and synchronously join its
    /// worker. This never terminates the child; it only releases this hub's
    /// exact process handle after waking the OS wait.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleWorkerRegistryPoisoned`] if the
    /// worker registry lock is poisoned, [`ServiceProcessError::Io`] if the
    /// platform cancellation fails, or
    /// [`ServiceProcessError::LifecycleWorkerPanicked`] if the exit worker
    /// panicked.
    pub fn cancel_exit_wait(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        self.cancel_exit_workers([identity])
    }

    /// Stop observing every remaining identity and synchronously join all
    /// workers. Use this only when the enclosing owner is leaving its
    /// supervision loop; child-process termination remains the launcher's
    /// responsibility.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleWorkerRegistryPoisoned`] if the
    /// worker registry lock is poisoned, [`ServiceProcessError::Io`] if a
    /// platform cancellation fails, or
    /// [`ServiceProcessError::LifecycleWorkerPanicked`] if an exit worker
    /// panicked.
    pub fn cancel_all_exit_waits(&self) -> Result<(), ServiceProcessError> {
        // This is terminal for the hub. Mark it before taking the registry so
        // a racing startup cannot attach an unobserved waiter afterward.
        self.closing.store(true, Ordering::Release);
        let subscriptions = {
            let mut workers = self
                .exit_workers
                .lock()
                .map_err(|_| ServiceProcessError::LifecycleWorkerRegistryPoisoned)?;
            for (identity, subscription) in workers.iter() {
                subscription.cancellation.cancel(*identity)?;
            }
            std::mem::take(&mut *workers)
                .into_iter()
                .collect::<Vec<_>>()
        };
        self.join_cancelled_exit_workers(subscriptions)
    }

    /// Retire an identity after its owner synchronously reaped it outside the
    /// lifecycle receive path. This joins the exact wait worker and removes
    /// only its now-stale notification. Unrelated transitions are requeued to
    /// the hub source, so every consumer (including a `select!` receiver) sees
    /// one stream rather than a hidden side queue. Cross-identity ordering is
    /// observational; identity attribution remains exact.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleWorkerRegistryPoisoned`] if the
    /// worker registry lock is poisoned, [`ServiceProcessError::Io`] if the
    /// platform cancellation fails, or
    /// [`ServiceProcessError::LifecycleWorkerPanicked`] if the exit worker
    /// panicked.
    pub fn retire_exit(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        self.cancel_exit_workers([identity])
    }

    fn join_exit_worker(&self, identity: ProcessIdentity) -> Result<(), ServiceProcessError> {
        let worker = self
            .exit_workers
            .lock()
            .map_err(|_| ServiceProcessError::LifecycleWorkerRegistryPoisoned)?
            .remove(&identity);
        if let Some(subscription) = worker {
            subscription.worker.join().map_err(|_| {
                ServiceProcessError::LifecycleWorkerPanicked {
                    worker: "process-exit",
                }
            })?;
        }
        Ok(())
    }

    fn cancel_exit_workers(
        &self,
        identities: impl IntoIterator<Item = ProcessIdentity>,
    ) -> Result<(), ServiceProcessError> {
        let identities = identities.into_iter().collect::<Vec<_>>();
        let subscriptions = {
            let mut workers = self
                .exit_workers
                .lock()
                .map_err(|_| ServiceProcessError::LifecycleWorkerRegistryPoisoned)?;
            for identity in &identities {
                if let Some(subscription) = workers.get(identity) {
                    subscription.cancellation.cancel(*identity)?;
                }
            }
            identities
                .iter()
                .filter_map(|identity| workers.remove(identity).map(|worker| (*identity, worker)))
                .collect::<Vec<_>>()
        };
        self.join_cancelled_exit_workers(subscriptions)
    }

    fn join_cancelled_exit_workers(
        &self,
        subscriptions: Vec<(ProcessIdentity, ProcessExitSubscription)>,
    ) -> Result<(), ServiceProcessError> {
        let identities = subscriptions
            .iter()
            .map(|(identity, _)| *identity)
            .collect::<Vec<_>>();
        for (_, subscription) in subscriptions {
            subscription.worker.join().map_err(|_| {
                ServiceProcessError::LifecycleWorkerPanicked {
                    worker: "process-exit",
                }
            })?;
        }
        self.discard_exit_events(&identities)
    }

    fn discard_exit_events(
        &self,
        identities: &[ProcessIdentity],
    ) -> Result<(), ServiceProcessError> {
        let mut unrelated = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if !identities
                .iter()
                .any(|identity| event.is_exit_for(*identity))
            {
                unrelated.push(event);
            }
        }
        for event in unrelated {
            self.sender
                .send(event)
                .map_err(|_| ServiceProcessError::LifecycleEventSourceClosed)?;
        }
        Ok(())
    }

    /// Wait for an OS event, or return `Ok(None)` when the one armed deadline
    /// expires. A disconnected event source is a failed binding, never a cue
    /// to resume by polling.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleEventSourceClosed`] if every
    /// sender has been dropped. Reaching `deadline` is `Ok(None)`.
    pub fn wait_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, ServiceProcessError> {
        match self.events.recv_deadline(deadline) {
            Ok(event) => Ok(Some(event)),
            Err(channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(channel::RecvTimeoutError::Disconnected) => {
                Err(ServiceProcessError::LifecycleEventSourceClosed)
            }
        }
    }

    /// Wait for one identity-bound exit without discarding unrelated lifecycle
    /// transitions. Deferred transitions are returned to the hub before this
    /// call returns; cross-identity order is observational, but attribution is
    /// never widened or lost.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleEventSourceClosed`] if every
    /// sender has been dropped, or if the deferred unrelated transitions cannot
    /// be returned to the hub. Reaching `deadline` is `Ok(None)`.
    pub fn wait_for_exit_until(
        &self,
        identity: ProcessIdentity,
        deadline: std::time::Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, ServiceProcessError> {
        let mut deferred = Vec::new();
        let result = loop {
            match self.events.recv_deadline(deadline) {
                Ok(event) if event.is_exit_for(identity) => break Ok(Some(event)),
                Ok(event) => deferred.push(event),
                Err(channel::RecvTimeoutError::Timeout) => break Ok(None),
                Err(channel::RecvTimeoutError::Disconnected) => {
                    break Err(ServiceProcessError::LifecycleEventSourceClosed);
                }
            }
        };
        for event in deferred {
            self.sender
                .send(event)
                .map_err(|_| ServiceProcessError::LifecycleEventSourceClosed)?;
        }
        result
    }

    /// Wait indefinitely for the next lifecycle transition after startup.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceProcessError::LifecycleEventSourceClosed`] if every
    /// sender has been dropped.
    pub fn wait(&self) -> Result<ServiceLifecycleEvent, ServiceProcessError> {
        self.events
            .recv()
            .map_err(|_| ServiceProcessError::LifecycleEventSourceClosed)
    }

    #[must_use]
    pub const fn receiver(&self) -> &channel::Receiver<ServiceLifecycleEvent> {
        &self.events
    }

    #[must_use]
    pub fn receiver_clone(&self) -> channel::Receiver<ServiceLifecycleEvent> {
        self.events.clone()
    }
}

impl Drop for ServiceLifecycleEvents {
    fn drop(&mut self) {
        // The public cancellation methods are the fallible path. Drop repeats
        // their native wake-and-join ownership as a final leak barrier.
        let workers = match self.exit_workers.get_mut() {
            Ok(workers) => workers,
            Err(poisoned) => poisoned.into_inner(),
        };
        let subscriptions = std::mem::take(workers).into_iter().collect::<Vec<_>>();
        for (identity, subscription) in &subscriptions {
            let _ = subscription.cancellation.cancel(*identity);
        }
        for (_, subscription) in subscriptions {
            let _ = subscription.worker.join();
        }
    }
}

pub struct StdServiceProcessLauncher {
    children: RefCell<BTreeMap<ServiceProcessKey, OwnedSynchronousChild>>,
    environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for StdServiceProcessLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdServiceProcessLauncher")
            .field(
                "children",
                &self.children.borrow().keys().collect::<Vec<_>>(),
            )
            .field("environment", &self.environment)
            .finish()
    }
}

impl Default for StdServiceProcessLauncher {
    fn default() -> Self {
        Self::new()
    }
}

impl StdServiceProcessLauncher {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: RefCell::new(BTreeMap::new()),
            environment: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_environment_var(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub const fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

/// Rotate one stable service log at planning time.
///
/// The current file becomes `<stem>.previous.<extension>` and replaces the
/// prior predecessor. No launch identity participates in the path.
///
/// # Errors
///
/// Returns [`ServiceProcessError::Io`] if the prior predecessor cannot be
/// removed or the current log cannot be renamed over it. An empty `path`, an
/// absent predecessor, and an absent current log are all `Ok(())`.
pub fn rotate_log_at_plan_time(path: &Path) -> Result<(), ServiceProcessError> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    let previous = previous_log_path(path);
    match fs::remove_file(&previous) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match fs::rename(path, previous) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[must_use]
pub fn previous_log_path(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return path.with_extension("previous");
    };
    let previous_name = match file_name.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem}.previous.{extension}"),
        None => format!("{file_name}.previous"),
    };
    path.with_file_name(previous_name)
}

fn reset_structured_log(path: &Path) -> Result<(), ServiceProcessError> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    Ok(())
}

fn open_output_log(path: &Path) -> Result<fs::File, ServiceProcessError> {
    Ok(OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?)
}

fn capture_spawned_process_identity_with(
    process_id: u32,
    capture: impl FnOnce(u32) -> std::io::Result<Option<ProcessIdentity>>,
) -> Result<ProcessIdentity, ServiceProcessError> {
    capture(process_id)
        .map_err(|source| ServiceProcessError::ProcessIdentityCapture {
            pid: process_id,
            source,
        })?
        .ok_or(ServiceProcessError::ProcessIdentityUnavailable { pid: process_id })
}

fn capture_spawned_process_identity_or_terminate(
    child: &mut OwnedSynchronousChild,
) -> Result<ProcessIdentity, ServiceProcessError> {
    capture_spawned_process_identity_or_terminate_with(child, ProcessIdentity::capture)
}

fn capture_spawned_process_identity_or_terminate_with(
    child: &mut OwnedSynchronousChild,
    capture: impl FnOnce(u32) -> std::io::Result<Option<ProcessIdentity>>,
) -> Result<ProcessIdentity, ServiceProcessError> {
    match capture_spawned_process_identity_with(child.id(), capture) {
        Ok(identity) => Ok(identity),
        Err(error) => {
            let _ = child.terminate_and_wait();
            Err(error)
        }
    }
}

impl ServiceProcessLauncher for StdServiceProcessLauncher {
    type Error = ServiceProcessError;

    fn spawn(&self, process: &ServiceProcessRecord) -> Result<SpawnedServiceProcess, Self::Error> {
        if let Some(parent) = process.stdout_log.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = process.stderr_log.parent() {
            fs::create_dir_all(parent)?;
        }
        if !process.structured_log.as_os_str().is_empty()
            && let Some(parent) = process.structured_log.parent()
        {
            fs::create_dir_all(parent)?;
        }
        reset_structured_log(&process.structured_log)?;

        let stdout = open_output_log(&process.stdout_log)?;
        let stderr = open_output_log(&process.stderr_log)?;
        let mut command = Command::new(&process.program);
        command
            .args(&process.args)
            .envs(&self.environment)
            .current_dir(&process.cwd)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let owner = OwnedSynchronousCommandTree::new_hidden().map_err(|source| {
            ServiceProcessError::SpawnFailed {
                service: process.service_name.clone(),
                program: process.program.clone(),
                source,
            }
        })?;
        let mut child =
            owner
                .spawn(&mut command)
                .map_err(|source| ServiceProcessError::SpawnFailed {
                    service: process.service_name.clone(),
                    program: process.program.clone(),
                    source,
                })?;
        let identity = capture_spawned_process_identity_or_terminate(&mut child)?;
        self.children
            .borrow_mut()
            .insert(ServiceProcessKey::from_process(process), child);
        Ok(SpawnedServiceProcess {
            service_name: process.service_name.clone(),
            role: process.role,
            run: process.run,
            identity,
        })
    }

    fn is_tracking(&self, process: &ServiceProcessKey) -> bool {
        self.children.borrow().contains_key(process)
    }

    fn try_wait(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, Self::Error> {
        let mut children = self.children.borrow_mut();
        let Some(child) = children.get_mut(process) else {
            return Ok(None);
        };
        let Some(status) = child.try_wait()? else {
            return Ok(None);
        };
        children.remove(process);
        Ok(Some(service_exit(&process.service_name, status)))
    }

    fn terminate(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, Self::Error> {
        let mut children = self.children.borrow_mut();
        let Some(child) = children.get_mut(process) else {
            return Ok(None);
        };
        if let Some(status) = child.try_wait()? {
            children.remove(process);
            return Ok(Some(service_exit(&process.service_name, status)));
        }
        let status = child.terminate_and_wait()?;
        children.remove(process);
        Ok(Some(service_exit(&process.service_name, status)))
    }
}

impl Drop for StdServiceProcessLauncher {
    fn drop(&mut self) {
        for child in self.children.get_mut().values_mut() {
            let _ = child.terminate_and_wait();
        }
        self.children.get_mut().clear();
    }
}

fn service_exit(service_name: &str, status: ExitStatus) -> ServiceProcessExit {
    ServiceProcessExit {
        service_name: service_name.to_string(),
        exit_code: status.code(),
        success: status.success(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_process_key_is_stable_across_runs() {
        let mut first = planned_record();
        let mut replacement = first.clone();
        replacement.run = uuid::Uuid::now_v7();

        assert_ne!(first.run, replacement.run);
        assert_eq!(
            ServiceProcessKey::from_process(&first),
            ServiceProcessKey::from_process(&replacement)
        );

        first.endpoint_address = r"\\.\pipe\azoth-local-editor-project-host".to_string();
        assert_eq!(
            first.endpoint_address,
            r"\\.\pipe\azoth-local-editor-project-host"
        );
        assert_ne!(
            ServiceProcessKey::new("shared", SupervisedServiceRole::ProjectHost),
            ServiceProcessKey::new("shared", SupervisedServiceRole::AssetProcessor),
            "role is part of the durable process key even when service names collide"
        );
    }

    #[test]
    fn durable_service_control_equality_excludes_run_observations() {
        let first_process = planned_record();
        let mut second_process = first_process.clone();
        second_process.run = uuid::Uuid::now_v7();
        second_process.previous_run = Some(first_process.run);

        assert_ne!(first_process.run, second_process.run);
        assert_eq!(first_process, second_process);

        let descriptor = ServiceDescriptor {
            id: ServiceId::new("azoth", "project-host"),
            role: ServiceRole::ProjectHost,
            run: uuid::Uuid::now_v7(),
            protocol: ProtocolVersion::CURRENT,
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41000"),
            observability_endpoint: None,
            lifecycle_endpoint: None,
            capabilities: Vec::new(),
        };
        let first_service = ServiceRecord::from_descriptor(&descriptor).unwrap();
        let mut second_service = first_service.clone();
        second_service.run = uuid::Uuid::now_v7();

        assert_eq!(first_service, second_service);
        assert!(!first_service.same_durable_publication(&second_service));
    }

    #[test]
    fn observability_endpoint_uses_a_separate_bind_target() {
        let pipe = observability_endpoint(Endpoint::new(
            EndpointKind::WindowsNamedPipe,
            r"\\.\pipe\azoth-local-editor-project-host",
        ));
        assert_eq!(
            pipe.address,
            r"\\.\pipe\azoth-local-editor-project-host-observability"
        );

        let socket = observability_endpoint(Endpoint::new(
            EndpointKind::UnixDomainSocket,
            "/tmp/project-host.sock",
        ));
        assert!(socket.address.ends_with("project-host-observability.sock"));

        let tcp = observability_endpoint(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:9444"));
        assert_eq!(tcp.address, "127.0.0.1:0");
    }

    #[test]
    fn lifecycle_endpoint_is_portable_when_role_uses_windows_named_pipe() {
        let lifecycle = service_lifecycle_endpoint(Endpoint::new(
            EndpointKind::WindowsNamedPipe,
            r"\\.\pipe\azoth-local-editor-project-host",
        ));
        assert_eq!(lifecycle.kind, EndpointKind::Tcp);
        assert_eq!(lifecycle.address, "127.0.0.1:0");
    }

    #[test]
    fn wait_for_exit_until_requeues_unrelated_lifecycle_events() {
        let lifecycle = ServiceLifecycleEvents::new();
        let expected = ProcessIdentity {
            process_id: 17,
            process_start_time: 23,
        };
        let unrelated = ProcessIdentity {
            process_id: 19,
            process_start_time: 29,
        };
        lifecycle
            .sender
            .send(ServiceLifecycleEvent::ProcessExited(unrelated))
            .unwrap();
        lifecycle
            .sender
            .send(ServiceLifecycleEvent::ProcessExited(expected))
            .unwrap();

        assert_eq!(
            lifecycle
                .wait_for_exit_until(
                    expected,
                    std::time::Instant::now() + std::time::Duration::from_secs(1),
                )
                .unwrap(),
            Some(ServiceLifecycleEvent::ProcessExited(expected))
        );
        assert_eq!(
            lifecycle.wait().unwrap(),
            ServiceLifecycleEvent::ProcessExited(unrelated)
        );
    }

    #[test]
    fn output_log_starts_each_process_incarnation_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("worker.stdout.log");
        fs::write(&path, b"previous process incarnation\n").unwrap();

        drop(open_output_log(&path).unwrap());

        assert_eq!(fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn structured_log_starts_each_process_incarnation_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("asset-worker.capnp.log");
        fs::write(&path, b"previous process incarnation\n").unwrap();

        reset_structured_log(&path).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn planning_rotates_exactly_one_predecessor_under_a_stable_name() {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("asset-worker.capnp.log");
        let previous = previous_log_path(&current);
        assert_eq!(
            previous.file_name().unwrap(),
            "asset-worker.capnp.previous.log"
        );
        fs::write(&current, b"first run\n").unwrap();
        fs::write(&previous, b"obsolete predecessor\n").unwrap();
        let unrelated = temp.path().join("asset-processor.capnp.log");
        fs::write(&unrelated, b"unrelated\n").unwrap();

        rotate_log_at_plan_time(&current).unwrap();

        assert!(!current.exists());
        assert_eq!(fs::read(&previous).unwrap(), b"first run\n");
        assert!(unrelated.exists());
    }

    fn planned_record() -> ServiceProcessRecord {
        ServiceProcessRecord::planned(
            "asset-processor",
            SupervisedServiceRole::AssetProcessor,
            uuid::Uuid::now_v7(),
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            // Deliberately not this test binary: a regression that skips the
            // identity check must land on the executable-image comparison and
            // fail the assertion, never terminate the test runner.
            "azoth-service-that-is-not-this-test",
            PathBuf::from("."),
            Vec::new(),
            PathBuf::from("stdout.log"),
            PathBuf::from("stderr.log"),
            PathBuf::from("structured.log"),
            None,
            1,
        )
    }

    fn running_record(identity: ProcessIdentity) -> ServiceProcessRecord {
        let mut record = planned_record();
        record.mark_running(identity, 2).unwrap();
        record
    }

    #[test]
    fn spawned_identity_capture_reports_query_failure_and_disappearance() {
        let error = capture_spawned_process_identity_with(4242, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "identity query denied",
            ))
        })
        .unwrap_err();
        assert!(matches!(
            error,
            ServiceProcessError::ProcessIdentityCapture { pid: 4242, source }
                if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert!(matches!(
            capture_spawned_process_identity_with(4242, |_| Ok(None)),
            Err(ServiceProcessError::ProcessIdentityUnavailable { pid: 4242 })
        ));
    }

    #[test]
    fn spawned_identity_capture_failure_reaps_the_untracked_child() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1 > NUL"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30"]);
            command
        };
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let owner = OwnedSynchronousCommandTree::new_hidden().unwrap();
        let mut child = owner.spawn(&mut command).unwrap();
        let process_id = child.id();

        let error = capture_spawned_process_identity_or_terminate_with(&mut child, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "identity query denied",
            ))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            ServiceProcessError::ProcessIdentityCapture { pid, source }
                if pid == process_id && source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn a_live_pid_under_a_different_start_time_is_not_the_launched_process() {
        let live = ProcessIdentity::current().unwrap();
        let stale = live.process_start_time.wrapping_sub(1);
        assert_ne!(stale, live.process_start_time);

        // Control: the very same pid, with the token actually read from it, is
        // judged live — so the verdict below cannot be an artefact of the pid
        // being unreadable or dead.
        assert_eq!(
            RecordedProcess::assess(Some(live.process_id), Some(live.process_start_time)).unwrap(),
            RecordedProcess::Live { recorded: live }
        );

        assert_eq!(
            RecordedProcess::assess(Some(live.process_id), Some(stale)).unwrap(),
            RecordedProcess::Reused {
                recorded: ProcessIdentity {
                    process_id: live.process_id,
                    process_start_time: stale,
                },
                actual_start_time: live.process_start_time,
            }
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn process_exit_waiter_refuses_a_reused_pid_identity() {
        let live = ProcessIdentity::current().unwrap();
        let stale = ProcessIdentity {
            process_id: live.process_id,
            process_start_time: live.process_start_time.wrapping_sub(1),
        };

        assert!(matches!(
            ProcessExitWaiter::bind(stale),
            Err(ServiceProcessError::ProcessExitIdentityMismatch { identity, .. })
                if identity == stale
        ));
    }

    #[test]
    fn ready_subscription_finishes_without_a_later_filesystem_event() {
        let temp = tempfile::tempdir().unwrap();
        let ready_file = temp.path().join("asset-processor.ready");
        let lifecycle = ServiceLifecycleEvents::new();
        let mut subscription = lifecycle.subscribe_ready([ready_file.as_path()]).unwrap();

        // `finish` must wake the forwarding worker itself. No write is made to
        // the watched directory after arming the subscription.
        subscription.finish().unwrap();
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn retiring_one_identity_keeps_later_exit_subscription_without_a_stale_event() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = StdServiceProcessLauncher::new();
        let lifecycle = ServiceLifecycleEvents::new();
        let first = launcher
            .spawn(&lifecycle_test_record(
                "first",
                SupervisedServiceRole::AssetProcessor,
                temp.path(),
                60,
            ))
            .unwrap();
        lifecycle.add_identity(first.identity).unwrap();

        // This models a later partial start: it may add new children but it
        // must not discard the exit wait already bound for `first`.
        let second = launcher
            .spawn(&lifecycle_test_record(
                "second",
                SupervisedServiceRole::Worker,
                temp.path(),
                1,
            ))
            .unwrap();
        lifecycle.add_identity(second.identity).unwrap();

        launcher
            .terminate(&ServiceProcessKey::from_spawned(&first))
            .unwrap();
        lifecycle.retire_exit(first.identity).unwrap();

        assert_eq!(
            lifecycle.wait().unwrap(),
            ServiceLifecycleEvent::ProcessExited(second.identity)
        );
        assert!(
            launcher
                .try_wait(&ServiceProcessKey::from_spawned(&second))
                .unwrap()
                .is_some()
        );
        lifecycle.consume_exit(second.identity).unwrap();
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn cancelling_a_live_exit_wait_joins_without_publishing_a_stale_exit() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = StdServiceProcessLauncher::new();
        let lifecycle = ServiceLifecycleEvents::new();
        let spawned = launcher
            .spawn(&lifecycle_test_record(
                "live",
                SupervisedServiceRole::AssetProcessor,
                temp.path(),
                60,
            ))
            .unwrap();
        lifecycle.add_identity(spawned.identity).unwrap();

        // The process stays alive. Returning here proves cancellation wakes
        // the OS-bound wait and joins its worker rather than detaching it.
        lifecycle.cancel_exit_wait(spawned.identity).unwrap();
        assert!(
            lifecycle
                .wait_until(std::time::Instant::now() + std::time::Duration::from_millis(25))
                .unwrap()
                .is_none(),
            "retired live identity published a stale lifecycle transition"
        );

        launcher
            .terminate(&ServiceProcessKey::from_spawned(&spawned))
            .unwrap();
    }

    #[cfg(windows)]
    fn lifecycle_test_command(seconds: u64) -> (String, Vec<String>) {
        (
            "powershell.exe".to_string(),
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                format!("Start-Sleep -Seconds {seconds}"),
            ],
        )
    }

    #[cfg(target_os = "linux")]
    fn lifecycle_test_command(seconds: u64) -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec!["-c".to_string(), format!("sleep {seconds}")],
        )
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn lifecycle_test_record(
        service_name: &str,
        role: SupervisedServiceRole,
        directory: &Path,
        sleep_seconds: u64,
    ) -> ServiceProcessRecord {
        let (program, args) = lifecycle_test_command(sleep_seconds);
        ServiceProcessRecord::planned(
            service_name,
            role,
            uuid::Uuid::now_v7(),
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            program,
            directory.to_path_buf(),
            args,
            directory.join(format!("{service_name}.out")),
            directory.join(format!("{service_name}.err")),
            directory.join(format!("{service_name}.capnp.log")),
            None,
            1,
        )
    }

    #[test]
    fn marking_a_record_running_persists_the_start_token_with_the_pid() {
        let live = ProcessIdentity::current().unwrap();
        let record = running_record(live);

        assert_eq!(record.pid, Some(live.process_id));
        assert_eq!(record.process_start_time, Some(live.process_start_time));
        assert!(record.process().unwrap().is_live());
    }

    #[test]
    fn terminating_a_recycled_pid_is_refused_rather_than_killing_a_stranger() {
        let live = ProcessIdentity::current().unwrap();
        let mut record = running_record(live);
        let stale = live.process_start_time.wrapping_sub(1);
        record.process_start_time = Some(stale);

        assert_eq!(
            terminate_recorded_service_process(&record).unwrap(),
            RecordedServiceProcessCleanup::Reused {
                pid: live.process_id,
                recorded_start_time: stale,
                actual_start_time: live.process_start_time,
            }
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn platform_termination_rechecks_identity_on_the_bound_termination_object() {
        let live = ProcessIdentity::current().unwrap();
        let stale = live.process_start_time.wrapping_sub(1);
        let mut record = running_record(live);
        record.process_start_time = Some(stale);

        assert_eq!(
            platform_terminate_recorded_service_process(&record).unwrap(),
            RecordedServiceProcessCleanup::Reused {
                pid: live.process_id,
                recorded_start_time: stale,
                actual_start_time: live.process_start_time,
            }
        );
        assert_eq!(ProcessIdentity::current().unwrap(), live);
    }

    #[test]
    fn a_record_without_a_start_token_cannot_attribute_its_pid() {
        let live = ProcessIdentity::current().unwrap();

        assert_eq!(
            RecordedProcess::assess(Some(live.process_id), None).unwrap(),
            RecordedProcess::Unattributable {
                process_id: live.process_id
            }
        );
        assert_eq!(
            RecordedProcess::assess(None, None).unwrap(),
            RecordedProcess::Unrecorded
        );
    }

    #[test]
    fn a_legacy_serialized_record_without_a_start_token_remains_unattributable() {
        let live = ProcessIdentity::current().unwrap();
        let encoded = toml::to_string(&running_record(live)).unwrap();
        let legacy = encoded
            .lines()
            .filter(|line| !line.starts_with("process_start_time ="))
            .collect::<Vec<_>>()
            .join("\n");

        let record: ServiceProcessRecord = toml::from_str(&legacy).unwrap();
        assert_eq!(record.process_start_time, None);
        assert_eq!(
            record.process().unwrap(),
            RecordedProcess::Unattributable {
                process_id: live.process_id
            }
        );
        assert_eq!(
            terminate_recorded_service_process(&record).unwrap(),
            RecordedServiceProcessCleanup::Unattributable {
                pid: live.process_id
            }
        );
    }

    #[test]
    fn cleanup_verdict_only_proves_gone_for_terminal_identity_outcomes() {
        for cleanup in [
            RecordedServiceProcessCleanup::NotRunning { pid: 1 },
            RecordedServiceProcessCleanup::Terminated { pid: 1 },
            RecordedServiceProcessCleanup::Reused {
                pid: 1,
                recorded_start_time: 10,
                actual_start_time: 11,
            },
        ] {
            assert!(cleanup.proves_recorded_process_gone(), "{cleanup:?}");
        }
        for cleanup in [
            RecordedServiceProcessCleanup::NoPid,
            RecordedServiceProcessCleanup::ImageMismatch {
                pid: 1,
                expected: "expected".to_string(),
                actual: "actual".to_string(),
            },
            RecordedServiceProcessCleanup::Unattributable { pid: 1 },
            RecordedServiceProcessCleanup::IdentityBindingUnavailable {
                pid: 1,
                reason: "unsupported".to_string(),
            },
        ] {
            assert!(!cleanup.proves_recorded_process_gone(), "{cleanup:?}");
        }
    }

    #[test]
    fn marking_running_persists_the_spawn_identity_without_recapturing_pid() {
        let live = ProcessIdentity::current().unwrap();
        let supplied = ProcessIdentity {
            process_id: live.process_id,
            process_start_time: live.process_start_time.wrapping_sub(1),
        };
        let record = running_record(supplied);

        assert_eq!(record.pid, Some(supplied.process_id));
        assert_eq!(record.process_start_time, Some(supplied.process_start_time));
    }

    #[test]
    fn marking_running_rejects_an_incomplete_identity_without_mutation() {
        let before = planned_record();
        let mut record = before.clone();
        let identity = ProcessIdentity {
            process_id: 42,
            process_start_time: 0,
        };

        assert!(matches!(
            record.mark_running(identity, 2),
            Err(ServiceProcessError::InvalidProcessIdentity { identity: actual })
                if actual == identity
        ));
        assert_eq!(record, before);
    }

    #[cfg(unix)]
    #[test]
    fn replaced_unix_program_still_matches_its_recorded_owner_path() {
        assert_eq!(
            recorded_unix_process_program(Path::new("/opt/azoth/asset-worker (deleted)")),
            PathBuf::from("/opt/azoth/asset-worker")
        );
    }
}
