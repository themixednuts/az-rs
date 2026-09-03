//! Editor-side `azd` client.
//!
//! The editor starts global discovery at the user daemon, then follows the
//! returned session-supervisor descriptor into per-session service discovery.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use az_endpoint_discovery::DaemonEndpointRecord;
use az_proto_core::{
    Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor, ServiceId, ServiceRole,
};
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_LEASE_PERMISSION, DAEMON_NAMESPACE, DAEMON_PROJECTS_PERMISSION,
    DAEMON_READ_PERMISSION, DAEMON_SERVICE_NAME, DAEMON_SESSIONS_PERMISSION,
    EnsureProjectSessionRequest, EnsureProjectSessionServicesRequest, ExecuteProjectBuildRequest,
    ListProjectsRequest, ListProjectsResult, ListSessionSupervisorsRequest,
    ListSessionSupervisorsResult, PlanProjectBuildRequest, PrepareProjectSessionServicesRequest,
    ProcessIdentity as ProtoProcessIdentity, ProjectBuildCommand, ProjectBuildExecutionResult,
    ProjectBuildPlan, ProjectRecord, ProjectResult, ProjectSessionResult,
    ProjectSessionServicesResult, ProjectSessionServicesStartResult, RegisterProjectRootRequest,
    ResolveProjectRequest, ResolveSessionSupervisorRequest, SessionSupervisorDescriptor,
    SessionSupervisorResult, TouchEditorLeaseRequest, TouchEditorLeaseResult, daemon_capnp,
    editor_process_lease_id,
};
use az_service_supervision::ProcessIdentity;

use crate::error::{EditorError, EditorResult};
use crate::project_host::{EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE};
use crate::session_supervisor::validate_session_supervisor_descriptor;

#[derive(Clone)]
pub struct AzDaemonClient {
    client: daemon_capnp::az_daemon::Client,
    editor_service: ServiceId,
}

impl AzDaemonClient {
    fn from_client(client: daemon_capnp::az_daemon::Client) -> Self {
        Self {
            client,
            editor_service: ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn new(client: daemon_capnp::az_daemon::Client) -> Self {
        Self::from_client(client)
    }

    #[cfg(test)]
    #[must_use]
    pub const fn with_editor_service(
        client: daemon_capnp::az_daemon::Client,
        editor_service: ServiceId,
    ) -> Self {
        Self {
            client,
            editor_service,
        }
    }

    /// Dial the `azd` at `endpoint` and hold it to the current protocol version
    /// before handing back a client.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::RpcTransport`] when the endpoint cannot be reached
    /// or the bootstrap handshake fails, and [`EditorError::ServiceDiscovery`]
    /// when the connected daemon fails the protocol preflight — the preflight
    /// call itself erroring, or the daemon reporting a protocol version other
    /// than [`ProtocolVersion::CURRENT`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn connect(endpoint: &Endpoint) -> EditorResult<Self> {
        let client = az_rpc::connect_twoparty_bootstrap(endpoint).await?;
        let daemon = Self::from_client(client);
        daemon.require_current_protocol().await?;
        Ok(daemon)
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn require_current_protocol(&self) -> EditorResult<()> {
        let mut request = self.client.list_projects_request();
        (ListProjectsRequest {
            capability: self.daemon_read_capability(),
        })
        .to_capnp(request.get().init_request())
        .map_err(daemon_protocol_preflight_failed)?;
        let response = request
            .send()
            .promise
            .await
            .map_err(daemon_protocol_preflight_failed)?;
        let result = ListProjectsResult::from_capnp(
            response
                .get()
                .map_err(daemon_protocol_preflight_failed)?
                .get_result()
                .map_err(daemon_protocol_preflight_failed)?,
        )
        .map_err(daemon_protocol_preflight_failed)?;
        result
            .protocol_version
            .require(ProtocolVersion::CURRENT)
            .map_err(daemon_protocol_preflight_failed)
    }

    /// Round-trip the protocol preflight against an already-connected daemon,
    /// proving it is both alive and speaking this build's protocol.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when the preflight call cannot
    /// be encoded or decoded, when it fails in transport, or when the daemon
    /// reports a protocol version other than [`ProtocolVersion::CURRENT`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn probe(&self) -> EditorResult<()> {
        self.require_current_protocol().await
    }

    /// Renew this editor process's lease on the daemon, so it keeps the
    /// session services this editor is using alive.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the daemon answers with a
    /// lease id other than the one derived from `owner_process`, or reports the
    /// lease as not accepted.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn touch_editor_process_lease(
        &self,
        owner_process: ProcessIdentity,
    ) -> EditorResult<TouchEditorLeaseResult> {
        let owner_process = ProtoProcessIdentity {
            process_id: owner_process.process_id,
            process_start_time: owner_process.process_start_time,
        };
        let lease_id = editor_process_lease_id(owner_process);
        let mut request = self.client.touch_editor_lease_request();
        (TouchEditorLeaseRequest {
            capability: self.daemon_lease_capability(),
            lease_id: lease_id.clone(),
            owner_process,
            purpose: "editor project workflow".to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = TouchEditorLeaseResult::from_capnp(response.get()?.get_result()?)?;
        if result.lease_id != lease_id || !result.accepted {
            return Err(daemon_authority_mismatch(
                "touchEditorLease",
                format!(
                    "daemon returned lease `{}` accepted={} for requested lease `{lease_id}`",
                    result.lease_id, result.accepted
                ),
            ));
        }
        Ok(result)
    }

    /// Look up the daemon's record for `project_id`, or `None` when it has none
    /// registered.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when a returned record is for a
    /// different project id, leaves any identity field blank, gives a
    /// non-absolute root or manifest path, or names a manifest path that is not
    /// `azoth.toml` directly under its own root.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_project(&self, project_id: &str) -> EditorResult<Option<ProjectRecord>> {
        let mut request = self.client.resolve_project_request();
        (ResolveProjectRequest {
            capability: self.daemon_read_capability(),
            project_id: project_id.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let project = ProjectResult::from_capnp(response.get()?.get_result()?)?.project;
        if let Some(project) = &project {
            ensure_daemon_project_record_matches_request(
                project,
                Some(project_id),
                None,
                "resolveProject",
            )?;
        }
        Ok(project)
    }

    /// Register `root` with the daemon and return the project record it now
    /// owns for that root.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the returned record leaves
    /// any identity field blank, gives a non-absolute root or manifest path,
    /// names a manifest path that is not `azoth.toml` directly under its own
    /// root, or reports a root other than the one requested.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn register_project_root(&self, root: &Path) -> EditorResult<ProjectRecord> {
        let mut request = self.client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: self.daemon_projects_capability(),
            root: root.to_string_lossy().into_owned(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let project = ProjectRecord::from_capnp(response.get()?.get_project()?)?;
        ensure_daemon_project_record_matches_request(
            &project,
            None,
            Some(root),
            "registerProjectRoot",
        )?;
        Ok(project)
    }

    /// Resolve the session-supervisor descriptor the daemon holds for one
    /// session of `project_id`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceDiscovery`] when the daemon answers with no
    /// descriptor, meaning no supervisor is registered for that session. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when `session_slug` is blank,
    /// or when the returned descriptor does not name the session-supervisor
    /// service id and role, claims a stale protocol version, or carries empty
    /// or tokenless brokered capability templates.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_session_supervisor_descriptor(
        &self,
        project_id: &str,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        let mut request = self.client.resolve_session_supervisor_request();
        (ResolveSessionSupervisorRequest {
            capability: self.daemon_read_capability(),
            project_id: project_id.to_string(),
            session_slug: session_slug.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let descriptor = SessionSupervisorResult::from_capnp(response.get()?.get_result()?)?
            .descriptor
            .ok_or_else(|| {
                EditorError::ServiceDiscovery(format!(
                    "session supervisor for project `{project_id}` session `{session_slug}` was not registered"
                ))
            })?;
        ensure_daemon_session_supervisor_descriptor_matches_request(
            &descriptor,
            session_slug,
            "resolveSessionSupervisor",
        )?;
        Ok(descriptor)
    }

    /// List every session-supervisor the daemon has registered for
    /// `project_id`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when an entry has a blank
    /// session slug, when the list repeats a slug, or when an entry's descriptor
    /// does not name the session-supervisor service id and role, claims a stale
    /// protocol version, or carries empty or tokenless brokered capability
    /// templates.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn list_session_supervisor_descriptors(
        &self,
        project_id: &str,
    ) -> EditorResult<Vec<SessionSupervisorDescriptor>> {
        let mut request = self.client.list_session_supervisors_request();
        (ListSessionSupervisorsRequest {
            capability: self.daemon_read_capability(),
            project_id: project_id.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let supervisors =
            ListSessionSupervisorsResult::from_capnp(response.get()?.get_result()?)?.supervisors;
        ensure_daemon_session_supervisor_list_is_authoritative(&supervisors)?;
        Ok(supervisors)
    }

    /// Create `session_name` in `project_id` if it does not exist yet, and
    /// return its manifest either way.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when `project_id` is blank,
    /// when the returned manifest belongs to another project, when its slug,
    /// project root, workspace root, or run directory is blank, or when any of
    /// those three paths is not absolute.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn ensure_project_session(
        &self,
        project_id: &str,
        session_name: &str,
    ) -> EditorResult<ProjectSessionResult> {
        let mut request = self.client.ensure_project_session_request();
        (EnsureProjectSessionRequest {
            capability: self.daemon_sessions_capability(),
            project_id: project_id.to_string(),
            session_name: session_name.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ProjectSessionResult::from_capnp(response.get()?.get_result()?)?;
        ensure_daemon_project_session_matches_request(&result, project_id)?;
        Ok(result)
    }

    /// Prepare (but do not start) the service processes of one session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when `session_slug` is blank,
    /// when the result is for another project or session, when it prepared no
    /// services at all, when a prepared service name is blank, or when the
    /// prepared process count disagrees with the number of service names.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn prepare_project_session_services(
        &self,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: az_proto_core::EndpointKind,
        skip_build: bool,
    ) -> EditorResult<ProjectSessionServicesResult> {
        let mut request = self.client.prepare_project_session_services_request();
        (PrepareProjectSessionServicesRequest {
            capability: self.daemon_sessions_capability(),
            project_id: project_id.to_string(),
            session_slug: session_slug.to_string(),
            endpoint_kind,
            skip_build,
            service_names: Vec::new(),
            otlp_endpoint: None,
            recover: false,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ProjectSessionServicesResult::from_capnp(response.get()?.get_result()?)?;
        ensure_daemon_project_session_services_match_request(&result, project_id, session_slug)?;
        Ok(result)
    }

    /// Supplies a callback `sink` capability the daemon drives with live
    /// open-project progress events during build/start/wait. The final result
    /// still returns on this RPC's promise.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the result is for another
    /// project, when nothing came back attach-ready (no prepared services, no
    /// running services, or a zero prepared process count), when a running
    /// service is not in the prepared list, when the session-supervisor
    /// descriptor it returns is invalid for that role or protocol, or when the
    /// `az-sessiond` log path is blank.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn ensure_project_session_services_with_progress(
        &self,
        project_id: &str,
        session_name: &str,
        endpoint_kind: az_proto_core::EndpointKind,
        skip_build: bool,
        start_service_names: Vec<String>,
        timeout_ms: u64,
        daemon_endpoint: &Endpoint,
        sink: daemon_capnp::project_open_progress_sink::Client,
    ) -> EditorResult<ProjectSessionServicesStartResult> {
        let mut request = self
            .client
            .ensure_project_session_services_with_progress_request();
        {
            let mut outer = request.get().init_request();
            (EnsureProjectSessionServicesRequest {
                capability: self.daemon_sessions_capability(),
                project_id: project_id.to_string(),
                session_name: session_name.to_string(),
                endpoint_kind,
                skip_build,
                start_service_names,
                timeout_ms,
                daemon_endpoint: daemon_endpoint.clone(),
            })
            .to_capnp(outer.reborrow().init_request())?;
            outer.set_progress_sink(sink);
        }

        let response = request.send().promise.await?;
        let result = ProjectSessionServicesStartResult::from_capnp(response.get()?.get_result()?)?;
        ensure_daemon_project_session_services_start_match_request(&result, project_id)?;
        Ok(result)
    }

    /// Ask the daemon for the build command plan of `project_id`, without
    /// running it.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when `project_id` is blank,
    /// when the plan has no commands, or when a command is untraceable — a blank
    /// owner id, owner root, target name, program, or working directory, a
    /// non-absolute owner root or working directory, or no arguments.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn plan_project_build(
        &self,
        project_id: &str,
        profile: &str,
        target_triple: Option<&str>,
    ) -> EditorResult<ProjectBuildPlan> {
        let mut request = self.client.plan_project_build_request();
        (PlanProjectBuildRequest {
            capability: self.daemon_projects_capability(),
            project_id: project_id.to_string(),
            profile: profile.to_string(),
            target_triple: target_triple.map(str::to_string),
            package_selectors: Vec::new(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let plan = ProjectBuildPlan::from_capnp(response.get()?.get_plan()?)?;
        ensure_daemon_project_build_plan_matches_request(&plan, project_id)?;
        Ok(plan)
    }

    /// Run the build of `project_id`, streaming progress to `sink` while the
    /// final result returns on this call.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceProtocol`] when the request cannot be
    /// encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] for everything
    /// [`Self::plan_project_build`] rejects about the embedded plan, and when
    /// the completed command count exceeds the plan length, a successful build
    /// still names a failing command, a failed build names none, or the failing
    /// command it does name is untraceable. A build that simply failed comes
    /// back as a result with `success` unset, not as an error.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn execute_project_build(
        &self,
        project_id: &str,
        profile: &str,
        target_triple: Option<&str>,
        sink: daemon_capnp::project_build_progress_sink::Client,
    ) -> EditorResult<ProjectBuildExecutionResult> {
        let mut request = self.client.execute_project_build_request();
        {
            let mut outer = request.get().init_request();
            (ExecuteProjectBuildRequest {
                capability: self.daemon_projects_capability(),
                project_id: project_id.to_string(),
                profile: profile.to_string(),
                target_triple: target_triple.map(str::to_string),
                package_selectors: Vec::new(),
            })
            .to_capnp(outer.reborrow().init_request())?;
            outer.set_progress_sink(sink);
        }

        let response = request.send().promise.await?;
        let result = ProjectBuildExecutionResult::from_capnp(response.get()?.get_result()?)?;
        ensure_daemon_project_build_execution_matches_request(&result, project_id)?;
        Ok(result)
    }

    fn daemon_read_capability(&self) -> Capability {
        Capability::new(self.editor_service.clone(), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_READ_PERMISSION])
    }

    fn daemon_projects_capability(&self) -> Capability {
        Capability::new(self.editor_service.clone(), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_PROJECTS_PERMISSION])
    }

    fn daemon_sessions_capability(&self) -> Capability {
        Capability::new(self.editor_service.clone(), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_SESSIONS_PERMISSION])
    }

    fn daemon_lease_capability(&self) -> Capability {
        Capability::new(self.editor_service.clone(), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_LEASE_PERMISSION])
    }
}

#[must_use]
pub fn daemon_service_id() -> ServiceId {
    ServiceId::new(DAEMON_NAMESPACE, DAEMON_SERVICE_NAME)
}

fn ensure_daemon_project_record_matches_request(
    project: &ProjectRecord,
    expected_project_id: Option<&str>,
    expected_root: Option<&Path>,
    operation: &'static str,
) -> EditorResult<()> {
    if let Some(expected_project_id) = expected_project_id
        && project.project_id != expected_project_id
    {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "daemon returned project `{}`, expected `{expected_project_id}`",
                project.project_id
            ),
        ));
    }
    if project.project_id.trim().is_empty()
        || project.name.trim().is_empty()
        || project.root.trim().is_empty()
        || project.manifest_path.trim().is_empty()
        || project.engine_version.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            operation,
            "daemon project record identity fields cannot be empty".to_string(),
        ));
    }

    let root = Path::new(&project.root);
    let manifest_path = Path::new(&project.manifest_path);
    if !root.is_absolute() || !manifest_path.is_absolute() {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "daemon project `{}` returned non-absolute root or manifest path",
                project.project_id
            ),
        ));
    }
    let expected_manifest_path = root.join("azoth.toml");
    if !same_daemon_path(manifest_path, &expected_manifest_path) {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "daemon project `{}` manifest path `{}` does not match root `{}`",
                project.project_id, project.manifest_path, project.root
            ),
        ));
    }
    if let Some(expected_root) = expected_root
        && !same_daemon_path(root, expected_root)
    {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "daemon project `{}` root `{}` does not match requested root `{}`",
                project.project_id,
                project.root,
                expected_root.display()
            ),
        ));
    }
    Ok(())
}

fn ensure_daemon_session_supervisor_descriptor_matches_request(
    descriptor: &ServiceDescriptor,
    expected_session_slug: &str,
    operation: &'static str,
) -> EditorResult<()> {
    if expected_session_slug.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            operation,
            "requested session slug cannot be empty".to_string(),
        ));
    }
    validate_session_supervisor_descriptor(descriptor).map_err(|error| {
        daemon_authority_mismatch(
            operation,
            format!("daemon returned invalid session-supervisor descriptor: {error}"),
        )
    })
}

fn ensure_daemon_session_supervisor_list_is_authoritative(
    supervisors: &[SessionSupervisorDescriptor],
) -> EditorResult<()> {
    let mut seen_slugs = BTreeSet::new();
    for supervisor in supervisors {
        if supervisor.session_slug.trim().is_empty() {
            return Err(daemon_authority_mismatch(
                "listSessionSupervisors",
                "daemon returned a session-supervisor with an empty slug".to_string(),
            ));
        }
        if !seen_slugs.insert(supervisor.session_slug.as_str()) {
            return Err(daemon_authority_mismatch(
                "listSessionSupervisors",
                format!(
                    "daemon returned duplicate session-supervisor slug `{}`",
                    supervisor.session_slug
                ),
            ));
        }
        validate_session_supervisor_descriptor(&supervisor.descriptor).map_err(|error| {
            daemon_authority_mismatch(
                "listSessionSupervisors",
                format!(
                    "daemon returned invalid descriptor for session `{}`: {error}",
                    supervisor.session_slug
                ),
            )
        })?;
    }
    Ok(())
}

fn ensure_daemon_project_session_matches_request(
    result: &ProjectSessionResult,
    expected_project_id: &str,
) -> EditorResult<()> {
    let manifest = &result.manifest;
    if expected_project_id.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "ensureProjectSession",
            "requested project id cannot be empty".to_string(),
        ));
    }
    if manifest.project_id != expected_project_id {
        return Err(daemon_authority_mismatch(
            "ensureProjectSession",
            format!(
                "daemon returned session for project `{}`, expected `{expected_project_id}`",
                manifest.project_id
            ),
        ));
    }
    if manifest.slug.trim().is_empty()
        || manifest.project_root.trim().is_empty()
        || manifest.workspace_root.trim().is_empty()
        || manifest.run_dir.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            "ensureProjectSession",
            "daemon returned incomplete session manifest identity fields".to_string(),
        ));
    }
    for (label, path) in [
        ("project root", &manifest.project_root),
        ("workspace root", &manifest.workspace_root),
        ("run dir", &manifest.run_dir),
    ] {
        if !Path::new(path).is_absolute() {
            return Err(daemon_authority_mismatch(
                "ensureProjectSession",
                format!("daemon returned non-absolute session {label} `{path}`"),
            ));
        }
    }
    Ok(())
}

fn ensure_daemon_project_session_services_match_request(
    result: &ProjectSessionServicesResult,
    expected_project_id: &str,
    expected_session_slug: &str,
) -> EditorResult<()> {
    if expected_session_slug.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "prepareProjectSessionServices",
            "requested session slug cannot be empty".to_string(),
        ));
    }
    if result.manifest.project_id != expected_project_id {
        return Err(daemon_authority_mismatch(
            "prepareProjectSessionServices",
            format!(
                "daemon returned session services for project `{}`, expected `{expected_project_id}`",
                result.manifest.project_id
            ),
        ));
    }
    if result.manifest.slug != expected_session_slug {
        return Err(daemon_authority_mismatch(
            "prepareProjectSessionServices",
            format!(
                "daemon returned session `{}`, expected `{expected_session_slug}`",
                result.manifest.slug
            ),
        ));
    }
    if result.service_names.is_empty() || result.prepared_process_count == 0 {
        return Err(daemon_authority_mismatch(
            "prepareProjectSessionServices",
            "daemon returned no prepared services".to_string(),
        ));
    }
    for service_name in &result.service_names {
        if service_name.trim().is_empty() {
            return Err(daemon_authority_mismatch(
                "prepareProjectSessionServices",
                "daemon returned an empty prepared service name".to_string(),
            ));
        }
    }
    if result.prepared_process_count as usize != result.service_names.len() {
        return Err(daemon_authority_mismatch(
            "prepareProjectSessionServices",
            format!(
                "daemon returned {} service names but {} prepared processes",
                result.service_names.len(),
                result.prepared_process_count
            ),
        ));
    }
    Ok(())
}

fn ensure_daemon_project_session_services_start_match_request(
    result: &ProjectSessionServicesStartResult,
    expected_project_id: &str,
) -> EditorResult<()> {
    if result.manifest.project_id != expected_project_id {
        return Err(daemon_authority_mismatch(
            "ensureProjectSessionServicesWithProgress",
            format!(
                "daemon returned session services for project `{}`, expected `{expected_project_id}`",
                result.manifest.project_id
            ),
        ));
    }
    if result.service_names.is_empty()
        || result.running_service_names.is_empty()
        || result.prepared_process_count == 0
    {
        return Err(daemon_authority_mismatch(
            "ensureProjectSessionServicesWithProgress",
            "daemon returned no attach-ready services".to_string(),
        ));
    }
    let service_names = result
        .service_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for service_name in &result.running_service_names {
        if !service_names.contains(service_name.as_str()) {
            return Err(daemon_authority_mismatch(
                "ensureProjectSessionServicesWithProgress",
                format!(
                    "daemon returned running service `{service_name}` outside prepared service list"
                ),
            ));
        }
    }
    validate_session_supervisor_descriptor(&result.supervisor).map_err(|error| {
        daemon_authority_mismatch(
            "ensureProjectSessionServicesWithProgress",
            format!("daemon returned invalid session-supervisor descriptor: {error}"),
        )
    })?;
    if result.log_path.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "ensureProjectSessionServicesWithProgress",
            "daemon returned an empty az-sessiond log path".to_string(),
        ));
    }
    Ok(())
}

fn ensure_daemon_project_build_plan_matches_request(
    plan: &ProjectBuildPlan,
    expected_project_id: &str,
) -> EditorResult<()> {
    if expected_project_id.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            "requested project id cannot be empty".to_string(),
        ));
    }
    if plan.commands.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!("daemon returned no build commands for project `{expected_project_id}`"),
        ));
    }
    for command in &plan.commands {
        ensure_daemon_project_build_command_is_traceable(command, expected_project_id)?;
    }
    Ok(())
}

fn ensure_daemon_project_build_execution_matches_request(
    result: &ProjectBuildExecutionResult,
    expected_project_id: &str,
) -> EditorResult<()> {
    ensure_daemon_project_build_plan_matches_request(&result.plan, expected_project_id)?;
    if result.completed_command_count as usize > result.plan.commands.len() {
        return Err(daemon_authority_mismatch(
            "executeProjectBuild",
            format!(
                "daemon returned completed command count beyond plan length for project `{expected_project_id}`"
            ),
        ));
    }
    if result.success {
        if result.failing_command.is_some() {
            return Err(daemon_authority_mismatch(
                "executeProjectBuild",
                "daemon returned a failing command for a successful build".to_string(),
            ));
        }
        return Ok(());
    }
    let Some(command) = result.failing_command.as_ref() else {
        return Err(daemon_authority_mismatch(
            "executeProjectBuild",
            format!(
                "daemon returned failed build without a failing command for project `{expected_project_id}`"
            ),
        ));
    };
    ensure_daemon_project_build_command_is_traceable(command, expected_project_id)
}

fn ensure_daemon_project_build_command_is_traceable(
    command: &ProjectBuildCommand,
    expected_project_id: &str,
) -> EditorResult<()> {
    if command.owner_id.trim().is_empty()
        || command.owner_root.trim().is_empty()
        || command.target_name.trim().is_empty()
        || command.program.trim().is_empty()
        || command.cwd.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!("daemon returned incomplete build command for project `{expected_project_id}`"),
        ));
    }
    let owner_root = Path::new(&command.owner_root);
    let cwd = Path::new(&command.cwd);
    if !owner_root.is_absolute() || !cwd.is_absolute() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!(
                "daemon returned non-absolute build command paths for owner `{}`",
                command.owner_id
            ),
        ));
    }
    if command.args.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!(
                "daemon returned build command `{}` for owner `{}` without args",
                command.target_name, command.owner_id
            ),
        ));
    }
    Ok(())
}

fn same_daemon_path(left: &Path, right: &Path) -> bool {
    let left = comparable_daemon_path(left);
    let right = comparable_daemon_path(right);
    #[cfg(windows)]
    {
        comparable_daemon_path_text(&left) == comparable_daemon_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn comparable_daemon_path(path: &Path) -> PathBuf {
    if path.exists() {
        path.canonicalize().map_or_else(
            |_| cli_compatible_daemon_path(path.to_path_buf()),
            cli_compatible_daemon_path,
        )
    } else {
        cli_compatible_daemon_path(path.to_path_buf())
    }
}

#[cfg(windows)]
fn comparable_daemon_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn cli_compatible_daemon_path(path: PathBuf) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(rest) = path_text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = path_text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
fn cli_compatible_daemon_path(path: PathBuf) -> PathBuf {
    path
}

pub(crate) fn validate_daemon_endpoint_record_protocol(
    record: &DaemonEndpointRecord,
) -> EditorResult<()> {
    record
        .require_protocol_version(ProtocolVersion::CURRENT)
        .map_err(daemon_protocol_preflight_failed)
}

/// ADR 0002/0031 keep all editor↔daemon IPC machine-local. Named pipes under
/// `\\.\pipe\`, Unix domain sockets, and in-process endpoints are local by
/// construction; only TCP addresses can point off-machine, so they must be
/// loopback. A project directory is untrusted input: its endpoint record
/// never gets to name a remote host.
pub(crate) fn ensure_daemon_endpoint_is_local(endpoint: &Endpoint) -> EditorResult<()> {
    let local = match endpoint.kind {
        EndpointKind::Tcp => endpoint.address.starts_with("127.0.0.1:"),
        EndpointKind::WindowsNamedPipe => endpoint.address.starts_with(r"\\.\pipe\"),
        // Unix sockets are filesystem objects and in-process endpoints never
        // leave the process; neither can reach a remote host. The wildcard
        // arm keeps this a locality check rather than an exhaustive kind
        // inventory.
        _ => true,
    };
    if local {
        Ok(())
    } else {
        Err(daemon_authority_mismatch(
            "endpoint discovery",
            format!(
                "daemon endpoint `{}` is not machine-local; expected loopback TCP or `\\\\.\\pipe\\azoth-…`",
                endpoint.address
            ),
        ))
    }
}

fn daemon_protocol_preflight_failed(error: impl std::fmt::Display) -> EditorError {
    EditorError::ServiceDiscovery(format!(
        "azd unavailable until restarted: protocol preflight failed: {error}"
    ))
}

const fn daemon_authority_mismatch(operation: &'static str, reason: String) -> EditorError {
    EditorError::ServiceAuthorityMismatch {
        service: "azd",
        operation,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use az_daemon::{AzDaemon, AzDaemonRpc};
    use az_project::{
        ProjectBuildTarget, ProjectManifest, refresh_project_lock, write_project_manifest,
    };
    use std::rc::Rc;

    #[test]
    fn daemon_endpoint_locality_rejects_remote_and_accepts_local_shapes() {
        use az_proto_core::EndpointKind;

        let remote = Endpoint::new(EndpointKind::Tcp, "203.0.113.7:37612");
        let error = ensure_daemon_endpoint_is_local(&remote).unwrap_err();
        assert!(
            matches!(error, EditorError::ServiceAuthorityMismatch { .. }),
            "expected authority mismatch, got {error:?}"
        );

        assert!(
            ensure_daemon_endpoint_is_local(&Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612"))
                .is_ok()
        );
        assert!(
            ensure_daemon_endpoint_is_local(&Endpoint::new(
                EndpointKind::WindowsNamedPipe,
                r"\\.\pipe\azoth-proj-token"
            ))
            .is_ok()
        );
        assert!(
            ensure_daemon_endpoint_is_local(&Endpoint::new(
                EndpointKind::UnixDomainSocket,
                "/run/azoth/svc-proj-token.sock"
            ))
            .is_ok()
        );
    }
    use az_proto_core::{Endpoint, EndpointKind};
    use az_proto_daemon::ProjectRecord;
    use uuid::Uuid;

    use super::*;

    struct OutdatedDaemon;

    impl daemon_capnp::az_daemon::Server for OutdatedDaemon {
        #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
        async fn list_projects(
            self: capnp::capability::Rc<Self>,
            _params: daemon_capnp::az_daemon::ListProjectsParams,
            mut results: daemon_capnp::az_daemon::ListProjectsResults,
        ) -> Result<(), capnp::Error> {
            (ListProjectsResult {
                projects: Vec::new(),
                protocol_version: ProtocolVersion {
                    major: 0,
                    minor: 1,
                    patch: 0,
                },
            })
            .to_capnp(results.get().init_result());
            Ok(())
        }
    }

    fn project_record() -> ProjectRecord {
        let root = std::env::temp_dir().join("azoth-editor-daemon");
        ProjectRecord {
            project_id: "local.editor_daemon".to_string(),
            name: "Editor Daemon".to_string(),
            root: root.to_string_lossy().into_owned(),
            manifest_path: root.join("azoth.toml").to_string_lossy().into_owned(),
            engine_version: "0.1.0".to_string(),
        }
    }

    fn write_test_project_manifest(root: &Path, manifest: &ProjectManifest) {
        write_project_manifest(root, manifest).unwrap();
        refresh_project_lock(root).unwrap();
    }

    fn valid_project_record_for_root(root: &Path) -> ProjectRecord {
        ProjectRecord {
            project_id: "local.editor_daemon".to_string(),
            name: "Editor Daemon".to_string(),
            root: root.to_string_lossy().into_owned(),
            manifest_path: root.join("azoth.toml").to_string_lossy().into_owned(),
            engine_version: "0.1.0".to_string(),
        }
    }

    fn valid_build_command(root: &Path) -> ProjectBuildCommand {
        ProjectBuildCommand {
            owner_id: "local.editor_daemon".to_string(),
            owner_root: root.to_string_lossy().into_owned(),
            target_name: "game".to_string(),
            program: "cargo".to_string(),
            cwd: root.to_string_lossy().into_owned(),
            args: vec!["build".to_string()],
            cargo_target_dir: None,
        }
    }

    fn assert_daemon_authority_mismatch(
        error: EditorError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            EditorError::ServiceAuthorityMismatch {
                service,
                operation,
                reason,
            } => {
                assert_eq!(service, "azd");
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected reason containing `{expected_reason}`, got `{reason}`"
                );
            }
            other => panic!("expected daemon authority mismatch, got {other:?}"),
        }
    }

    #[test]
    fn stable_daemon_preflight_rejects_outdated_protocol_before_application_rpc() {
        let client: daemon_capnp::az_daemon::Client = capnp_rpc::new_client(OutdatedDaemon);
        let daemon = AzDaemonClient::from_client(client);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        let error = local
            .block_on(&runtime, daemon.require_current_protocol())
            .unwrap_err();

        assert!(matches!(
            error,
            EditorError::ServiceDiscovery(message)
                if message.contains("azd unavailable until restarted")
                    && message.contains("0.1.0")
                    && message.contains("0.3.0")
        ));
    }

    #[test]
    fn daemon_project_record_must_echo_requested_project_id() {
        let root = tempfile::tempdir().unwrap();
        let mut project = valid_project_record_for_root(root.path());
        project.project_id = "local.other".to_string();

        let error = ensure_daemon_project_record_matches_request(
            &project,
            Some("local.editor_daemon"),
            None,
            "resolveProject",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(error, "resolveProject", "expected `local.editor_daemon`");
    }

    #[test]
    fn daemon_registered_project_root_must_echo_requested_root() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let project = valid_project_record_for_root(root.path());

        let error = ensure_daemon_project_record_matches_request(
            &project,
            None,
            Some(other.path()),
            "registerProjectRoot",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(error, "registerProjectRoot", "requested root");
    }

    #[test]
    fn daemon_session_supervisor_list_rejects_duplicate_slugs() {
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37674"),
        );
        let supervisors = vec![
            SessionSupervisorDescriptor {
                session_slug: "lighting".to_string(),
                descriptor: descriptor.clone(),
            },
            SessionSupervisorDescriptor {
                session_slug: "lighting".to_string(),
                descriptor,
            },
        ];

        let error =
            ensure_daemon_session_supervisor_list_is_authoritative(&supervisors).unwrap_err();

        assert_daemon_authority_mismatch(error, "listSessionSupervisors", "duplicate");
    }

    #[test]
    fn daemon_session_supervisor_descriptor_must_be_valid() {
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37674"),
        );
        descriptor.role = ServiceRole::Editor;

        let error = ensure_daemon_session_supervisor_descriptor_matches_request(
            &descriptor,
            "lighting",
            "resolveSessionSupervisor",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(
            error,
            "resolveSessionSupervisor",
            "session-supervisor descriptor",
        );
    }

    #[test]
    fn daemon_project_build_plan_must_be_traceable() {
        let root = tempfile::tempdir().unwrap();
        let mut command = valid_build_command(root.path());
        command.args.clear();
        let plan = ProjectBuildPlan {
            commands: vec![command],
            package_profile: None,
        };

        let error = ensure_daemon_project_build_plan_matches_request(&plan, "local.editor_daemon")
            .unwrap_err();

        assert_daemon_authority_mismatch(error, "planProjectBuild", "without args");
    }

    #[test]
    fn editor_resolves_project_and_session_supervisor_through_daemon() {
        let daemon = AzDaemon::new();
        let project = project_record();
        daemon.register_project(&project).unwrap();
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37674"),
        );
        daemon
            .register_session_supervisor(&project.project_id, "lighting", &descriptor)
            .unwrap();
        let rpc = Rc::new(AzDaemonRpc::test_new(daemon));
        let client = AzDaemonClient::new(AzDaemonRpc::client_from_rc(&rpc));

        let resolved_project =
            futures::executor::block_on(client.resolve_project(&project.project_id)).unwrap();
        assert_eq!(resolved_project, Some(project.clone()));

        let resolved_session = futures::executor::block_on(
            client.resolve_session_supervisor_descriptor(&project.project_id, "lighting"),
        )
        .unwrap();
        assert_eq!(resolved_session, descriptor);
        let sessions = futures::executor::block_on(
            client.list_session_supervisor_descriptors(&project.project_id),
        )
        .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_slug, "lighting");
        assert_eq!(sessions[0].descriptor, descriptor);
        assert_eq!(
            daemon_service_id(),
            ServiceId::new(DAEMON_NAMESPACE, DAEMON_SERVICE_NAME)
        );
    }

    #[test]
    fn missing_session_supervisor_is_an_editor_discovery_error() {
        let daemon = AzDaemon::new();
        let project = project_record();
        daemon.register_project(&project).unwrap();
        let rpc = Rc::new(AzDaemonRpc::test_new(daemon));
        let client = AzDaemonClient::new(AzDaemonRpc::client_from_rc(&rpc));

        let error = futures::executor::block_on(
            client.resolve_session_supervisor_descriptor(&project.project_id, "missing"),
        )
        .unwrap_err();

        assert!(matches!(error, EditorError::ServiceDiscovery(_)));
    }

    #[test]
    fn editor_requests_project_build_plan_through_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.editor_build", "Editor Build", "0.1.0");
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "editor_build"));
        write_test_project_manifest(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();
        let rpc = Rc::new(AzDaemonRpc::test_new(daemon));
        let client = AzDaemonClient::new(AzDaemonRpc::client_from_rc(&rpc));

        let plan = futures::executor::block_on(client.plan_project_build(
            "local.editor_build",
            "release",
            Some("x86_64-pc-windows-msvc"),
        ))
        .unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(
            plan.commands[0].args,
            vec![
                "build",
                "-p",
                "editor_build",
                "--release",
                "--target",
                "x86_64-pc-windows-msvc",
            ]
        );
    }

    #[test]
    fn editor_registers_project_root_through_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let root = az_filesystem::normalize(temp.path());
        let manifest = ProjectManifest::new("local.editor_root", "Editor Root", "0.1.0");
        write_test_project_manifest(&root, &manifest);
        let rpc = Rc::new(AzDaemonRpc::test_new(AzDaemon::new()));
        let client = AzDaemonClient::new(AzDaemonRpc::client_from_rc(&rpc));

        let project = futures::executor::block_on(client.register_project_root(&root)).unwrap();

        assert_eq!(project.project_id, "local.editor_root");
        assert_eq!(project.name, "Editor Root");
        assert_eq!(project.root, root.to_string_lossy());
        assert_eq!(
            futures::executor::block_on(client.resolve_project("local.editor_root")).unwrap(),
            Some(project)
        );
    }
}
