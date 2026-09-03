use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use az_proto_core::{
    CapabilityGrantSet, Endpoint, EndpointKind, ServiceId, ServiceRole, encode_capability_grant_set,
};
use az_service_catalog::{
    ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS, DAEMON_SERVICE_NAME, DAEMON_SERVICE_NAMESPACE,
    PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS, asset_processor_service_descriptor,
    is_service_lifecycle_grant,
};
use az_service_supervision::{
    ServiceLifecycleController, ServiceReadyRecord, request_service_lifecycle_shutdown,
};
use uuid::Uuid;

struct ChildGuard(Option<Child>);

impl ChildGuard {
    const fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child")
    }

    fn wait_for_exit(&mut self, deadline: Instant) -> std::process::ExitStatus {
        loop {
            if let Some(status) = self.child_mut().try_wait().expect("query child") {
                self.0.take();
                return status;
            }
            assert!(Instant::now() < deadline, "asset-processor did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Splits the service descriptor's capabilities into role and lifecycle grant sets,
/// checks both against their brokered requirements, and writes them next to the test.
fn write_lifecycle_grant_files(
    temp: &std::path::Path,
    descriptor: &az_proto_core::ServiceDescriptor,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let role_grants = CapabilityGrantSet::from_grants(
        descriptor
            .capabilities
            .iter()
            .filter(|capability| !is_service_lifecycle_grant(capability))
            .cloned()
            .collect::<Vec<_>>(),
    );
    role_grants
        .validate_exact_brokered_for_project(ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS)
        .expect("exact role grants");
    let lifecycle_grants = CapabilityGrantSet::from_grants(
        descriptor
            .capabilities
            .iter()
            .filter(|capability| is_service_lifecycle_grant(capability))
            .cloned()
            .collect::<Vec<_>>(),
    );
    lifecycle_grants
        .validate_exact_brokered_for_project(PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS)
        .expect("exact lifecycle grants");

    let grants_dir = temp.join("grants");
    std::fs::create_dir_all(&grants_dir).expect("grant directory");
    let role_grants_path = grants_dir.join("asset-processor.capnp");
    let lifecycle_grants_path = grants_dir.join("asset-processor.lifecycle.capnp");
    std::fs::write(
        &role_grants_path,
        encode_capability_grant_set(&role_grants).expect("encode role grants"),
    )
    .expect("write role grants");
    std::fs::write(
        &lifecycle_grants_path,
        encode_capability_grant_set(&lifecycle_grants).expect("encode lifecycle grants"),
    )
    .expect("write lifecycle grants");
    (role_grants_path, lifecycle_grants_path)
}

#[test]
fn asset_processor_publishes_lifecycle_readiness_and_exits_on_authenticated_shutdown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let assets = workspace.join("assets");
    std::fs::create_dir_all(&assets).expect("assets");
    let project_id = "local.asset_processor_lifecycle";
    let project = az_project::ProjectManifest::new(
        project_id,
        "Asset Processor Lifecycle",
        env!("CARGO_PKG_VERSION"),
    );
    az_project::write_project_manifest(&workspace, &project).expect("write project manifest");
    az_project::refresh_project_lock(&workspace).expect("refresh project lock");

    let run = Uuid::now_v7();
    let mut descriptor =
        asset_processor_service_descriptor(run, Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"));
    let (role_grants_path, lifecycle_grants_path) =
        write_lifecycle_grant_files(temp.path(), &descriptor);

    let ready_file = temp.path().join("ready/asset-processor.toml");
    let structured_log = temp.path().join("logs/asset-processor.capnp.log");
    let asset_db = temp.path().join("assetdb/assetdb.sqlite");
    let data_home = temp.path().join("azoth-home");
    let child = Command::new(env!("CARGO_BIN_EXE_asset-processor"))
        .current_dir(&workspace)
        .env("AZOTH_HOME", &data_home)
        .args([
            "--endpoint-kind",
            "tcp",
            "--endpoint",
            "127.0.0.1:0",
            "--project",
            path_text(&workspace),
            "--project-id",
            project_id,
            "--owner-id",
            "project:local.asset_processor_lifecycle",
            "--owner-root",
            path_text(&data_home),
            "--workspace-root",
            path_text(&workspace),
            "--branch",
            "main",
            "--service",
            "asset-processor",
            "--run",
            &run.to_string(),
            "--ready-file",
            path_text(&ready_file),
            "--capability-grants",
            path_text(&role_grants_path),
            "--lifecycle-capability-grants",
            path_text(&lifecycle_grants_path),
            "--structured-log",
            path_text(&structured_log),
            "--lifecycle-endpoint-kind",
            "tcp",
            "--lifecycle-endpoint",
            "127.0.0.1:0",
            "--asset-db",
            path_text(&asset_db),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn asset-processor");
    let mut child = ChildGuard(Some(child));

    let ready = wait_for_ready(
        &mut child,
        &ready_file,
        Instant::now() + Duration::from_secs(20),
    );
    descriptor.endpoint = ready.endpoint();
    descriptor.lifecycle_endpoint = ready.lifecycle_endpoint();

    request_service_lifecycle_shutdown(
        &descriptor,
        &ServiceLifecycleController::new(
            ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
            ServiceRole::Daemon,
        ),
    )
    .expect("authenticated lifecycle shutdown");

    let status = child.wait_for_exit(Instant::now() + Duration::from_secs(20));
    assert!(status.success(), "asset-processor exited with {status}");
}

fn wait_for_ready(
    child: &mut ChildGuard,
    ready_file: &Path,
    deadline: Instant,
) -> ServiceReadyRecord {
    loop {
        if let Ok(text) = std::fs::read_to_string(ready_file) {
            return toml::from_str(&text).expect("decode ready record");
        }
        if let Some(status) = child.child_mut().try_wait().expect("query child") {
            panic!("asset-processor exited before readiness with {status}");
        }
        assert!(
            Instant::now() < deadline,
            "asset-processor did not become ready"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 test path")
}
