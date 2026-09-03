use std::path::{Path, PathBuf};
use std::time::Duration;

use az_filesystem::AzothDataHome;
use az_proto_core::{Endpoint, EndpointKind, ServiceRole};
use az_proto_runtime::RUNTIME_HOST_SERVICE_NAME;
use az_service_catalog::{asset_processor_service_descriptor, project_host_service_descriptor};
use az_service_supervision::{
    ServiceProcessKey, ServiceProcessState, StdServiceProcessLauncher, SupervisedServiceRole,
};
use az_session::{
    CreateSessionRequest, SessionManager, SessionServiceLaunchCommand, SessionServiceSupervisor,
};

const EXIT_CODE: i32 = 23;

/// A session carrying one planned runtime-host service whose program is the
/// exit-probe binary, plus the signal file that tells the probe to exit.
struct PlannedProbeSession {
    /// Dropped last; the project and the data home live inside it.
    _temp: tempfile::TempDir,
    manager: SessionManager,
    slug: String,
    planned: az_session::SessionManifest,
    exit_signal: PathBuf,
}

fn plan_exit_probe_session() -> PlannedProbeSession {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).unwrap();
    write_project_manifest(&project_root);

    let manager = SessionManager::with_data_home(
        &project_root,
        AzothDataHome::new(temp.path().join("data-home")),
    )
    .unwrap();
    let created = manager
        .create_session(CreateSessionRequest::new("Service Exit Evidence"))
        .unwrap();
    manager
        .attach_project_service_descriptors(
            &created.slug,
            &[
                asset_processor_service_descriptor(
                    uuid::Uuid::now_v7(),
                    Endpoint::new(EndpointKind::Tcp, "127.0.0.1:42000"),
                ),
                project_host_service_descriptor(
                    uuid::Uuid::now_v7(),
                    Endpoint::new(EndpointKind::Tcp, "127.0.0.1:42001"),
                ),
            ],
        )
        .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_az-session-exit-probe"));
    let exit_signal = temp.path().join("service-exit.signal");
    let build_output_root = program
        .parent()
        .and_then(Path::parent)
        .expect("probe binary must be under target/<profile>");
    let planned = manager
        .record_planned_services(
            &created.slug,
            &[SessionServiceLaunchCommand {
                owner_id: created.project_id.clone(),
                owner_root: project_root.clone(),
                build_output_root: build_output_root.to_path_buf(),
                service_name: RUNTIME_HOST_SERVICE_NAME.to_string(),
                role: ServiceRole::RuntimeHost,
                endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
                program: program.to_string_lossy().into_owned(),
                cwd: project_root,
                args: vec![
                    "--probe-exit-code".to_string(),
                    EXIT_CODE.to_string(),
                    "--probe-exit-signal".to_string(),
                    exit_signal.to_string_lossy().into_owned(),
                ],
            }],
        )
        .unwrap();
    PlannedProbeSession {
        _temp: temp,
        manager,
        slug: created.slug,
        planned,
        exit_signal,
    }
}

#[test]
fn production_supervisor_persists_attributed_service_exit() {
    let PlannedProbeSession {
        _temp,
        manager,
        slug,
        planned,
        exit_signal,
    } = plan_exit_probe_session();

    let planned_process = current_runtime_process(&planned);
    assert_eq!(planned_process.state, ServiceProcessState::Planned);

    let supervisor =
        SessionServiceSupervisor::with_manager(manager, StdServiceProcessLauncher::new())
            .with_ready_timeout(Duration::from_secs(5));
    let start = supervisor
        .start_planned_services(&slug)
        .unwrap_or_else(|error| {
            let manifest = supervisor.manager().session(&slug).unwrap();
            let process = current_runtime_process(&manifest);
            let stderr = std::fs::read_to_string(&process.stderr_log)
                .unwrap_or_else(|read_error| format!("<stderr unavailable: {read_error}>"));
            panic!("probe startup failed: {error}; stderr: {stderr}");
        });
    assert_eq!(start.started.len(), 1);

    let running = supervisor.manager().session(&slug).unwrap();
    let running_process = current_runtime_process(&running);
    let observed_pid = running_process.pid.expect("running row must name its pid");
    let observed_start_time = running_process
        .process_start_time
        .expect("running row must carry its process start token");
    let observed_run = running_process.run;
    assert_eq!(running_process.role, SupervisedServiceRole::RuntimeHost);
    assert_eq!(running_process.state, ServiceProcessState::Running);
    std::fs::write(&exit_signal, []).unwrap();

    let exit = supervisor.wait_for_service_exit(&slug).unwrap();
    assert_eq!(exit.service_name, RUNTIME_HOST_SERVICE_NAME);
    assert_eq!(exit.exit_code, Some(EXIT_CODE));
    assert!(!exit.success);

    let persisted = supervisor.manager().session(&slug).unwrap();
    let terminal = current_runtime_process(&persisted);
    assert_eq!(terminal.service_name, RUNTIME_HOST_SERVICE_NAME);
    assert_eq!(terminal.role, SupervisedServiceRole::RuntimeHost);
    assert_eq!(terminal.run, observed_run);
    assert_eq!(terminal.state, ServiceProcessState::Failed);
    assert_eq!(terminal.exit_code, Some(EXIT_CODE));
    assert!(
        terminal
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("exited with status"))
    );

    eprintln!(
        "SERVICE_EXIT_EVIDENCE observable=StdServiceProcessLauncher::try_wait \
         session={} service={} role={:?} run={} pid={} process_start_time={} \
         result_state={:?} exit_code={} persisted_manifest={}",
        persisted.id,
        terminal.service_name,
        terminal.role,
        terminal.run,
        observed_pid,
        observed_start_time,
        terminal.state,
        EXIT_CODE,
        persisted.manifest_path().display(),
    );
}

fn current_runtime_process(
    manifest: &az_session::SessionManifest,
) -> &az_service_supervision::ServiceProcessRecord {
    let index = manifest
        .current_service_process_index(&ServiceProcessKey::new(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
        ))
        .expect("runtime-host process record must exist");
    &manifest.processes[index]
}

fn write_project_manifest(project_root: &Path) {
    let manifest = az_project::ProjectManifest::new(
        "local.service_exit_evidence",
        "Service Exit Evidence",
        env!("CARGO_PKG_VERSION"),
    );
    az_project::write_project_manifest(project_root, &manifest).unwrap();
    az_project::refresh_project_lock(project_root).unwrap();
}
