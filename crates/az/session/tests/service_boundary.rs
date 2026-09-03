use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};

use az_filesystem::AzothDataHome;
use az_filesystem::{project_asset_db_path, project_default_platform_product_cache_dir};
use az_proto_core::{CapabilityGrantSet, Endpoint, EndpointKind, ServiceDescriptor, ServiceRole};
use az_proto_runtime::RUNTIME_HOST_SERVICE_NAME;
use az_service_catalog::{
    asset_processor_service_descriptor, is_observability_control_grant, is_service_lifecycle_grant,
    project_host_service_descriptor, runtime_host_service_descriptor,
};
use az_service_supervision::{
    ProcessIdentity, ServiceEndpointKind, ServiceProcessExit, ServiceProcessKey,
    ServiceProcessLauncher, ServiceProcessRecord, ServiceProcessState, ServiceReadyRecord,
    SpawnedServiceProcess, SupervisedServiceRole,
};
use az_session::{
    CreateSessionRequest, ProtocolServiceBoundaryProbe, ServiceBoundaryProbe, SessionError,
    SessionId, SessionManager, SessionManifest, SessionServiceLaunchCommand,
    SessionServiceSupervisor,
};
use clap::Parser;

const PROJECT_ID: &str = "local.service_boundary_smoke";
const SESSION_SLUG: &str = "editor";
const BRANCH: &str = "az/session/editor";

fn role_capability_grants(descriptor: &ServiceDescriptor) -> CapabilityGrantSet {
    CapabilityGrantSet::from_grants(
        descriptor
            .capabilities
            .iter()
            .filter(|capability| {
                !is_observability_control_grant(capability)
                    && !is_service_lifecycle_grant(capability)
            })
            .cloned()
            .collect::<Vec<_>>(),
    )
}

#[test]
fn protocol_boundary_probe_accepts_real_service_hosts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace_root = temp.path().join("workspace");
    let run_dir = temp.path().join("run");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::create_dir_all(&run_dir).unwrap();
    write_project_manifest(&workspace_root);

    let session_id = SessionId::new();
    let mut manifest = SessionManifest::new(
        session_id,
        PROJECT_ID.to_string(),
        SESSION_SLUG.to_string(),
        temp.path().to_path_buf(),
        workspace_root.clone(),
        run_dir,
        1,
    );
    manifest.activate(2);

    let probe = ProtocolServiceBoundaryProbe::new();
    let asset_db = session_asset_db_path(&manifest);
    let project_host = start_project_host(&mut manifest, &workspace_root);
    let asset_processor = start_asset_processor(&mut manifest, &asset_db, &workspace_root);
    let runtime_host = start_runtime_host(&mut manifest);

    let runtime_report = probe_descriptor(
        &probe,
        &manifest,
        &runtime_host.descriptor,
        "runtime-host",
        SupervisedServiceRole::RuntimeHost,
    );
    assert_eq!(
        runtime_report.checks,
        [
            "runtime-host.projectionCatalog",
            "runtime-host.projectHostRuntimeControlGrant"
        ]
    );

    project_host.server.stop().unwrap();
    asset_processor.server.stop().unwrap();
    runtime_host.server.stop().unwrap();
}

#[test]
fn supervisor_startup_accepts_real_service_hosts_after_protocol_probes() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("project");
    write_project_manifest(&project_root);

    let manager = SessionManager::with_data_home(
        &project_root,
        AzothDataHome::new(temp.path().join("azoth-home")),
    )
    .unwrap();
    let mut manifest = manager
        .create_session(CreateSessionRequest::new(SESSION_SLUG))
        .unwrap();
    write_project_manifest(&manifest.workspace_root);

    let placeholder = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
    let project_host = start_project_host(&mut manifest, &project_root);
    let asset_processor = start_asset_processor(
        &mut manifest,
        &temp.path().join("assetdb.sqlite"),
        &project_root,
    );
    manager
        .attach_project_service_descriptors(
            SESSION_SLUG,
            &[
                project_host.descriptor.clone(),
                asset_processor.descriptor.clone(),
            ],
        )
        .unwrap();
    let runtime = manager
        .register_runtime_host_descriptor(SESSION_SLUG, placeholder)
        .unwrap();
    manager
        .record_service_launch_plan(
            SESSION_SLUG,
            &[launch_command(
                RUNTIME_HOST_SERVICE_NAME,
                ServiceRole::RuntimeHost,
                &runtime.descriptor.endpoint,
                &manifest.workspace_root,
            )],
        )
        .unwrap();

    let launcher = SmokeServiceLauncher::new(manifest.id);
    let supervisor = SessionServiceSupervisor::with_manager_and_probe(
        manager,
        launcher,
        ProtocolServiceBoundaryProbe::new(),
    );

    let report = supervisor.start_planned_services(SESSION_SLUG).unwrap();

    assert_eq!(
        report
            .started
            .iter()
            .map(|process| process.service_name.as_str())
            .collect::<Vec<_>>(),
        vec![RUNTIME_HOST_SERVICE_NAME]
    );
    let manifest = supervisor.manager().session(SESSION_SLUG).unwrap();
    let process = manifest
        .processes
        .iter()
        .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
        .unwrap();
    assert_eq!(process.state, ServiceProcessState::Running);
    assert_ne!(process.endpoint_address, "127.0.0.1:0");
    assert!(manifest.processes.iter().all(|process| {
        !matches!(
            process.role,
            SupervisedServiceRole::ProjectHost
                | SupervisedServiceRole::AssetProcessor
                | SupervisedServiceRole::Worker
        )
    }));

    let stop = supervisor
        .stop_owned_services(SESSION_SLUG, "service-boundary smoke complete")
        .unwrap();
    assert_eq!(stop.stopped.len(), 1);
    project_host.server.stop().unwrap();
    asset_processor.server.stop().unwrap();
}

struct StartedProjectHost {
    descriptor: ServiceDescriptor,
    server: az_project_host::ProjectHostRpcServer,
}

struct StartedAssetProcessor {
    descriptor: ServiceDescriptor,
    server: az_asset_processor::AssetProcessorRpcServer,
}

struct StartedRuntimeHost {
    descriptor: ServiceDescriptor,
    server: az_runtime_host::RuntimeHostRpcServer,
}

struct SmokeServiceLauncher {
    session_id: SessionId,
    servers: RefCell<BTreeMap<ServiceProcessKey, SmokeRuntimeHost>>,
}

struct SmokeRuntimeHost {
    server: Option<az_runtime_host::RuntimeHostRpcServer>,
    lifecycle_control: Option<az_service_entrypoint::ServiceLifecycleControlServer>,
    lifetime_result: Receiver<Result<(), az_service_entrypoint::ServiceLifetimeError>>,
    lifetime_thread: Option<JoinHandle<()>>,
    sentinel: Child,
}

impl SmokeRuntimeHost {
    fn stop(&mut self) -> bool {
        let mut success = true;
        if let Some(server) = self.server.take() {
            success &= server.stop().is_ok();
        }
        if let Some(lifecycle_control) = self.lifecycle_control.take() {
            success &= lifecycle_control.stop().is_ok();
        }
        let _ = self.sentinel.kill();
        let _ = self.sentinel.wait();
        if let Some(lifetime_thread) = self.lifetime_thread.take() {
            success &= lifetime_thread.join().is_ok();
        }
        success
    }
}

impl Drop for SmokeRuntimeHost {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl SmokeServiceLauncher {
    const fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            servers: RefCell::new(BTreeMap::new()),
        }
    }
}

impl SmokeServiceLauncher {
    /// The runtime-host half of `spawn`: stand up an in-process runtime host, a
    /// lifecycle control, and a sentinel child that gives the record a real OS
    /// process identity.
    fn spawn_runtime_host(
        &self,
        process: &ServiceProcessRecord,
        endpoint: Endpoint,
        grants: PathBuf,
    ) -> Result<SpawnedServiceProcess, SessionError> {
        let side_channel_root = required_arg(&process.args, "--side-channel-root")?;
        assert!(PathBuf::from(&side_channel_root).is_absolute());
        let server =
            az_runtime_host::start_runtime_host_rpc_server_for_session_with_capability_grant_file(
                endpoint,
                self.session_id.0.to_string(),
                side_channel_root,
                process.run,
                grants,
                runtime_host_registries(),
            )
            .map_err(|error| spawn_error(process, error))?;
        let endpoint = server.endpoint().clone();
        let mut argv = vec![process.program.clone()];
        argv.extend(process.args.iter().cloned());
        let service_args = az_service_entrypoint::RuntimeHostCli::try_parse_from(argv)
            .map_err(|error| spawn_error(process, error))?
            .into_service_args()
            .map_err(|error| spawn_error(process, error))?;
        let (lifecycle_control, lifetime) =
            match az_service_entrypoint::start_service_lifecycle_control(&service_args) {
                Ok(started) => started,
                Err(error) => {
                    let _ = server.stop();
                    return Err(spawn_error(process, error));
                }
            };
        let lifecycle_endpoint = lifecycle_control.endpoint().clone();
        let mut sentinel = match spawn_smoke_service_sentinel() {
            Ok(sentinel) => sentinel,
            Err(source) => {
                let _ = server.stop();
                let _ = lifecycle_control.stop();
                return Err(SessionError::ServiceProcessSpawnFailed {
                    service: process.service_name.clone(),
                    program: process.program.clone(),
                    source,
                });
            }
        };
        let identity = match ProcessIdentity::capture(sentinel.id()) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let _ = sentinel.kill();
                let _ = sentinel.wait();
                let _ = server.stop();
                let _ = lifecycle_control.stop();
                return Err(SessionError::ServiceProcessSpawnFailed {
                    service: process.service_name.clone(),
                    program: process.program.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "smoke service sentinel disappeared before identity capture",
                    ),
                });
            }
            Err(source) => {
                let _ = sentinel.kill();
                let _ = sentinel.wait();
                let _ = server.stop();
                let _ = lifecycle_control.stop();
                return Err(SessionError::ServiceProcessSpawnFailed {
                    service: process.service_name.clone(),
                    program: process.program.clone(),
                    source,
                });
            }
        };
        let (lifetime_result_tx, lifetime_result) = mpsc::sync_channel(1);
        let lifetime_thread = thread::spawn(move || {
            let _ = lifetime_result_tx.send(lifetime.wait());
        });
        let mut smoke_host = SmokeRuntimeHost {
            server: Some(server),
            lifecycle_control: Some(lifecycle_control),
            lifetime_result,
            lifetime_thread: Some(lifetime_thread),
            sentinel,
        };
        if let Err(error) =
            write_ready_file(process, &endpoint, &lifecycle_endpoint, identity.process_id)
        {
            let _ = smoke_host.stop();
            return Err(error);
        }
        self.servers
            .borrow_mut()
            .insert(ServiceProcessKey::from_process(process), smoke_host);
        Ok(SpawnedServiceProcess {
            service_name: process.service_name.clone(),
            role: process.role,
            run: process.run,
            identity,
        })
    }
}

impl ServiceProcessLauncher for SmokeServiceLauncher {
    type Error = SessionError;

    fn spawn(&self, process: &ServiceProcessRecord) -> Result<SpawnedServiceProcess, SessionError> {
        let endpoint = Endpoint::new(process.endpoint_kind.to_proto(), &process.endpoint_address);
        let grants = required_arg(&process.args, "--capability-grants")?;
        match process.role {
            SupervisedServiceRole::RuntimeHost => {
                self.spawn_runtime_host(process, endpoint, grants)
            }
            role => Err(SessionError::ServiceProcessSpawnFailed {
                service: process.service_name.clone(),
                program: process.program.clone(),
                source: std::io::Error::other(format!("unsupported smoke service role {role:?}")),
            }),
        }
    }

    fn is_tracking(&self, process: &ServiceProcessKey) -> bool {
        self.servers.borrow().contains_key(process)
    }

    fn try_wait(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, SessionError> {
        let lifetime_result = {
            let servers = self.servers.borrow();
            let Some(server) = servers.get(process) else {
                return Ok(None);
            };
            match server.lifetime_result.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(
                    az_service_entrypoint::ServiceLifetimeError::ControlLost,
                )),
            }
        };
        let Some(lifetime_result) = lifetime_result else {
            return Ok(None);
        };
        let mut server = self
            .servers
            .borrow_mut()
            .remove(process)
            .expect("tracked smoke runtime host");
        let stopped = server.stop();
        Ok(Some(ServiceProcessExit {
            service_name: process.service_name.clone(),
            exit_code: None,
            success: lifetime_result.is_ok() && stopped,
        }))
    }

    fn terminate(
        &self,
        process: &ServiceProcessKey,
    ) -> Result<Option<ServiceProcessExit>, SessionError> {
        let Some(mut server) = self.servers.borrow_mut().remove(process) else {
            return Ok(None);
        };
        let success = server.stop();
        Ok(Some(ServiceProcessExit {
            service_name: process.service_name.clone(),
            exit_code: None,
            success,
        }))
    }
}

#[cfg(windows)]
fn spawn_smoke_service_sentinel() -> std::io::Result<Child> {
    Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 300",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(unix)]
fn spawn_smoke_service_sentinel() -> std::io::Result<Child> {
    Command::new("sh")
        .args(["-c", "sleep 300"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn start_project_host(manifest: &mut SessionManifest, workspace_root: &Path) -> StartedProjectHost {
    let descriptor = project_host_service_descriptor(
        uuid::Uuid::now_v7(),
        Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
    );
    let grants_path = manifest.run_dir.join("project-host.capnp.packed");
    std::fs::create_dir_all(&manifest.run_dir).unwrap();
    std::fs::write(
        &grants_path,
        az_proto_core::encode_capability_grant_set(&role_capability_grants(&descriptor)).unwrap(),
    )
    .unwrap();
    let server = az_project_host::start_project_host_rpc_server_for_project_session_with_capability_grant_file(
        workspace_root,
        descriptor.endpoint.clone(),
        PROJECT_ID,
        manifest.id.0.to_string(),
        &grants_path,
    )
    .unwrap();
    let descriptor = with_bound_endpoint(descriptor, server.endpoint().clone());
    manifest.upsert_service_descriptor(&descriptor, 3).unwrap();
    StartedProjectHost { descriptor, server }
}

// The registered DB handle is moved into the RPC server below, so clippy's
// "drop it after its last use" rewrite would be a use-after-move.
#[allow(clippy::significant_drop_tightening)]
fn start_asset_processor(
    manifest: &mut SessionManifest,
    db_path: &Path,
    workspace_root: &Path,
) -> StartedAssetProcessor {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let registered_db = az_asset_processor::open_registered_workspace_asset_db(
        db_path,
        PROJECT_ID,
        workspace_root,
        BRANCH,
        now_unix_ms_i64(),
    )
    .unwrap();
    assert!(
        !registered_db.registration().source_roots.is_empty(),
        "asset-processor smoke setup must register DB-owned source roots"
    );

    let descriptor = asset_processor_service_descriptor(
        uuid::Uuid::now_v7(),
        Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
    );
    let grants_path = manifest.run_dir.join("asset-processor.capnp.packed");
    std::fs::write(
        &grants_path,
        az_proto_core::encode_capability_grant_set(&role_capability_grants(&descriptor)).unwrap(),
    )
    .unwrap();
    let server = az_asset_processor::start_asset_processor_rpc_server_with_registered_workspace_db(
        registered_db,
        descriptor.endpoint.clone(),
        descriptor.run,
        &grants_path,
    )
    .unwrap();
    let descriptor = with_bound_endpoint(descriptor, server.endpoint().clone());
    manifest.upsert_service_descriptor(&descriptor, 5).unwrap();
    StartedAssetProcessor { descriptor, server }
}

/// A runtime-host process serves the projections it composed, and this probe
/// composes none: it is testing the service boundary, not the projections.
fn runtime_host_registries() -> &'static az_gem_contract::Registries {
    static REGISTRIES: std::sync::OnceLock<az_gem_contract::Registries> =
        std::sync::OnceLock::new();
    REGISTRIES.get_or_init(az_gem_contract::Registries::new)
}

fn start_runtime_host(manifest: &mut SessionManifest) -> StartedRuntimeHost {
    let descriptor = runtime_host_service_descriptor(
        manifest.id.0,
        uuid::Uuid::now_v7(),
        Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
    );
    let grants = role_capability_grants(&descriptor);
    let server = az_runtime_host::start_runtime_host_rpc_server_for_session_with_capability_grants(
        descriptor.endpoint.clone(),
        manifest.id.0.to_string(),
        manifest.run_dir.join("side-channels").join("runtime-host"),
        grants,
        runtime_host_registries(),
    )
    .unwrap();
    let descriptor = with_bound_endpoint(descriptor, server.endpoint().clone());
    manifest.upsert_service_descriptor(&descriptor, 6).unwrap();
    StartedRuntimeHost { descriptor, server }
}

fn probe_descriptor(
    probe: &ProtocolServiceBoundaryProbe,
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    service_name: &str,
    role: SupervisedServiceRole,
) -> az_session::ServiceBoundaryProbeReport {
    let grants_path = manifest
        .run_dir
        .join(format!("{service_name}.capnp.packed"));
    std::fs::write(
        &grants_path,
        az_proto_core::encode_capability_grant_set(&role_capability_grants(descriptor)).unwrap(),
    )
    .unwrap();
    let mut args = vec![
        "--capability-grants".to_string(),
        grants_path.to_string_lossy().into_owned(),
    ];
    if matches!(
        role,
        SupervisedServiceRole::ProjectHost | SupervisedServiceRole::AssetProcessor
    ) {
        args.push("--asset-db".to_string());
        args.push(
            session_asset_db_path(manifest)
                .to_string_lossy()
                .into_owned(),
        );
    }
    if matches!(
        role,
        SupervisedServiceRole::ProjectHost | SupervisedServiceRole::RuntimeHost
    ) {
        args.push("--side-channel-root".to_string());
        args.push(
            manifest
                .run_dir
                .join("side-channels")
                .join(service_name)
                .to_string_lossy()
                .into_owned(),
        );
    }
    if role == SupervisedServiceRole::Worker {
        args.push("--staging-root".to_string());
        args.push(
            manifest
                .run_dir
                .join("staging")
                .join(service_name)
                .to_string_lossy()
                .into_owned(),
        );
        args.push("--cache-root".to_string());
        args.push(
            project_default_platform_product_cache_dir(
                &manifest.project_id,
                &manifest.workspace_root,
            )
            .to_string_lossy()
            .into_owned(),
        );
    }
    let process = ServiceProcessRecord::planned(
        service_name,
        role,
        descriptor.run,
        &descriptor.endpoint,
        service_name,
        manifest.workspace_root.clone(),
        args,
        manifest.run_dir.join(format!("{service_name}.stdout.log")),
        manifest.run_dir.join(format!("{service_name}.stderr.log")),
        PathBuf::new(),
        Some(manifest.run_dir.join(format!("{service_name}.ready.toml"))),
        7,
    );
    assert_eq!(process.state, ServiceProcessState::Planned);
    assert_eq!(role.to_proto(), descriptor.role);
    probe
        .verify_service_boundary(SESSION_SLUG, manifest, &process, descriptor)
        .unwrap()
}

fn with_bound_endpoint(mut descriptor: ServiceDescriptor, endpoint: Endpoint) -> ServiceDescriptor {
    descriptor.endpoint = endpoint;
    descriptor
}

/// The program path a planned service records — materialized, because the
/// manager fingerprints the artifact when it marks a service `Starting`.
///
/// The smoke launcher never executes it: what it stands in for is the file
/// `ServiceProgramArtifact::capture` reads. Without it, startup fails before
/// the launcher is reached, which is the second latent failure this test hit
/// once the builder floor stopped failing first.
fn test_service_program(root: &Path, service_name: &str) -> String {
    let directory = root.join("target").join("debug");
    std::fs::create_dir_all(&directory).unwrap();
    let program = directory.join(test_service_executable_name(service_name));
    if !program.exists() {
        std::fs::write(&program, b"service-boundary smoke placeholder").unwrap();
    }
    program.to_string_lossy().into_owned()
}

fn test_service_executable_name(service_name: &str) -> String {
    if cfg!(windows) {
        format!("{service_name}.exe")
    } else {
        service_name.to_string()
    }
}

fn launch_command(
    service_name: &str,
    role: ServiceRole,
    endpoint: &Endpoint,
    workspace_root: &Path,
) -> SessionServiceLaunchCommand {
    SessionServiceLaunchCommand {
        owner_id: "service-boundary-smoke".to_string(),
        owner_root: workspace_root.to_path_buf(),
        build_output_root: workspace_root.join("target"),
        service_name: service_name.to_string(),
        role,
        endpoint: endpoint.clone(),
        program: test_service_program(workspace_root, service_name),
        cwd: workspace_root.to_path_buf(),
        args: vec![
            "--service".to_string(),
            service_name.to_string(),
            "--branch".to_string(),
            BRANCH.to_string(),
        ],
    }
}

fn required_arg(args: &[String], flag: &str) -> Result<PathBuf, SessionError> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or_else(|| SessionError::ServiceProcessSpawnFailed {
            service: flag.to_string(),
            program: "service-boundary-smoke".to_string(),
            source: std::io::Error::other(format!("missing required argument {flag}")),
        })
}

fn session_asset_db_path(manifest: &SessionManifest) -> PathBuf {
    project_asset_db_path(&manifest.project_id, &manifest.project_root)
}

fn now_unix_ms_i64() -> i64 {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    i64::try_from(millis).unwrap()
}

fn write_ready_file(
    process: &ServiceProcessRecord,
    endpoint: &Endpoint,
    lifecycle_endpoint: &Endpoint,
    pid: u32,
) -> Result<(), SessionError> {
    let ready_file =
        process
            .ready_file
            .as_ref()
            .ok_or_else(|| SessionError::MissingServiceReadyFile {
                session: SESSION_SLUG.to_string(),
                service: process.service_name.clone(),
            })?;
    let mut ready = ServiceReadyRecord::new(
        &process.service_name,
        process.role,
        process.run,
        endpoint,
        Some(pid),
        9,
    );
    ready.lifecycle_endpoint_kind = Some(ServiceEndpointKind::from_proto(lifecycle_endpoint.kind));
    ready.lifecycle_endpoint_address = Some(lifecycle_endpoint.address.clone());
    match (
        optional_arg_value(&process.args, "--observability-endpoint-kind"),
        optional_arg_value(&process.args, "--observability-endpoint"),
    ) {
        (None, None) => {}
        (Some(kind), Some(address)) => {
            ready.observability_endpoint_kind = Some(ServiceEndpointKind::from_proto(
                parse_endpoint_kind_arg(process, kind)?,
            ));
            ready.observability_endpoint_address = Some(address.to_string());
        }
        _ => {
            return Err(spawn_error(
                process,
                "observability readiness requires both endpoint kind and address",
            ));
        }
    }
    if let Some(parent) = ready_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(&ready)?;
    std::fs::write(ready_file, text)?;
    Ok(())
}

fn optional_arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

fn parse_endpoint_kind_arg(
    process: &ServiceProcessRecord,
    kind: &str,
) -> Result<EndpointKind, SessionError> {
    match kind {
        "windows-named-pipe" => Ok(EndpointKind::WindowsNamedPipe),
        "unix-domain-socket" => Ok(EndpointKind::UnixDomainSocket),
        "tcp" => Ok(EndpointKind::Tcp),
        "in-process" => Ok(EndpointKind::InProcess),
        _ => Err(spawn_error(
            process,
            format!("unsupported endpoint kind `{kind}`"),
        )),
    }
}

fn spawn_error(process: &ServiceProcessRecord, error: impl std::fmt::Display) -> SessionError {
    SessionError::ServiceProcessSpawnFailed {
        service: process.service_name.clone(),
        program: process.program.clone(),
        source: std::io::Error::other(error.to_string()),
    }
}

fn write_project_manifest(workspace_root: &Path) {
    let mut manifest =
        az_project::ProjectManifest::new(PROJECT_ID, "Service Boundary Smoke", "0.1.0");
    manifest.paths.assets = PathBuf::from("assets");
    manifest.gems.push(az_project::ProjectGem {
        id: "azoth.smoke".to_string(),
        enabled: true,
        capabilities: Vec::new(),
        linkage: None,
        path: Some(PathBuf::from("gems").join("smoke")),
    });
    az_project::write_project_manifest(workspace_root, &manifest).unwrap();
    std::fs::create_dir_all(workspace_root.join("assets")).unwrap();

    let gem_root = workspace_root.join("gems").join("smoke");
    let mut gem = az_project::GemManifest::new("azoth.smoke", "Smoke Gem", "0.1.0");
    gem.paths.assets = PathBuf::from("assets");
    az_project::write_gem_manifest(&gem_root, &gem).unwrap();
    az_project::refresh_project_lock(workspace_root).unwrap();
    std::fs::create_dir_all(gem_root.join("assets")).unwrap();
}
