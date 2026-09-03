//! The ingress control port the lifecycle reconciler consumes.
//!
//! Ingress owns only an opaque virtual route and its generation-fenced
//! attachment. The current `ServerProcess` retains DTLS and Carrier state, so
//! no transport, socket, certificate, or gateway type appears here.
//!
//! A route may be staged before readiness but becomes current only after a
//! fenced durable transition, which is why [`StagedRoute`] and [`CurrentRoute`]
//! are different types. Neither is admission.

use crate::{FenceClaim, FencedEffect, IngressRouteId, PlacementFence, PortFuture, StalePlacement};

/// The opaque envelope a client uses to reach a placement.
///
/// The digest stands for the envelope's canonical bytes. Its contents are
/// provider-owned and never interpreted by the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpaqueRoute {
    route: IngressRouteId,
    envelope_digest: [u8; 32],
}

impl OpaqueRoute {
    /// Binds a route identity to its canonical envelope digest.
    #[must_use]
    pub const fn new(route: IngressRouteId, envelope_digest: [u8; 32]) -> Self {
        Self {
            route,
            envelope_digest,
        }
    }

    /// Returns the route identity.
    #[must_use]
    pub const fn route(self) -> IngressRouteId {
        self.route
    }

    /// Returns the canonical envelope digest.
    #[must_use]
    pub const fn envelope_digest(&self) -> &[u8; 32] {
        &self.envelope_digest
    }
}

/// A route that exists but is not reachable by clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StagedRoute(OpaqueRoute);

impl StagedRoute {
    /// Records a staged, unpublished route.
    #[must_use]
    pub const fn new(route: OpaqueRoute) -> Self {
        Self(route)
    }

    /// Returns the opaque route.
    #[must_use]
    pub const fn route(self) -> OpaqueRoute {
        self.0
    }
}

/// A route that is published and currently reachable by clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CurrentRoute(OpaqueRoute);

impl CurrentRoute {
    /// Records a published route.
    #[must_use]
    pub const fn new(route: OpaqueRoute) -> Self {
        Self(route)
    }

    /// Returns the opaque route.
    #[must_use]
    pub const fn route(self) -> OpaqueRoute {
        self.0
    }
}

/// One fenced ingress control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IngressRouteRequest {
    fence: PlacementFence,
    route: IngressRouteId,
}

impl IngressRouteRequest {
    /// Names the route and the placement fence that authorizes the change.
    #[must_use]
    pub const fn new(fence: PlacementFence, route: IngressRouteId) -> Self {
        Self { fence, route }
    }

    /// Returns the fence that must still be current at the ingress plane.
    #[must_use]
    pub const fn fence(self) -> PlacementFence {
        self.fence
    }

    /// Returns the route this request controls.
    #[must_use]
    pub const fn route(self) -> IngressRouteId {
        self.route
    }

    /// Returns the claim an ingress adapter rechecks before publishing.
    #[must_use]
    pub const fn publish_claim(self) -> FenceClaim {
        FenceClaim::new(
            self.fence.world_instance(),
            self.fence.generation(),
            FencedEffect::PublishIngressRoute,
        )
    }
}

/// Closed failures returned by an ingress control adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IngressError {
    /// The request no longer names the current placement fence.
    #[error(transparent)]
    StalePlacement(#[from] StalePlacement),
    /// Publication was attempted before the route was staged.
    #[error("ingress route {route} is not staged")]
    RouteNotStaged {
        /// The unstaged route.
        route: IngressRouteId,
    },
    /// The ingress plane is unreachable or refusing work.
    #[error("ingress control is unavailable")]
    Unavailable,
}

/// Generation-fenced control of the opaque client-facing route.
pub trait IngressControl: Send + Sync {
    /// Prepares an unpublished route for the current placement.
    fn stage(
        &self,
        request: IngressRouteRequest,
    ) -> PortFuture<'_, Result<StagedRoute, IngressError>>;

    /// Publishes a staged route once the current fence committed readiness.
    fn publish(
        &self,
        request: IngressRouteRequest,
    ) -> PortFuture<'_, Result<CurrentRoute, IngressError>>;

    /// Withdraws a route idempotently.
    fn withdraw(&self, request: IngressRouteRequest) -> PortFuture<'_, Result<(), IngressError>>;
}
