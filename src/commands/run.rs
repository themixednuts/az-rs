use crate::error::{CliError, CliResult};
use az_proto_core::EndpointKind;
use az_proto_runtime::RuntimeRole;
use std::path::{Path, PathBuf};
use tracing::info;

const RUNTIME_LAUNCH_OPERATION: &str = "project runtime launch";

#[allow(clippy::too_many_arguments)]
pub fn execute(
    session: Option<String>,
    runtime_id: &str,
    role: RuntimeRole,
    include_unsaved_journal: bool,
    launch_profile: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    az_project_scaffold::project_contract::sync_project_contract(&project_path)?;
    let resolved_daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint.clone(),
    )?;
    let session = resolve_run_session(&project_path, session, resolved_daemon_endpoint.as_ref())?;

    info!(
        session = %session,
        runtime_id = %runtime_id,
        root = %project_path.display(),
        "launching project runtime through session workflow"
    );

    crate::commands::session::launch_runtime(
        &session,
        runtime_id,
        role,
        include_unsaved_journal,
        launch_profile,
        daemon_endpoint_kind,
        daemon_endpoint,
        Some(project_path),
    )
}

fn resolve_run_session(
    project_path: &Path,
    requested: Option<String>,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> CliResult<String> {
    if let Some(requested) = requested {
        return Ok(requested);
    }

    let Some(daemon_endpoint) = daemon_endpoint else {
        return Err(CliError::MissingDaemonEndpoint {
            operation: RUNTIME_LAUNCH_OPERATION,
        });
    };

    crate::commands::session::active_session_slug_through_daemon(
        project_path,
        daemon_endpoint,
        RUNTIME_LAUNCH_OPERATION,
    )?
    .ok_or(CliError::MissingDaemonEndpoint {
        operation: RUNTIME_LAUNCH_OPERATION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_session::{SESSION_MANIFEST_FILE, SessionId, SessionManifest, SessionState};

    fn ensure_test_project_manifest(root: &Path) -> String {
        if let Ok(manifest) = az_project::load_project_manifest(root) {
            return manifest.project.id;
        }

        let manifest = az_project::ProjectManifest::new("local.run_test", "Run Test", "0.1.0");
        az_project::write_project_manifest(root, &manifest).unwrap();
        az_project::refresh_project_lock(root).unwrap();
        manifest.project.id
    }

    fn write_session_manifest(manifest: &SessionManifest) {
        std::fs::write(
            manifest.run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_session(root: &Path, slug: &str, state: SessionState) -> SessionManifest {
        let project_id = ensure_test_project_manifest(root);
        let session_id = SessionId::new();
        let run_dir = az_session::SessionManager::with_data_home(
            root,
            az_filesystem::AzothDataHome::new(root.join("azoth-home")),
        )
        .unwrap()
        .sessions_dir()
        .join(session_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut manifest = SessionManifest::new(
            session_id,
            project_id,
            slug.to_string(),
            root.to_path_buf(),
            root.to_path_buf(),
            run_dir,
            0,
        );
        manifest.state = state;
        write_session_manifest(&manifest);
        manifest
    }

    #[test]
    fn explicit_run_session_is_used_without_local_lookup() {
        let missing = std::env::temp_dir().join("azoth-explicit-session-missing-project");
        let path = missing.as_path();

        let session = resolve_run_session(path, Some("playtest".to_string()), None).unwrap();

        assert_eq!(session, "playtest");
    }

    #[test]
    fn run_requires_daemon_endpoint_to_infer_active_session() {
        let temp = tempfile::tempdir().unwrap();
        write_session(temp.path(), "main", SessionState::Active);
        write_session(temp.path(), "old", SessionState::FailedPreserved);

        let error = resolve_run_session(temp.path(), None, None).unwrap_err();

        assert!(matches!(
            error,
            CliError::MissingDaemonEndpoint {
                operation: "project runtime launch"
            }
        ));
    }

    #[test]
    fn run_does_not_fallback_to_local_manifests_for_no_active_daemon_session() {
        let temp = tempfile::tempdir().unwrap();
        write_session(temp.path(), "old", SessionState::FailedPreserved);

        let error = resolve_run_session(temp.path(), None, None).unwrap_err();

        assert!(matches!(
            error,
            CliError::MissingDaemonEndpoint {
                operation: "project runtime launch"
            }
        ));
    }

    #[test]
    fn run_does_not_fallback_to_local_manifests_for_ambiguous_local_sessions() {
        let temp = tempfile::tempdir().unwrap();
        write_session(temp.path(), "main", SessionState::Active);
        write_session(temp.path(), "lighting", SessionState::Active);

        let error = resolve_run_session(temp.path(), None, None).unwrap_err();

        assert!(matches!(
            error,
            CliError::MissingDaemonEndpoint {
                operation: "project runtime launch"
            }
        ));
    }

    #[test]
    fn run_session_inference_stays_on_daemon_boundary_without_local_manifest_lookup() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands/run.rs"))
                .expect("read run command source");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source section");

        assert!(production_source.contains("active_session_slug_through_daemon"));
        for forbidden in ["SessionManager::new", ".list_sessions()"] {
            assert!(
                !production_source.contains(forbidden),
                "azoth run must infer omitted sessions through daemon/session-supervisor discovery, not local manifests via `{forbidden}`"
            );
        }
    }

    #[test]
    fn run_selects_active_session_through_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project = az_project::ProjectManifest::new("local.run_daemon", "Run Daemon", "0.1.0");
        az_project::write_project_manifest(&project_root, &project).unwrap();
        az_project::refresh_project_lock(&project_root).unwrap();
        let mut supervisor_manifest =
            write_session(&project_root, "daemon-main", SessionState::Active);

        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            supervisor_manifest.id.0,
            uuid::Uuid::from_bytes([1; 16]),
            az_proto_core::Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        supervisor_manifest
            .upsert_service_descriptor(&descriptor, 1)
            .unwrap();
        write_session_manifest(&supervisor_manifest);
        let session_supervisor = az_session::start_session_supervisor_rpc_server_with_manager(
            az_session::SessionManager::with_data_home(
                &project_root,
                az_filesystem::AzothDataHome::new(project_root.join("azoth-home")),
            )
            .unwrap(),
            descriptor.endpoint.clone(),
            &supervisor_manifest.slug,
        )
        .unwrap();
        descriptor.endpoint = session_supervisor.endpoint().clone();
        supervisor_manifest
            .upsert_service_descriptor(&descriptor, 2)
            .unwrap();
        write_session_manifest(&supervisor_manifest);
        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
        .unwrap();
        daemon.register_project_root(&project_root).unwrap();
        daemon
            .register_session_supervisor("local.run_daemon", "daemon-main", &descriptor)
            .unwrap();
        let daemon_server = az_daemon::start_az_daemon_rpc_server_with_daemon(
            daemon,
            az_proto_core::Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: daemon_server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let session = resolve_run_session(&project_root, None, Some(&daemon_endpoint)).unwrap();

        assert_eq!(session, "daemon-main");
        session_supervisor.stop();
        daemon_server.stop();
    }
}
