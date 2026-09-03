//! Authenticated request client for the one generic service-lifetime endpoint.
//!
//! Scope owners choose stop order and fallback policy. This module only proves
//! that a ready descriptor grants the specified controller authority and sends
//! the shutdown request through the advertised endpoint.

use az_proto_core::{
    Capability, SERVICE_LIFECYCLE_AUDIENCE, SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION,
    ServiceDescriptor, ServiceId, ServiceRole, core_capnp,
};
use az_rpc::AzRpcTransportError;
use thiserror::Error;
use tokio::runtime::Builder;
use tokio::task::LocalSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLifecycleController {
    pub service: ServiceId,
    pub role: ServiceRole,
}

impl ServiceLifecycleController {
    #[must_use]
    pub const fn new(service: ServiceId, role: ServiceRole) -> Self {
        Self { service, role }
    }
}

#[derive(Debug, Error)]
pub enum ServiceLifecycleShutdownError {
    #[error("service `{service}` has no lifecycle endpoint")]
    MissingEndpoint { service: String },
    #[error("service `{service}` grants no lifecycle shutdown capability to {controller:?}")]
    MissingCapability {
        service: String,
        controller: ServiceLifecycleController,
    },
    #[error("lifecycle control runtime failed: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("lifecycle control transport failed: {0}")]
    Transport(#[from] AzRpcTransportError),
    #[error("lifecycle control protocol failed: {0}")]
    Protocol(#[from] capnp::Error),
}

/// Request graceful shutdown through a descriptor's authenticated lifecycle
/// endpoint.
///
/// It returns only after the control RPC has acknowledged admission; callers
/// still own waiting for the exact process identity and force fallback.
///
/// # Errors
///
/// Returns [`ServiceLifecycleShutdownError::MissingEndpoint`] if `descriptor`
/// advertises no lifecycle endpoint,
/// [`ServiceLifecycleShutdownError::MissingCapability`] if it grants
/// `controller` no shutdown authority,
/// [`ServiceLifecycleShutdownError::Runtime`] if the current-thread Tokio
/// runtime cannot be built, [`ServiceLifecycleShutdownError::Transport`] if the
/// endpoint cannot be connected or cleanly disconnected, or
/// [`ServiceLifecycleShutdownError::Protocol`] if the shutdown request itself
/// fails.
pub fn request_service_lifecycle_shutdown(
    descriptor: &ServiceDescriptor,
    controller: &ServiceLifecycleController,
) -> Result<(), ServiceLifecycleShutdownError> {
    let endpoint = descriptor.lifecycle_endpoint.as_ref().ok_or_else(|| {
        ServiceLifecycleShutdownError::MissingEndpoint {
            service: descriptor.id.name.clone(),
        }
    })?;
    let capability = lifecycle_shutdown_capability(descriptor, controller)?;
    let runtime = Builder::new_current_thread().enable_io().build()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async move {
        let connection: az_rpc::ScopedTwopartyClient<
            core_capnp::service_lifecycle_control::Client,
        > = az_rpc::connect_twoparty_bootstrap_scoped(endpoint).await?;
        let mut request = connection.client().shutdown_request();
        capability.to_capnp(request.get().init_capability())?;
        request.send().promise.await?;
        connection.disconnect().await?;
        Ok(())
    })
}

fn lifecycle_shutdown_capability(
    descriptor: &ServiceDescriptor,
    controller: &ServiceLifecycleController,
) -> Result<Capability, ServiceLifecycleShutdownError> {
    descriptor
        .capabilities
        .iter()
        .find(|capability| {
            capability.service == controller.service
                && capability.role == controller.role
                && capability.audience == SERVICE_LIFECYCLE_AUDIENCE
                && capability.has_permissions(&[SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
        })
        .cloned()
        .ok_or_else(|| ServiceLifecycleShutdownError::MissingCapability {
            service: descriptor.id.name.clone(),
            controller: controller.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_proto_core::Endpoint;

    #[test]
    fn lifecycle_capability_selects_the_exact_controller_not_a_role_name() {
        let daemon =
            ServiceLifecycleController::new(ServiceId::new("azoth", "azd"), ServiceRole::Daemon);
        let wrong_controller = Capability::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
        )
        .with_audience(SERVICE_LIFECYCLE_AUDIENCE)
        .with_permissions([SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION]);
        let exact_controller = Capability::new(daemon.service.clone(), daemon.role)
            .with_audience(SERVICE_LIFECYCLE_AUDIENCE)
            .with_permissions([SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION]);
        let mut descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "asset-worker"),
            ServiceRole::Worker,
            Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:0"),
        );
        descriptor.capabilities = vec![wrong_controller, exact_controller.clone()];

        assert_eq!(
            lifecycle_shutdown_capability(&descriptor, &daemon).unwrap(),
            exact_controller
        );
    }
}
