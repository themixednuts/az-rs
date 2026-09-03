//! Canonical descriptors for Azoth's built-in services.
//!
//! A descriptor is a protocol contract, not supervision ownership. Project
//! supervisors and session supervisors both consume this catalog, then persist
//! the resulting descriptor only in the scope that owns the process.

use az_proto_asset::{
    ASSET_JOBS_PERMISSION, ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE,
    ASSET_PROCESSOR_SERVICE_NAME, ASSET_READ_PERMISSION, ASSET_WORKER_SERVICE_NAME,
    ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WRITE_PERMISSION,
};
use az_proto_core::{
    Capability, CapabilityGrantRequirement, EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE,
    Endpoint, SERVICE_LIFECYCLE_AUDIENCE, SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION, ServiceDescriptor,
    ServiceId, ServiceRole,
};
use az_proto_observability::{OBSERVABILITY_AUDIENCE, OBSERVABILITY_CONTROL_PERMISSION};
use az_proto_project::{
    PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_EDIT_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION, PROJECT_GRAPH_CATALOG_PERMISSION, PROJECT_HOST_AUDIENCE,
    PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME, PROJECT_INVENTORY_PERMISSION,
    PROJECT_NODE_CATALOG_PERMISSION, PROJECT_RUNTIME_LAUNCH_PERMISSION, PROJECT_SCHEMA_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION,
};
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RUNTIME_HOST_NAMESPACE,
    RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION,
};
use az_proto_session::{
    SESSION_EXEC_PERMISSION, SESSION_MANAGE_PERMISSION, SESSION_READ_PERMISSION,
    SESSION_SAVE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME,
};
use az_service_supervision::{observability_endpoint, service_lifecycle_endpoint};
use uuid::Uuid;

pub const DAEMON_SERVICE_NAMESPACE: &str = "azoth";
pub const DAEMON_SERVICE_NAME: &str = "azd";

const EDITOR_PROJECT_HOST_PERMISSIONS: &[&str] = &[
    PROJECT_SCHEMA_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION,
    PROJECT_NODE_CATALOG_PERMISSION,
    PROJECT_GRAPH_CATALOG_PERMISSION,
    PROJECT_INVENTORY_PERMISSION,
    PROJECT_EDIT_PERMISSION,
    PROJECT_DOCUMENT_READ_PERMISSION,
    PROJECT_DOCUMENT_WRITE_PERMISSION,
    PROJECT_RUNTIME_LAUNCH_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION,
];

const EDITOR_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const WORKER_ASSET_PROCESSOR_PERMISSIONS: &[&str] = &[ASSET_JOBS_PERMISSION];
const PROJECT_HOST_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const SESSION_SUPERVISOR_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const SERVICE_LIFECYCLE_PERMISSIONS: &[&str] = &[SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION];

/// Exact project-scope authority admitted by the lifecycle control interface.
pub const PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS: &[CapabilityGrantRequirement<
    'static,
>] = &[CapabilityGrantRequirement::new(
    DAEMON_SERVICE_NAMESPACE,
    DAEMON_SERVICE_NAME,
    ServiceRole::Daemon,
    SERVICE_LIFECYCLE_AUDIENCE,
    SERVICE_LIFECYCLE_PERMISSIONS,
)];

/// Exact session-scope authority admitted by the lifecycle control interface.
pub const SESSION_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS: &[CapabilityGrantRequirement<
    'static,
>] = &[CapabilityGrantRequirement::new(
    SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME,
    ServiceRole::SessionSupervisor,
    SERVICE_LIFECYCLE_AUDIENCE,
    SERVICE_LIFECYCLE_PERMISSIONS,
)];

/// Canonical callers admitted by the project-scoped Asset Processor service.
///
/// The service catalog owns this cross-service topology contract. The Asset
/// Processor transport validates brokered grants against it without depending
/// on any caller's RPC protocol crate.
pub const ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        ASSET_PROCESSOR_AUDIENCE,
        EDITOR_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        ASSET_WORKER_SERVICE_NAMESPACE,
        ASSET_WORKER_SERVICE_NAME,
        ServiceRole::Worker,
        ASSET_PROCESSOR_AUDIENCE,
        WORKER_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        PROJECT_HOST_NAMESPACE,
        PROJECT_HOST_SERVICE_NAME,
        ServiceRole::ProjectHost,
        ASSET_PROCESSOR_AUDIENCE,
        PROJECT_HOST_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
        ServiceRole::SessionSupervisor,
        ASSET_PROCESSOR_AUDIENCE,
        SESSION_SUPERVISOR_ASSET_PROCESSOR_PERMISSIONS,
    ),
];

fn capability_token_hash() -> Vec<u8> {
    Uuid::new_v4().as_bytes().to_vec()
}

/// Classify a brokered grant by its observability-control audience.
#[must_use]
pub fn is_observability_control_grant(capability: &Capability) -> bool {
    capability.audience == OBSERVABILITY_AUDIENCE
}

/// Classify a brokered grant by its service-lifecycle audience.
#[must_use]
pub fn is_service_lifecycle_grant(capability: &Capability) -> bool {
    capability.audience == SERVICE_LIFECYCLE_AUDIENCE
}

/// Add the canonical observability endpoint and brokered callers.
///
/// Project services pass `None`; session services pass their session id. This
/// makes scope explicit at the call site and prevents project descriptors from
/// accidentally inheriting a session capability.
pub fn add_observability_contract(descriptor: &mut ServiceDescriptor, session: Option<Uuid>) {
    if descriptor.observability_endpoint.is_none() {
        descriptor.observability_endpoint =
            Some(observability_endpoint(descriptor.endpoint.clone()));
    }
    for (namespace, name, role) in [
        (
            EDITOR_SERVICE_NAMESPACE,
            EDITOR_SERVICE_NAME,
            ServiceRole::Editor,
        ),
        (
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
            ServiceRole::SessionSupervisor,
        ),
    ] {
        if descriptor.capabilities.iter().any(|capability| {
            capability.role == role
                && capability.audience == OBSERVABILITY_AUDIENCE
                && capability.has_permissions(&[OBSERVABILITY_CONTROL_PERMISSION])
        }) {
            continue;
        }
        let mut capability = Capability::new(ServiceId::new(namespace, name), role)
            .with_audience(OBSERVABILITY_AUDIENCE)
            .with_permissions([OBSERVABILITY_CONTROL_PERMISSION])
            .with_token_hash(capability_token_hash());
        capability.session = session;
        descriptor.capabilities.push(capability);
    }
}

/// Add the one entrypoint-owned shutdown contract every generated service
/// exposes. The supervisor capability is separate from role APIs and carries
/// no endpoint-selection authority.
pub fn add_service_lifecycle_contract(
    descriptor: &mut ServiceDescriptor,
    session: Option<Uuid>,
    controller: ServiceId,
    controller_role: ServiceRole,
) {
    if descriptor.lifecycle_endpoint.is_none() {
        descriptor.lifecycle_endpoint =
            Some(service_lifecycle_endpoint(descriptor.endpoint.clone()));
    }
    if descriptor.capabilities.iter().any(|capability| {
        capability.service == controller
            && capability.role == controller_role
            && capability.audience == SERVICE_LIFECYCLE_AUDIENCE
            && capability.has_permissions(&[SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
    }) {
        return;
    }
    let mut capability = Capability::new(controller, controller_role)
        .with_audience(SERVICE_LIFECYCLE_AUDIENCE)
        .with_permissions([SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
        .with_token_hash(capability_token_hash());
    capability.session = session;
    descriptor.capabilities.push(capability);
}

#[must_use]
pub fn asset_worker_service_descriptor(run: Uuid, endpoint: Endpoint) -> ServiceDescriptor {
    let service_id = ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME);
    project_service_descriptor_with_lifecycle(
        ServiceDescriptor::new(service_id, ServiceRole::Worker, endpoint)
            .with_run(run)
            .with_capability(
                Capability::new(
                    ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
                    ServiceRole::Worker,
                )
                .with_audience(ASSET_PROCESSOR_AUDIENCE)
                .with_permissions([ASSET_JOBS_PERMISSION])
                .with_token_hash(capability_token_hash()),
            ),
    )
}

#[must_use]
pub fn asset_processor_service_descriptor(run: Uuid, endpoint: Endpoint) -> ServiceDescriptor {
    let service_id = ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME);
    let mut descriptor =
        ServiceDescriptor::new(service_id, ServiceRole::AssetProcessor, endpoint).with_run(run);
    for requirement in ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS {
        descriptor = descriptor.with_capability(
            Capability::new(
                ServiceId::new(requirement.service_namespace, requirement.service_name),
                requirement.role,
            )
            .with_audience(requirement.audience)
            .with_permissions(requirement.permissions.iter().copied())
            .with_token_hash(capability_token_hash()),
        );
    }
    project_service_descriptor_with_lifecycle(descriptor)
}

#[must_use]
pub fn project_host_service_descriptor(run: Uuid, endpoint: Endpoint) -> ServiceDescriptor {
    project_service_descriptor_with_lifecycle(
        ServiceDescriptor::new(
            ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            endpoint,
        )
        .with_run(run)
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions(EDITOR_PROJECT_HOST_PERMISSIONS.iter().copied())
            .with_token_hash(capability_token_hash()),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(
                    SESSION_SUPERVISOR_NAMESPACE,
                    SESSION_SUPERVISOR_SERVICE_NAME,
                ),
                ServiceRole::SessionSupervisor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions([PROJECT_DOCUMENT_WRITE_PERMISSION])
            .with_token_hash(capability_token_hash()),
        ),
    )
}

#[must_use]
pub fn runtime_host_service_descriptor(
    session: Uuid,
    run: Uuid,
    endpoint: Endpoint,
) -> ServiceDescriptor {
    let mut descriptor = ServiceDescriptor::new(
        ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
        ServiceRole::RuntimeHost,
        endpoint,
    )
    .with_run(run)
    .with_capability(
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(session)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_READ_PERMISSION, RUNTIME_CONTROL_PERMISSION])
        .with_token_hash(capability_token_hash()),
    )
    .with_capability(
        Capability::new(
            ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        )
        .with_session(session)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash(capability_token_hash()),
    );
    add_service_lifecycle_contract(
        &mut descriptor,
        Some(session),
        ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
    );
    descriptor
}

fn project_service_descriptor_with_lifecycle(
    mut descriptor: ServiceDescriptor,
) -> ServiceDescriptor {
    add_service_lifecycle_contract(
        &mut descriptor,
        None,
        ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
        ServiceRole::Daemon,
    );
    descriptor
}

#[must_use]
pub fn session_supervisor_service_descriptor(
    session: Uuid,
    run: Uuid,
    endpoint: Endpoint,
) -> ServiceDescriptor {
    ServiceDescriptor::new(
        ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
        endpoint,
    )
    .with_run(run)
    .with_capability(
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(session)
        .with_audience(SESSION_SUPERVISOR_AUDIENCE)
        .with_permissions([
            SESSION_READ_PERMISSION,
            SESSION_SAVE_PERMISSION,
            SESSION_EXEC_PERMISSION,
            SESSION_MANAGE_PERMISSION,
        ])
        .with_token_hash(capability_token_hash()),
    )
    .with_capability(
        Capability::new(
            ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
            ServiceRole::Daemon,
        )
        .with_session(session)
        .with_audience(SESSION_SUPERVISOR_AUDIENCE)
        .with_permissions([SESSION_READ_PERMISSION, SESSION_MANAGE_PERMISSION])
        .with_token_hash(capability_token_hash()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_proto_core::EndpointKind;

    #[test]
    fn project_descriptors_never_capture_a_session() {
        let run = Uuid::now_v7();
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
        for mut descriptor in [
            project_host_service_descriptor(run, endpoint.clone()),
            asset_processor_service_descriptor(run, endpoint.clone()),
            asset_worker_service_descriptor(run, endpoint),
        ] {
            add_observability_contract(&mut descriptor, None);
            assert!(
                descriptor
                    .capabilities
                    .iter()
                    .all(|capability| capability.session.is_none())
            );
        }
    }

    #[test]
    fn asset_processor_brokers_project_host_source_authoring() {
        let descriptor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let grant = descriptor
            .capabilities
            .iter()
            .find(|capability| capability.role == ServiceRole::ProjectHost)
            .expect("asset processor must broker project-host source authoring");

        assert_eq!(grant.audience, ASSET_PROCESSOR_AUDIENCE);
        assert!(grant.has_permissions(&[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION]));
    }

    #[test]
    fn runtime_descriptors_are_session_scoped() {
        let session = Uuid::new_v4();
        let descriptor = runtime_host_service_descriptor(
            session,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        assert!(
            descriptor
                .capabilities
                .iter()
                .all(|capability| capability.session == Some(session))
        );
    }
}
