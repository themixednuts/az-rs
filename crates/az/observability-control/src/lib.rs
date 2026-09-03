//! Cap'n Proto `ObservabilityControl` server: lets a trusted peer (the editor or
//! the session supervisor) dial this process's per-channel log verbosity live.
//!
//! The server is deliberately thin — it validates the caller's capability against
//! a brokered grant set, then applies the change to the process-global tracing
//! reload handle owned by [`az_observability`]. Keeping the RPC/transport surface
//! here (rather than in `az-observability`) means the wide leaf logging crate
//! never pulls in `az-rpc`/`capnp-rpc`; only the handful of service processes
//! that actually *serve* control do.

mod transport;

use std::rc::Rc;
use std::time::{Duration, Instant};

use az_proto_core::{Capability, CapabilityGrantSet, ServiceHealth, ServiceId, ServiceRole};
use az_proto_observability::{
    ChannelLevel, ListChannelLevelsResult, LogChannel, OBSERVABILITY_AUDIENCE,
    OBSERVABILITY_CONTROL_PERMISSION, SetLogChannelLevelRequest, SetLogChannelLevelResult,
    observability_capnp,
};
use capnp::Error;

pub use transport::{
    ObservabilityControlRpcServer, ObservabilityControlRpcTransportError,
    connect_observability_control_rpc_client,
    start_observability_control_rpc_server_with_capability_grant_file,
    start_observability_control_rpc_server_with_grants,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityControlScope {
    Project,
    Session(uuid::Uuid),
}

/// Serves the `ObservabilityControl` interface for a single service process.
///
/// Stateless beyond the owner scope and brokered grant set it
/// validates callers against — the actual verbosity state lives in the process
/// global owned by [`az_observability`].
pub struct ObservabilityControlRpc {
    scope: ObservabilityControlScope,
    capability_grants: CapabilityGrantSet,
    service: ServiceId,
    role: ServiceRole,
    run: uuid::Uuid,
    started_at: Instant,
}

impl std::fmt::Debug for ObservabilityControlRpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityControlRpc")
            .field("scope", &self.scope)
            .field("service", &self.service)
            .field("role", &self.role)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

impl ObservabilityControlRpc {
    #[must_use]
    pub fn new(
        scope: ObservabilityControlScope,
        capability_grants: CapabilityGrantSet,
        service: ServiceId,
        role: ServiceRole,
        run: uuid::Uuid,
    ) -> Self {
        Self {
            scope,
            capability_grants,
            service,
            role,
            run,
            started_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn into_client(self) -> observability_capnp::observability_control::Client {
        capnp_rpc::new_client(self)
    }

    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> observability_capnp::observability_control::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }

    /// Validate that `capability` is a live, brokered grant for this process's
    /// is allowed to dial this process's verbosity. Mirrors the runtime-host
    /// capability gate (lifetime → audience → role → permission → session scope →
    /// brokered grant). The `listChannelLevels` path passes a bare capability, so
    /// this performs the full check rather than relying on DTO-level validation.
    fn authorize(&self, capability: &Capability) -> Result<(), Error> {
        capability
            .validate_lifetime()
            .map_err(|error| Error::failed(format!("capability lifetime invalid: {error}")))?;
        if capability.audience != OBSERVABILITY_AUDIENCE {
            return Err(Error::failed(format!(
                "expected audience `{OBSERVABILITY_AUDIENCE}` but got `{}`",
                capability.audience
            )));
        }
        if !matches!(
            capability.role,
            ServiceRole::Editor | ServiceRole::SessionSupervisor
        ) {
            return Err(Error::failed(format!(
                "role {:?} cannot dial observability",
                capability.role
            )));
        }
        if !capability
            .permissions
            .iter()
            .any(|permission| permission == OBSERVABILITY_CONTROL_PERMISSION)
        {
            return Err(Error::failed(format!(
                "missing required permission `{OBSERVABILITY_CONTROL_PERMISSION}`"
            )));
        }
        match (self.scope, capability.session) {
            (ObservabilityControlScope::Project, None) => {}
            (ObservabilityControlScope::Session(expected), Some(actual)) if expected == actual => {}
            (ObservabilityControlScope::Project, Some(actual)) => {
                return Err(Error::failed(format!(
                    "session-scoped capability `{actual}` cannot control a project service"
                )));
            }
            (ObservabilityControlScope::Session(expected), Some(actual)) => {
                return Err(Error::failed(format!(
                    "capability session `{actual}` cannot control session `{expected}`"
                )));
            }
            (ObservabilityControlScope::Session(expected), None) => {
                return Err(Error::failed(format!(
                    "project-scoped capability cannot control session `{expected}`"
                )));
            }
        }
        self.capability_grants
            .validate(capability, OBSERVABILITY_CONTROL_PERMISSION)
            .map_err(|error| Error::failed(error.to_string()))?;
        Ok(())
    }

    /// Apply a verbosity change to the process tracing subscriber. Capability is
    /// assumed already authorized. Channel-resolution and reload failures become
    /// an application-level rejection (carried in the result) rather than an RPC
    /// transport error.
    fn apply(request: &SetLogChannelLevelRequest) -> SetLogChannelLevelResult {
        let Some(channel) = LogChannel::from_target(&request.target) else {
            return SetLogChannelLevelResult {
                accepted: false,
                effective_directives: String::new(),
                diagnostic: format!("unknown channel target `{}`", request.target),
            };
        };
        let outcome = if request.clear {
            az_observability::clear_channel_level(channel)
        } else {
            az_observability::set_channel_level(channel, request.verbosity)
        };
        match outcome {
            Ok(effective_directives) => {
                tracing::info!(
                    target: "az.observability",
                    channel = channel.display_name(),
                    clear = request.clear,
                    verbosity = request.verbosity.as_str(),
                    effective = %effective_directives,
                    "applied observability channel level"
                );
                SetLogChannelLevelResult {
                    accepted: true,
                    effective_directives,
                    diagnostic: String::new(),
                }
            }
            Err(error) => SetLogChannelLevelResult {
                accepted: false,
                effective_directives: String::new(),
                diagnostic: error.to_string(),
            },
        }
    }

    fn channel_levels() -> ListChannelLevelsResult {
        let effective_directives = az_observability::current_log_directives().unwrap_or_default();
        let overrides = az_observability::current_channel_levels()
            .into_iter()
            .map(|(channel, verbosity)| ChannelLevel {
                target: channel.as_target().to_string(),
                verbosity,
            })
            .collect();
        ListChannelLevelsResult {
            effective_directives,
            overrides,
        }
    }

    fn health_snapshot(&self) -> ServiceHealth {
        ServiceHealth::ready(
            self.service.clone(),
            self.role,
            self.run,
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_message("observability-control reachable")
    }
}

impl observability_capnp::observability_control::Server for ObservabilityControlRpc {
    // capnp's `Rc<Self>` receiver and client hooks are `!Send` by construction;
    // this server runs on a `LocalSet`.
    #[allow(clippy::future_not_send)]
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: observability_capnp::observability_control::HealthParams,
        mut results: observability_capnp::observability_control::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    // capnp's `Rc<Self>` receiver and client hooks are `!Send` by construction;
    // this server runs on a `LocalSet`.
    #[allow(clippy::future_not_send)]
    async fn set_log_channel_level(
        self: capnp::capability::Rc<Self>,
        params: observability_capnp::observability_control::SetLogChannelLevelParams,
        mut results: observability_capnp::observability_control::SetLogChannelLevelResults,
    ) -> Result<(), Error> {
        let request = SetLogChannelLevelRequest::from_capnp(params.get()?.get_request()?)?;
        self.authorize(&request.capability)?;
        let result = Self::apply(&request);
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp's `Rc<Self>` receiver and client hooks are `!Send` by construction;
    // this server runs on a `LocalSet`.
    #[allow(clippy::future_not_send)]
    async fn list_channel_levels(
        self: capnp::capability::Rc<Self>,
        params: observability_capnp::observability_control::ListChannelLevelsParams,
        mut results: observability_capnp::observability_control::ListChannelLevelsResults,
    ) -> Result<(), Error> {
        let capability = az_proto_core::Capability::from_capnp(params.get()?.get_capability()?)?;
        self.authorize(&capability)?;
        let result = Self::channel_levels();
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
