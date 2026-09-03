//! Editor-side bootstrap for the external `azd` sidecar process.
//!
//! This module is process orchestration only. It keeps the editor as an IPC
//! client and never links or hosts daemon/project service implementation code.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use az_endpoint_discovery::{
    DaemonEndpointRecord, default_daemon_endpoint_kind, project_daemon_endpoint,
    project_daemon_endpoint_record_path, read_project_daemon_endpoint_record,
    remove_project_daemon_endpoint_record,
};
use az_proto_core::Endpoint;
use az_service_supervision::{
    ProcessIdentity, ServiceLifecycleEvent, ServiceLifecycleEvents, ServiceProcessError,
};
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tracing::{info, instrument, warn};

use crate::daemon::{
    AzDaemonClient, ensure_daemon_endpoint_is_local, validate_daemon_endpoint_record_protocol,
};
use crate::error::{EditorError, EditorResult};

const EDITOR_DAEMON_START_TIMEOUT_MS: u64 = 10_000;

/// A probe is one round-trip to a daemon on this same machine, so a healthy one
/// answers in milliseconds; five seconds is slack for a loaded daemon while
/// staying inside the sidecar start budget above.
const EDITOR_DAEMON_PROBE_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonLaunchCommand {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
}

/// Resolve the endpoint of an `azd` serving `project_root`, starting a sidecar
/// when nothing is answering, and register this editor's process lease on it.
///
/// # Errors
///
/// Returns [`EditorError::Io`] when the calling process identity cannot be read,
/// or [`EditorError::GeneratedProjectTargets`] when the project's generated
/// targets cannot be synchronized. Returns [`EditorError::ServiceDiscovery`]
/// when the project endpoint cannot be resolved, or from
/// `reachable_runtime_record` when the endpoint record cannot be read or a
/// stale one cannot be removed. A probe failure that
/// [`is_daemon_transport_failure`] does not accept is a live daemon refusing the
/// preflight and is surfaced unchanged rather than starting a second daemon over
/// it; a transport failure instead falls through to `start_daemon_sidecar`,
/// which fails when the sibling `azd` executable is missing, its log file cannot
/// be opened, the process cannot be spawned, or it never publishes a matching
/// endpoint record before the start deadline. Finally, `touch_editor_process_lease`
/// surfaces [`EditorError::RpcTransport`] when the daemon that was found cannot
/// be dialed for the lease round-trip.
#[instrument(skip_all, fields(project_root = %project_root.display()))]
pub fn ensure_daemon_endpoint_for_project(project_root: &Path) -> EditorResult<Endpoint> {
    let started = Instant::now();
    let editor_process = ProcessIdentity::current()?;
    az_project_scaffold::ensure_project_generated_targets(project_root)
        .map_err(|error| EditorError::GeneratedProjectTargets(error.to_string()))?;
    let discovery_started = Instant::now();
    let mut started_sidecar = false;
    let endpoint = if let Some(record) = reachable_runtime_record(project_root)? {
        record.endpoint
    } else {
        let endpoint = project_daemon_endpoint(default_daemon_endpoint_kind(), project_root)
            .map_err(|error| {
                EditorError::ServiceDiscovery(format!(
                    "failed to resolve project azd endpoint: {error}"
                ))
            })?;
        match probe_daemon_endpoint(&endpoint) {
            Ok(()) => endpoint,
            Err(error) if is_daemon_transport_failure(&error) => {
                started_sidecar = true;
                start_daemon_sidecar(project_root, &endpoint, editor_process)?
            }
            Err(error) => return Err(error),
        }
    };
    let discovery_ms = discovery_started.elapsed().as_millis();
    let lease_started = Instant::now();
    touch_editor_process_lease(&endpoint, editor_process)?;
    let lease_ms = lease_started.elapsed().as_millis();
    info!(
        total_ms = started.elapsed().as_millis(),
        discovery_ms, lease_ms, started_sidecar, "editor daemon bootstrap timing summary"
    );
    Ok(endpoint)
}

fn touch_editor_process_lease(
    endpoint: &Endpoint,
    editor_process: ProcessIdentity,
) -> EditorResult<()> {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    let endpoint = endpoint.clone();
    local.block_on(&runtime, async move {
        let daemon = AzDaemonClient::connect(&endpoint).await?;
        let result = daemon.touch_editor_process_lease(editor_process).await?;
        info!(
            active_lease_count = result.active_lease_count,
            lease_id = %result.lease_id,
            "touched azd editor process lease"
        );
        Ok(())
    })
}

/// Round-trip the daemon at `endpoint`, bounded by
/// `EDITOR_DAEMON_PROBE_TIMEOUT_MS`.
///
/// Both halves are inside the deadline: a peer can complete the transport
/// handshake and then never answer the protocol preflight, which is the shape
/// that stalls bootstrap. Nothing else bounds this — the sidecar start timeout
/// covers only a child this process spawned itself.
///
/// This is the editor's *only* daemon probe. The standalone `az-editor` binary
/// re-probes a runtime-record endpoint before opening a session and calls this;
/// it previously carried its own connect-only copy with no deadline, which is
/// exactly the hazard the deadline above exists to close.
///
/// # Errors
///
/// Returns [`EditorError::RpcTransport`] when the endpoint cannot be reached,
/// answers nothing within the deadline, or drops the connection —
/// [`is_daemon_transport_failure`] accepts that class and callers recover by
/// evicting the record and starting a sidecar. Any other error is a live daemon
/// refusing the preflight, which callers must surface unchanged.
pub fn probe_daemon_endpoint(endpoint: &Endpoint) -> EditorResult<()> {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    let endpoint = endpoint.clone();
    local.block_on(&runtime, async move {
        let probe = async {
            let daemon = AzDaemonClient::connect(&endpoint).await?;
            daemon.probe().await
        };
        match tokio::time::timeout(Duration::from_millis(EDITOR_DAEMON_PROBE_TIMEOUT_MS), probe)
            .await
        {
            Ok(result) => result,
            Err(_elapsed) => Err(probe_deadline_exceeded(&endpoint)),
        }
    })
}

/// A probe that runs out its deadline is a transport failure, the same class as
/// a refused connection: bootstrap must recover by spawning a sidecar rather
/// than surfacing a wedged peer as a hard bootstrap failure.
fn probe_deadline_exceeded(endpoint: &Endpoint) -> EditorError {
    EditorError::RpcTransport(az_rpc::AzRpcTransportError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "azd at `{}` did not answer the bootstrap probe within {EDITOR_DAEMON_PROBE_TIMEOUT_MS}ms",
            endpoint.address
        ),
    )))
}

/// Probe the endpoint a record names, then hold the daemon that answered to the
/// protocol version that record claims.
///
/// That order is the whole content of this function, and it is shared rather
/// than restated because both callers of a runtime record need it. A recorded
/// protocol is a claim *about the daemon that wrote the record*, and the claim
/// only binds while that daemon is still alive: a record left by a foreign azd
/// build whose endpoint no longer answers is stale in exactly the way an absent
/// one is, so it must arrive at its caller as a transport failure — the class
/// every caller already recovers from by evicting the record. Validating first
/// made that same record a hard error that evicted nothing, so nothing could
/// self-heal and the machine-wide record had to be deleted by hand (bootstrap:
/// ticket 044; attach: ticket 050).
///
/// Only the ordering lives here. What a caller *does* with the two outcomes is
/// its own: `reachable_runtime_record` treats an evicted record as no runtime
/// and starts a sidecar, `attach_to_session` evicts and surfaces the failure.
///
/// # Errors
///
/// Returns [`EditorError::RpcTransport`] when the recorded endpoint is
/// unreachable, wedged, or drops the connection — the class
/// [`is_daemon_transport_failure`] accepts. A live daemon still fails fast twice
/// over, and neither way is a transport failure, so neither evicts: a peer
/// speaking another protocol fails the probe's own preflight, and a peer
/// answering the current protocol under a record claiming another build fails
/// the validation here. A version conflict is for the user to see.
pub(crate) fn probe_daemon_endpoint_record(record: &DaemonEndpointRecord) -> EditorResult<()> {
    probe_daemon_endpoint(&record.endpoint)?;
    validate_daemon_endpoint_record_protocol(record)
}

/// The recorded endpoint, if a daemon is still answering there.
///
/// Locality comes first because it is a trust gate on untrusted project state:
/// it decides whether this process may open the connection at all, so nothing
/// dials ahead of it. Reachability and the recorded protocol version follow in
/// that order, on [`probe_daemon_endpoint_record`], which owns why.
///
/// This is the bootstrap half of that split: a record whose endpoint does not
/// answer is evicted and reported as no runtime at all, which is what lets the
/// sidecar republish. Everything else is surfaced unchanged — a live version
/// conflict is for the user to resolve, not for bootstrap to paper over by
/// starting a second daemon on top of the peer that is already there.
fn reachable_runtime_record(project_root: &Path) -> EditorResult<Option<DaemonEndpointRecord>> {
    let record = read_project_daemon_endpoint_record(project_root).map_err(|error| {
        EditorError::ServiceDiscovery(format!("failed to read azd endpoint record: {error}"))
    })?;
    let Some(record) = record else {
        return Ok(None);
    };
    ensure_daemon_endpoint_is_local(&record.endpoint)?;

    match probe_daemon_endpoint_record(&record) {
        Ok(()) => Ok(Some(record)),
        Err(error) if is_daemon_transport_failure(&error) => {
            warn!(
                error = %error,
                "azd runtime endpoint record was stale; removing record before sidecar bootstrap"
            );
            remove_project_daemon_endpoint_record(project_root).map_err(|error| {
                EditorError::ServiceDiscovery(format!(
                    "failed to remove stale azd endpoint record: {error}"
                ))
            })?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn start_daemon_sidecar(
    project_root: &Path,
    requested_endpoint: &Endpoint,
    editor_process: ProcessIdentity,
) -> EditorResult<Endpoint> {
    let command = daemon_launch_command(&[project_root.to_path_buf()], editor_process)?;
    let log_path = daemon_log_path(project_root)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;

    info!(
        program = %command.program.display(),
        args = ?command.args,
        cwd = %command.cwd.display(),
        log = %log_path.display(),
        "starting azd sidecar for editor project workflow"
    );

    let mut child = spawn_daemon_process(&command, stdout, stderr)?;
    let record = wait_for_daemon_start(
        project_root,
        &mut child,
        &command,
        requested_endpoint,
        EDITOR_DAEMON_START_TIMEOUT_MS,
        &log_path,
    )?;
    Ok(record.endpoint)
}

/// The parts of a start wait that do not change while the poll spins.
#[derive(Clone, Copy)]
struct DaemonStartWait<'a> {
    project_root: &'a Path,
    requested_endpoint: &'a Endpoint,
    command: &'a DaemonLaunchCommand,
    log_path: &'a Path,
    timeout_ms: u64,
    deadline: Instant,
}

fn wait_for_daemon_start(
    project_root: &Path,
    child: &mut Child,
    command: &DaemonLaunchCommand,
    requested_endpoint: &Endpoint,
    timeout_ms: u64,
    log_path: &Path,
) -> EditorResult<DaemonEndpointRecord> {
    let wait = DaemonStartWait {
        project_root,
        requested_endpoint,
        command,
        log_path,
        timeout_ms,
        deadline: Instant::now() + Duration::from_millis(timeout_ms),
    };
    let record_path = match project_daemon_endpoint_record_path(project_root) {
        Ok(path) => path,
        Err(error) => {
            abort_daemon_child(child);
            return Err(EditorError::ServiceDiscovery(format!(
                "failed to resolve azd endpoint record path: {error}"
            )));
        }
    };
    let process_identity = capture_daemon_process_identity(child)?;
    let lifecycle = ServiceLifecycleEvents::new();
    if let Err(error) = lifecycle.add_identity(process_identity) {
        abort_daemon_child(child);
        return Err(daemon_lifecycle_error(&error));
    }
    let mut ready = match lifecycle.subscribe_ready([record_path.as_path()]) {
        Ok(ready) => ready,
        Err(error) => {
            abort_daemon_child(child);
            let _ = lifecycle.retire_exit(process_identity);
            return Err(daemon_lifecycle_error(&error));
        }
    };

    match poll_for_daemon_endpoint(wait, child, &lifecycle, process_identity) {
        Ok(record) => {
            let release = lifecycle
                .cancel_exit_wait(process_identity)
                .map_err(|error| daemon_lifecycle_error(&error));
            let ready = ready
                .finish()
                .map_err(|error| daemon_lifecycle_error(&error));
            release?;
            ready?;
            Ok(record)
        }
        Err(error) => {
            abort_daemon_child(child);
            if let Err(cleanup) = ready.finish() {
                warn!(error = %cleanup, "failed to close azd readiness watcher after bootstrap failure");
            }
            if let Err(cleanup) = lifecycle.retire_exit(process_identity) {
                warn!(error = %cleanup, "failed to retire azd exit wait after bootstrap failure");
            }
            Err(error)
        }
    }
}

/// Binds the sidecar exit to a captured identity, killing the child when the
/// capture fails so no unowned process is left behind.
fn capture_daemon_process_identity(child: &mut Child) -> EditorResult<ProcessIdentity> {
    match ProcessIdentity::capture(child.id()) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => {
            abort_daemon_child(child);
            Err(EditorError::ServiceDiscovery(
                "azd sidecar disappeared before its exit could be bound".to_string(),
            ))
        }
        Err(error) => {
            abort_daemon_child(child);
            Err(error.into())
        }
    }
}

/// Spins until the sidecar publishes a reachable endpoint matching the request,
/// the sidecar exits, or the deadline passes.
fn poll_for_daemon_endpoint(
    wait: DaemonStartWait<'_>,
    child: &mut Child,
    lifecycle: &ServiceLifecycleEvents,
    process_identity: ProcessIdentity,
) -> EditorResult<DaemonEndpointRecord> {
    // One deadline governs the whole race; bind it once so every arm below
    // waits against the same instant.
    let deadline = wait.deadline;
    loop {
        if let Some(record) =
            read_project_daemon_endpoint_record(wait.project_root).map_err(|error| {
                EditorError::ServiceDiscovery(format!(
                    "failed to read azd endpoint record: {error}"
                ))
            })?
        {
            // A remote endpoint in the record is a trust violation, not a
            // stale record: fail fast instead of waiting out the timeout.
            ensure_daemon_endpoint_is_local(&record.endpoint)?;
            if endpoint_matches_requested(&record.endpoint, wait.requested_endpoint) {
                validate_daemon_endpoint_record_protocol(&record)?;
                if probe_daemon_endpoint(&record.endpoint).is_ok() {
                    return Ok(record);
                }
            }
        }

        match lifecycle
            .wait_until(deadline)
            .map_err(|error| daemon_lifecycle_error(&error))?
        {
            Some(ServiceLifecycleEvent::ProcessExited(identity))
                if identity == process_identity =>
            {
                let status = child.wait()?;
                return Err(EditorError::ServiceDiscovery(format!(
                    "azd sidecar exited before publishing a reachable endpoint: {} {:?}; status={:?}; log={}",
                    wait.command.program.display(),
                    wait.command.args,
                    status.code(),
                    wait.log_path.display()
                )));
            }
            Some(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason })
                if identity == process_identity =>
            {
                return Err(EditorError::ServiceDiscovery(format!(
                    "azd sidecar exit wait failed: {reason}"
                )));
            }
            Some(
                ServiceLifecycleEvent::ReadyFileChanged
                | ServiceLifecycleEvent::ProcessExited(_)
                | ServiceLifecycleEvent::ProcessExitWaitFailed { .. },
            ) => {}
            None => {
                let timeout_ms = wait.timeout_ms;
                return Err(EditorError::ServiceDiscovery(format!(
                    "azd sidecar did not publish a reachable endpoint within {timeout_ms}ms; log={}",
                    wait.log_path.display()
                )));
            }
        }
    }
}

fn abort_daemon_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn daemon_lifecycle_error(error: &ServiceProcessError) -> EditorError {
    EditorError::ServiceDiscovery(format!("azd lifecycle event failed: {error}"))
}

fn spawn_daemon_process(
    command: &DaemonLaunchCommand,
    stdout: std::fs::File,
    stderr: std::fs::File,
) -> std::io::Result<Child> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    configure_background_process(&mut process);
    process.spawn()
}

/// Win32 `CREATE_NEW_PROCESS_GROUP`: the sidecar leaves the editor's console
/// control group, so a Ctrl+C aimed at the editor never reaches azd.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Win32 `CREATE_NO_WINDOW`: azd is a background service and must never flash a
/// console window behind a GUI editor launch.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The complete creation-flag mask every azd sidecar is spawned under.
#[cfg(windows)]
const BACKGROUND_CREATION_FLAGS: u32 = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;

#[cfg(windows)]
fn configure_background_process(process: &mut Command) {
    use std::os::windows::process::CommandExt;

    process.creation_flags(BACKGROUND_CREATION_FLAGS);
}

#[cfg(not(windows))]
fn configure_background_process(_process: &mut Command) {}

fn daemon_launch_command(
    project_roots: &[PathBuf],
    editor_process: ProcessIdentity,
) -> EditorResult<DaemonLaunchCommand> {
    let program = sibling_daemon_executable()?.ok_or_else(|| {
        EditorError::ServiceDiscovery(format!(
            "`{}` was not found next to `{}`; build/package the azd sidecar beside az-editor",
            daemon_executable_name(),
            std::env::current_exe().map_or_else(
                |_| "az-editor".to_string(),
                |path| path.display().to_string()
            )
        ))
    })?;
    Ok(DaemonLaunchCommand {
        program,
        args: daemon_args(project_roots, editor_process)?,
        cwd: std::env::current_dir()?,
    })
}

fn daemon_args(
    project_roots: &[PathBuf],
    editor_process: ProcessIdentity,
) -> EditorResult<Vec<String>> {
    let mut args = Vec::new();
    for root in project_roots {
        args.extend([
            "--project".to_string(),
            child_path(root)?.to_string_lossy().into_owned(),
        ]);
    }
    args.extend([
        "--editor-owner-process".to_string(),
        format!(
            "{}:{}",
            editor_process.process_id, editor_process.process_start_time
        ),
        "--shutdown-when-editor-leases-gone".to_string(),
    ]);
    Ok(args)
}

fn child_path(path: &Path) -> EditorResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn sibling_daemon_executable() -> EditorResult<Option<PathBuf>> {
    let current = std::env::current_exe()?;
    Ok(sibling_daemon_executable_from(&current))
}

fn sibling_daemon_executable_from(current: &Path) -> Option<PathBuf> {
    let dir = current.parent()?;
    let candidate = dir.join(daemon_executable_name());
    candidate.is_file().then_some(candidate)
}

const fn daemon_executable_name() -> &'static str {
    if cfg!(windows) { "azd.exe" } else { "azd" }
}

fn daemon_log_path(project_root: &Path) -> EditorResult<PathBuf> {
    project_daemon_endpoint_record_path(project_root)
        .map(|path| path.with_file_name("azd.log"))
        .map_err(|error| {
            EditorError::ServiceDiscovery(format!("failed to resolve azd log path: {error}"))
        })
}

fn endpoint_matches_requested(record: &Endpoint, requested: &Endpoint) -> bool {
    record.kind == requested.kind
        && (record.address == requested.address || requested.address.ends_with(":0"))
}

/// True when `error` is the recoverable class: nothing is answering at the
/// endpoint, so the record that named it is stale.
///
/// The editor's three daemon-record call sites — bootstrap here, `attach.rs`,
/// and the standalone binary — must agree on which failures evict a record and
/// which are reported to the user, so they share this one definition rather
/// than each carrying a copy of the match.
#[must_use]
pub const fn is_daemon_transport_failure(error: &EditorError) -> bool {
    matches!(error, EditorError::RpcTransport(_))
}

/// Endpoint-record fixtures for the tests of every caller of
/// [`probe_daemon_endpoint_record`].
///
/// The probe-then-validate order is shared, so the records that discriminate it
/// are shared too: `attach.rs` pins the same split this module's own tests do,
/// against the same hand-written record rather than a second transcription of
/// the on-disk format.
///
/// `pub` rather than `pub(crate)` so the architecture guard sees this for what
/// it is. Its production scan strips `#[cfg(test)]` modules by matching `mod `
/// or `pub mod ` on the line after the gate
/// ([`az_architecture_guard::production_source_without_cfg_test_modules`]), and
/// `pub(crate) mod` matches neither — a fixture module spelled that way is read
/// as production code and would arm every needle in the editor's guard tables
/// against test scaffolding. Nothing widens: the module is `#[cfg(test)]`, so
/// it does not exist in a production build, and every item inside stays
/// `pub(crate)`, so nothing is reachable outside this crate in a test build
/// either.
#[cfg(test)]
pub mod test_records {
    use std::path::{Path, PathBuf};

    use az_endpoint_discovery::{
        project_daemon_endpoint_record_path, remove_project_daemon_endpoint_record,
    };
    use az_proto_core::{Endpoint, EndpointKind, ProtocolVersion};

    /// A project root that is stable across runs and unique per test.
    ///
    /// A project's endpoint record does not live under the project: it lives in
    /// the machine-wide Azoth data home, keyed by the manifest's project id and
    /// a hash of this path. Deriving both from a fixed `suite`/`name` keeps
    /// every test isolated from every other, and bounds what these tests leave
    /// in the data home to one directory per test instead of a fresh one on
    /// every run.
    pub(crate) fn record_test_project(suite: &str, name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("az-editor-{suite}-tests"))
            .join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("azoth.toml"),
            format!("[project]\nid = \"az-editor-{suite}-{name}\"\n"),
        )
        .unwrap();
        // A record left behind by an earlier run is state this test does not
        // own; clearing it is what makes a stable root safe to reuse.
        remove_project_daemon_endpoint_record(&root).unwrap();
        root
    }

    /// A machine-local endpoint of the platform's default kind that nothing is
    /// listening on. An absent named pipe and an absent unix socket both fail
    /// the connect immediately, so no test depends on a port-binding race.
    pub(crate) fn unreachable_local_endpoint() -> Endpoint {
        let unique = uuid::Uuid::now_v7();
        if cfg!(windows) {
            Endpoint::new(
                EndpointKind::WindowsNamedPipe,
                format!(r"\\.\pipe\az-editor-bootstrap-absent-{unique}"),
            )
        } else {
            Endpoint::new(
                EndpointKind::UnixDomainSocket,
                std::env::temp_dir()
                    .join(format!("az-editor-bootstrap-absent-{unique}.sock"))
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    /// A protocol build that is not this one.
    pub(crate) fn foreign_protocol_version() -> ProtocolVersion {
        ProtocolVersion {
            major: ProtocolVersion::CURRENT.major + 1,
            minor: 0,
            patch: 0,
        }
    }

    /// Write a record for `endpoint` stamped with [`foreign_protocol_version`],
    /// and return its path.
    ///
    /// Hand-written on purpose: the record writer always stamps the current
    /// protocol, so a foreign-protocol record cannot be produced through it.
    /// The address goes in a TOML *literal* string because a Windows pipe name
    /// is mostly backslashes.
    pub(crate) fn write_foreign_protocol_record(root: &Path, endpoint: &Endpoint) -> PathBuf {
        let record_path = project_daemon_endpoint_record_path(root).unwrap();
        std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
        let kind = match endpoint.kind {
            EndpointKind::Tcp => "tcp",
            EndpointKind::WindowsNamedPipe => "windows-named-pipe",
            EndpointKind::UnixDomainSocket => "unix-domain-socket",
            EndpointKind::InProcess => {
                unreachable!("endpoint records carry public transports only")
            }
        };
        let version = foreign_protocol_version();
        std::fs::write(
            &record_path,
            format!(
                "kind = \"{kind}\"\naddress = '{address}'\nprocess_id = 1\n\
                 protocol_major = {major}\nprotocol_minor = {minor}\n\
                 protocol_patch = {patch}\n",
                address = endpoint.address,
                major = version.major,
                minor = version.minor,
                patch = version.patch,
            ),
        )
        .unwrap();
        record_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use az_proto_core::EndpointKind;

    use super::test_records::{
        foreign_protocol_version, record_test_project, unreachable_local_endpoint,
        write_foreign_protocol_record,
    };

    /// A program that prints `marker` on stdout and exits `0`.
    #[cfg(windows)]
    fn echo_command(marker: &str) -> (PathBuf, Vec<String>) {
        (
            PathBuf::from("cmd.exe"),
            vec!["/C".to_string(), format!("echo {marker}")],
        )
    }

    #[cfg(not(windows))]
    fn echo_command(marker: &str) -> (PathBuf, Vec<String>) {
        (
            PathBuf::from("sh"),
            vec!["-c".to_string(), format!("echo {marker}")],
        )
    }

    /// A launch command for `program`/`args` rooted at `cwd`, shaped exactly
    /// like the one `daemon_launch_command` builds for the real sidecar.
    fn launch_command(program: PathBuf, args: Vec<String>, cwd: &Path) -> DaemonLaunchCommand {
        DaemonLaunchCommand {
            program,
            args,
            cwd: cwd.to_path_buf(),
        }
    }

    /// Open a sidecar log the way `start_daemon_sidecar` does.
    fn open_log(path: &Path) -> (std::fs::File, std::fs::File) {
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        let stderr = stdout.try_clone().unwrap();
        (stdout, stderr)
    }

    /// A long-running child that never publishes an endpoint record.
    #[cfg(windows)]
    fn never_ready_command() -> (PathBuf, Vec<String>) {
        (
            PathBuf::from("ping"),
            ["-n", "600", "127.0.0.1"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
    }

    #[cfg(not(windows))]
    fn never_ready_command() -> (PathBuf, Vec<String>) {
        (PathBuf::from("sleep"), vec!["600".to_string()])
    }

    /// A child that stays alive long enough for bootstrap to bind its exit,
    /// then exits with `code` without ever publishing an endpoint record. The
    /// delay is what keeps the test off the "disappeared before its exit could
    /// be bound" branch, which is a different behavior with its own message.
    #[cfg(windows)]
    fn exits_with_code_command(code: i32) -> (PathBuf, Vec<String>) {
        (
            PathBuf::from("powershell.exe"),
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                format!("Start-Sleep -Milliseconds 500; exit {code}"),
            ],
        )
    }

    #[cfg(not(windows))]
    fn exits_with_code_command(code: i32) -> (PathBuf, Vec<String>) {
        (
            PathBuf::from("sh"),
            vec!["-c".to_string(), format!("sleep 1; exit {code}")],
        )
    }

    fn bootstrap_test_project(name: &str) -> PathBuf {
        record_test_project("daemon-bootstrap", name)
    }

    /// A real azd listening on an ephemeral loopback port. The bound address is
    /// read back from the server, so nothing here guesses or races for a port.
    fn live_daemon() -> az_daemon::AzDaemonRpcServer {
        az_daemon::start_az_daemon_rpc_server(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"))
            .unwrap()
    }

    #[test]
    fn daemon_args_register_project_roots_and_owner_lifecycle() {
        let editor_process = ProcessIdentity {
            process_id: 42,
            process_start_time: 9_001,
        };
        let project_root = std::env::temp_dir().join("azoth-daemon-bootstrap-project");
        let args = daemon_args(std::slice::from_ref(&project_root), editor_process).unwrap();
        let expected = vec![
            "--project".to_string(),
            project_root.to_string_lossy().into_owned(),
            "--editor-owner-process".to_string(),
            "42:9001".to_string(),
            "--shutdown-when-editor-leases-gone".to_string(),
        ];

        assert_eq!(args, expected);
    }

    #[test]
    fn sibling_daemon_executable_requires_sidecar_next_to_editor() {
        let temp = tempfile::tempdir().unwrap();
        let editor = temp.path().join(if cfg!(windows) {
            "az-editor.exe"
        } else {
            "az-editor"
        });
        std::fs::write(&editor, b"editor").unwrap();

        assert_eq!(sibling_daemon_executable_from(&editor), None);

        let daemon = temp.path().join(daemon_executable_name());
        std::fs::write(&daemon, b"daemon").unwrap();

        assert_eq!(sibling_daemon_executable_from(&editor), Some(daemon));
    }

    #[test]
    fn endpoint_match_accepts_requested_tcp_port_zero() {
        let record = Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:38981");
        let requested = Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:0");

        assert!(endpoint_matches_requested(&record, &requested));
    }

    #[test]
    fn port_zero_wildcard_does_not_bypass_endpoint_locality() {
        // Matching alone is permissive about the address when the requested
        // port is `:0`; the locality gate is what rejects a remote record.
        let remote = Endpoint::new(az_proto_core::EndpointKind::Tcp, "203.0.113.7:38981");
        let requested = Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:0");

        assert!(endpoint_matches_requested(&remote, &requested));
        assert!(ensure_daemon_endpoint_is_local(&remote).is_err());
    }

    // ---- transport-failure classification -------------------------------

    #[test]
    fn only_rpc_transport_errors_are_treated_as_daemon_transport_failures() {
        let transport = EditorError::RpcTransport(az_rpc::AzRpcTransportError::Io(
            std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        ));

        assert!(
            is_daemon_transport_failure(&transport),
            "a refused connection is the signal that no daemon is listening"
        );

        // Everything else means something answered, or the editor itself
        // failed. Bootstrap must surface those instead of racing to spawn a
        // second sidecar on top of a daemon that is already there.
        for error in [
            EditorError::ServiceDiscovery(
                "azd unavailable until restarted: protocol preflight failed".to_string(),
            ),
            EditorError::ServiceAuthorityMismatch {
                service: "azd",
                operation: "endpoint discovery",
                reason: "daemon endpoint is not machine-local".to_string(),
            },
            EditorError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)),
        ] {
            assert!(
                !is_daemon_transport_failure(&error),
                "`{error}` must not be classified as a daemon transport failure"
            );
        }
    }

    #[test]
    fn probing_an_endpoint_with_no_listener_reports_a_transport_failure() {
        let error = probe_daemon_endpoint(&unreachable_local_endpoint()).unwrap_err();

        assert!(
            matches!(error, EditorError::RpcTransport(_)),
            "expected an RPC transport error, got {error:?}"
        );
        assert!(
            is_daemon_transport_failure(&error),
            "an absent listener is what makes bootstrap start the sidecar"
        );
    }

    // ---- sidecar spawn ---------------------------------------------------

    #[test]
    fn spawning_a_missing_sidecar_executable_reports_a_not_found_io_error() {
        let temp = tempfile::tempdir().unwrap();
        let command = launch_command(
            temp.path().join(daemon_executable_name()),
            vec!["--project".to_string(), "unused".to_string()],
            temp.path(),
        );
        let (stdout, stderr) = open_log(&temp.path().join("azd.log"));

        let error = spawn_daemon_process(&command, stdout, stderr).unwrap_err();

        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "expected a not-found spawn failure, got {error:?}"
        );

        // `start_daemon_sidecar` propagates this with `?`, so the bootstrap
        // surface for a missing sidecar binary is `EditorError::Io` — not a
        // transport failure, and so never a reason to retry the spawn.
        let editor_error = EditorError::from(error);
        assert!(
            matches!(editor_error, EditorError::Io(_)),
            "expected EditorError::Io, got {editor_error:?}"
        );
        assert!(!is_daemon_transport_failure(&editor_error));
    }

    #[test]
    fn spawned_sidecar_redirects_stdout_into_the_project_daemon_log() {
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("azd.log");
        let (program, args) = echo_command("daemon-bootstrap-stub-alive");
        let command = launch_command(program, args, temp.path());
        let (stdout, stderr) = open_log(&log_path);

        let mut child = spawn_daemon_process(&command, stdout, stderr).unwrap();
        let status = child.wait().unwrap();

        assert!(status.success(), "stub sidecar exited with {status:?}");
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("daemon-bootstrap-stub-alive"),
            "sidecar stdout must land in the project azd log; log was `{log}`"
        );
    }

    #[test]
    fn spawned_sidecar_runs_in_the_launch_command_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("azd.log");
        let marker = "cwd-probe-file";
        std::fs::write(temp.path().join(marker), b"present").unwrap();
        // `type`/`cat` resolves `marker` relative to the child's cwd, so the
        // read only succeeds when `current_dir` was honored.
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from("cmd.exe"),
            vec!["/C".to_string(), format!("type {marker}")],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            PathBuf::from("sh"),
            vec!["-c".to_string(), format!("cat {marker}")],
        );
        let command = launch_command(program, args, temp.path());
        let (stdout, stderr) = open_log(&log_path);

        let mut child = spawn_daemon_process(&command, stdout, stderr).unwrap();
        let status = child.wait().unwrap();

        assert!(status.success(), "cwd probe exited with {status:?}");
        assert!(
            std::fs::read_to_string(&log_path)
                .unwrap()
                .contains("present"),
            "sidecar must run with `cwd` as its working directory"
        );
    }

    // ---- per-OS background-process configuration -------------------------

    #[test]
    fn background_process_config_preserves_the_program_and_arguments() {
        let mut process = Command::new("azd-under-test");
        process.args(["--project", "root"]);

        configure_background_process(&mut process);

        assert_eq!(process.get_program(), "azd-under-test");
        assert_eq!(
            process.get_args().collect::<Vec<_>>(),
            vec!["--project", "root"],
            "background configuration is platform flags only; it must never \
             rewrite what gets launched"
        );
    }

    /// Windows is the only platform whose branch sets anything, so it is the
    /// only one with flags to pin. The expected bit values are the documented
    /// Win32 process-creation constants, not a restatement of the source.
    #[cfg(windows)]
    #[test]
    fn windows_sidecar_creation_flags_match_documented_win32_values() {
        const DOCUMENTED_CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DOCUMENTED_CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DOCUMENTED_DETACHED_PROCESS: u32 = 0x0000_0008;

        assert_eq!(
            CREATE_NEW_PROCESS_GROUP,
            DOCUMENTED_CREATE_NEW_PROCESS_GROUP
        );
        assert_eq!(CREATE_NO_WINDOW, DOCUMENTED_CREATE_NO_WINDOW);

        assert_eq!(
            BACKGROUND_CREATION_FLAGS & DOCUMENTED_CREATE_NEW_PROCESS_GROUP,
            DOCUMENTED_CREATE_NEW_PROCESS_GROUP,
            "azd must leave the editor's console control group"
        );
        assert_eq!(
            BACKGROUND_CREATION_FLAGS & DOCUMENTED_CREATE_NO_WINDOW,
            DOCUMENTED_CREATE_NO_WINDOW,
            "azd must not flash a console window behind a GUI editor launch"
        );
        assert_eq!(
            BACKGROUND_CREATION_FLAGS & DOCUMENTED_DETACHED_PROCESS,
            0,
            "bootstrap waits on the child, so azd must stay an attached child \
             process rather than a detached one"
        );
        assert_eq!(
            BACKGROUND_CREATION_FLAGS
                & !(DOCUMENTED_CREATE_NEW_PROCESS_GROUP | DOCUMENTED_CREATE_NO_WINDOW),
            0,
            "no creation flag beyond the two the sidecar needs"
        );
    }

    /// The non-Windows branch deliberately sets no platform flags, so there is
    /// no runtime state to observe. This pins that absence explicitly rather
    /// than letting the branch go silently uncovered: the config is reachable,
    /// is a no-op, and a sidecar still spawns under it (see the spawn tests,
    /// which run on every platform).
    // ---- probing a live daemon -------------------------------------------

    #[test]
    fn probing_a_live_daemon_succeeds_through_the_protocol_preflight() {
        let server = live_daemon();
        let endpoint = server.endpoint().clone();

        probe_daemon_endpoint(&endpoint)
            .expect("probe must succeed against a current-protocol azd");

        server.stop();
    }

    #[test]
    fn probing_a_listener_that_never_speaks_the_protocol_is_not_a_transport_failure() {
        // A socket that accepts and immediately closes is the "something is
        // bound here, but it is not an azd" case. It must not look like an
        // absent daemon, because starting a second sidecar on top of whatever
        // owns that address is not a recovery.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::new(
            EndpointKind::Tcp,
            listener.local_addr().unwrap().to_string(),
        );
        let accepting = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });

        let error = probe_daemon_endpoint(&endpoint).unwrap_err();

        accepting.join().unwrap();
        assert!(
            matches!(&error, EditorError::ServiceDiscovery(message)
                if message.contains("protocol preflight failed")),
            "expected a preflight failure, got {error:?}"
        );
        assert!(
            !is_daemon_transport_failure(&error),
            "a peer that answered is not an absent daemon"
        );
    }

    /// A peer that accepts the connection and then says nothing: the case with
    /// no natural end, since neither side ever closes the socket. The probe's
    /// own deadline is the only thing that ends it, and
    /// `ensure_daemon_endpoint_for_project` probes before it does anything
    /// else, so this bound is what keeps a wedged daemon from stalling editor
    /// startup outright.
    ///
    /// Classifying the timeout as a transport failure is the other half: it is
    /// what routes a wedged peer into the sidecar spawn instead of failing the
    /// bootstrap. Nothing here caps the wait from above — the assertion is that
    /// the deadline elapsed, not that it elapsed promptly.
    #[test]
    fn probing_a_wedged_daemon_fails_within_its_own_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::new(
            EndpointKind::Tcp,
            listener.local_addr().unwrap().to_string(),
        );
        let (release, released) = std::sync::mpsc::channel::<()>();

        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            // Accepted, and then not one word of protocol.
            let _ = released.recv();
            drop(stream);
        });

        let started = Instant::now();
        let error = probe_daemon_endpoint(&endpoint).unwrap_err();
        let elapsed = started.elapsed();

        // The peer only ever closes on this signal, so the probe returning at
        // all is the deadline firing rather than a hung-up connection.
        release.send(()).unwrap();
        peer.join().unwrap();

        assert!(
            elapsed >= Duration::from_millis(EDITOR_DAEMON_PROBE_TIMEOUT_MS),
            "the probe must wait out its own deadline before giving up; \
             returned after {elapsed:?}"
        );
        assert!(
            matches!(&error, EditorError::RpcTransport(_)),
            "expected a transport failure, got {error:?}"
        );
        assert!(
            is_daemon_transport_failure(&error),
            "a wedged peer must route to the sidecar spawn, not fail bootstrap"
        );
        assert!(
            error
                .to_string()
                .contains("did not answer the bootstrap probe"),
            "the error must name the deadline it exceeded; got `{error}`"
        );
    }

    // ---- runtime endpoint records ----------------------------------------

    #[test]
    fn a_project_with_no_endpoint_record_has_no_reachable_runtime() {
        let root = bootstrap_test_project("absent-record");

        assert!(reachable_runtime_record(&root).unwrap().is_none());
    }

    #[test]
    fn a_record_pointing_at_a_live_daemon_is_reachable_and_survives() {
        let root = bootstrap_test_project("live-record");
        let server = live_daemon();
        let endpoint = server.endpoint().clone();
        let _record =
            az_endpoint_discovery::write_project_daemon_endpoint_record(&root, &endpoint).unwrap();

        let record = reachable_runtime_record(&root)
            .unwrap()
            .expect("a record whose daemon answers the probe is reachable");

        assert_eq!(record.endpoint, endpoint);
        assert!(
            read_project_daemon_endpoint_record(&root)
                .unwrap()
                .is_some(),
            "a reachable record must not be evicted"
        );
        server.stop();
    }

    #[test]
    fn a_record_whose_daemon_is_gone_is_evicted_before_the_sidecar_bootstrap() {
        let root = bootstrap_test_project("stale-record");
        let _record = az_endpoint_discovery::write_project_daemon_endpoint_record(
            &root,
            &unreachable_local_endpoint(),
        )
        .unwrap();
        assert!(
            read_project_daemon_endpoint_record(&root)
                .unwrap()
                .is_some(),
            "test setup must leave a record on disk"
        );

        assert!(
            reachable_runtime_record(&root).unwrap().is_none(),
            "an unreachable record means no runtime, not an error"
        );
        assert!(
            read_project_daemon_endpoint_record(&root)
                .unwrap()
                .is_none(),
            "the stale record must be removed so the sidecar can republish"
        );
    }

    /// A wedged daemon takes the same route as a dead one. The record probe is
    /// the first thing bootstrap does, so before the probe had a deadline this
    /// call never returned; now the timeout classifies as a transport failure
    /// and lands on the existing unreachable-record path — evicted, no error,
    /// sidecar free to republish. Evicting is the coherent choice: the record's
    /// only claim is "a daemon is answering here", and a peer that will not
    /// answer within the deadline has falsified it just as an absent one does.
    #[test]
    fn a_record_whose_daemon_is_wedged_is_evicted_like_an_unreachable_one() {
        let root = bootstrap_test_project("wedged-record");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::new(
            EndpointKind::Tcp,
            listener.local_addr().unwrap().to_string(),
        );
        let (release, released) = std::sync::mpsc::channel::<()>();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = released.recv();
            drop(stream);
        });
        let _record =
            az_endpoint_discovery::write_project_daemon_endpoint_record(&root, &endpoint).unwrap();

        let started = Instant::now();
        let reachable = reachable_runtime_record(&root).unwrap();
        let elapsed = started.elapsed();

        release.send(()).unwrap();
        peer.join().unwrap();

        assert!(
            reachable.is_none(),
            "a daemon that will not answer is no runtime, and not an error"
        );
        assert!(
            elapsed >= Duration::from_millis(EDITOR_DAEMON_PROBE_TIMEOUT_MS),
            "the record probe carries the same deadline; returned after {elapsed:?}"
        );
        assert!(
            read_project_daemon_endpoint_record(&root)
                .unwrap()
                .is_none(),
            "a wedged record must be evicted so the sidecar can republish"
        );
    }

    #[test]
    fn a_record_naming_a_remote_host_fails_fast_and_is_left_on_disk() {
        let root = bootstrap_test_project("remote-record");
        let _record = az_endpoint_discovery::write_project_daemon_endpoint_record(
            &root,
            &Endpoint::new(EndpointKind::Tcp, "203.0.113.7:38981"),
        )
        .unwrap();

        let error = reachable_runtime_record(&root).unwrap_err();

        assert!(
            matches!(error, EditorError::ServiceAuthorityMismatch { .. }),
            "a project directory must not redirect the editor off-machine; got {error:?}"
        );
        assert!(
            read_project_daemon_endpoint_record(&root)
                .unwrap()
                .is_some(),
            "a remote record is a trust violation rather than staleness, so \
             bootstrap fails without touching it"
        );
    }

    /// A record left by a *different protocol build* of azd is stale first and
    /// foreign second: the daemon that wrote it is usually long gone, so its
    /// recorded protocol is a claim about nobody. While the protocol gate ran
    /// ahead of the probe this was a hard failure that evicted nothing, and the
    /// machine-wide record could only be cleared by hand. It now takes the same
    /// eviction path as any other record whose endpoint does not answer.
    #[test]
    fn a_stale_foreign_protocol_record_is_evicted_like_any_other_stale_record() {
        let root = bootstrap_test_project("stale-protocol-record");
        let record_path = write_foreign_protocol_record(&root, &unreachable_local_endpoint());
        assert!(
            record_path.is_file(),
            "test setup must leave a record on disk"
        );

        assert!(
            reachable_runtime_record(&root).unwrap().is_none(),
            "a foreign-protocol record whose daemon is gone is no runtime, and \
             not an error"
        );
        assert!(
            !record_path.is_file(),
            "the stale record must be removed so the sidecar can republish"
        );
    }

    /// The other half of that split. A foreign-protocol record at an address
    /// where a daemon *is* answering is a real version conflict: bootstrap
    /// fails fast and leaves the record alone, because evicting it would start
    /// a second daemon on top of a live one and hide the conflict from the
    /// user.
    ///
    /// The live peer is a current-protocol azd, since that is the only kind
    /// this build can start; the mismatch is therefore the recorded field
    /// against `ProtocolVersion::CURRENT`, which is exactly what
    /// `validate_daemon_endpoint_record_protocol` compares. A peer that truly
    /// speaks another protocol fails one gate earlier, inside the probe's own
    /// preflight — a non-transport error, so it is likewise surfaced rather
    /// than evicted (see
    /// `probing_a_listener_that_never_speaks_the_protocol_is_not_a_transport_failure`).
    #[test]
    fn a_live_daemon_under_a_foreign_protocol_record_fails_fast_and_is_left_on_disk() {
        let root = bootstrap_test_project("live-protocol-record");
        let server = live_daemon();
        let record_path = write_foreign_protocol_record(&root, server.endpoint());

        let outcome = reachable_runtime_record(&root);

        server.stop();
        let error = outcome.expect_err("a live protocol conflict must fail the bootstrap");
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
            record_path.is_file(),
            "a live version conflict is for the user to resolve, so bootstrap \
             fails without touching the record"
        );

        std::fs::remove_file(&record_path).unwrap();
    }

    // ---- waiting for the sidecar to become ready -------------------------

    #[test]
    fn a_sidecar_that_never_publishes_times_out_and_is_reaped() {
        const TIMEOUT_MS: u64 = 250;

        let root = bootstrap_test_project("start-timeout");
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("azd.log");
        let (program, args) = never_ready_command();
        let command = launch_command(program, args, temp.path());
        let (stdout, stderr) = open_log(&log_path);
        let mut child = spawn_daemon_process(&command, stdout, stderr).unwrap();
        assert!(
            matches!(child.try_wait(), Ok(None)),
            "the stub must still be running going in, or the reaping assertion \
             below proves nothing"
        );

        let started = Instant::now();
        let error = wait_for_daemon_start(
            &root,
            &mut child,
            &command,
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            TIMEOUT_MS,
            &log_path,
        )
        .unwrap_err();
        let elapsed = started.elapsed();

        assert!(
            matches!(&error, EditorError::ServiceDiscovery(message)
                if message.contains("did not publish a reachable endpoint within 250ms")
                    && message.contains("azd.log")),
            "expected a startup timeout naming the log, got {error:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(TIMEOUT_MS),
            "bootstrap must wait out the configured timeout; waited {elapsed:?}"
        );
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "a timed-out sidecar must be killed and reaped, never left running"
        );
    }

    #[test]
    fn a_sidecar_that_exits_before_publishing_reports_its_status_and_log() {
        let root = bootstrap_test_project("start-exit");
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("azd.log");
        let (program, args) = exits_with_code_command(3);
        let command = launch_command(program, args, temp.path());
        let (stdout, stderr) = open_log(&log_path);
        let mut child = spawn_daemon_process(&command, stdout, stderr).unwrap();

        // Generous: the child decides when this returns, not the clock.
        let error = wait_for_daemon_start(
            &root,
            &mut child,
            &command,
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            120_000,
            &log_path,
        )
        .unwrap_err();

        assert!(
            matches!(&error, EditorError::ServiceDiscovery(message)
                if message.contains("exited before publishing a reachable endpoint")
                    && message.contains("status=Some(3)")
                    && message.contains("azd.log")),
            "expected an early-exit report carrying the status and log, got {error:?}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_sidecar_spawn_applies_no_platform_creation_flags() {
        let mut configured = Command::new("azd-under-test");
        configured.args(["--project", "root"]);
        configure_background_process(&mut configured);

        let untouched = Command::new("azd-under-test");

        assert_eq!(configured.get_program(), untouched.get_program());
        assert_eq!(
            configured.get_args().collect::<Vec<_>>(),
            vec!["--project", "root"],
            "the non-Windows branch adds nothing; only Windows has creation flags"
        );
    }
}
