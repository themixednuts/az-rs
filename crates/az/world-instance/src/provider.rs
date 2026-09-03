//! The placement provider port the lifecycle reconciler consumes.
//!
//! A local `InstanceHost` supervising statically composed generated `server`
//! children and an external asynchronous provider implement the same trait.
//! Gameplay cannot select machines, regions, processes, sockets, ports, or
//! packing, and no provider-native type appears in this signature: provider
//! facts stay below the adapter and never become lifecycle identities.

use crate::{
    FenceClaim, FencedEffect, PlacementFence, PortFuture, ServerProcessId, StalePlacement,
    WorldInstancePlacementId,
};

/// Why a provider refused a placement request outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlacementRejection {
    /// No resource class satisfies the sealed semantic requirements.
    #[error("placement requirements are unsatisfiable")]
    UnsatisfiableRequirements,
    /// Feasible resources exist but none are currently available.
    #[error("placement capacity is exhausted")]
    CapacityExhausted,
    /// The named launch product is not runnable by this provider.
    #[error("placement release is incompatible")]
    IncompatibleRelease,
}

/// Closed failures returned by a placement provider adapter.
///
/// Provider-native errors are translated once at the adapter; no SDK error type
/// crosses this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlacementProviderError {
    /// The request no longer names the current placement fence.
    #[error(transparent)]
    StalePlacement(#[from] StalePlacement),
    /// The provider refused the request.
    #[error(transparent)]
    Rejected(#[from] PlacementRejection),
    /// The provider is unreachable or refusing work.
    #[error("placement provider is unavailable")]
    Unavailable,
    /// The outcome is unknown; the reconciler must observe the same request.
    #[error("placement provider outcome is indeterminate")]
    Indeterminate,
}

/// A placement control request that carries no launch input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacementRequest {
    fence: PlacementFence,
    placement: WorldInstancePlacementId,
}

impl PlacementRequest {
    /// Names the exact placement attempt and the fence that authorizes it.
    #[must_use]
    pub const fn new(fence: PlacementFence, placement: WorldInstancePlacementId) -> Self {
        Self { fence, placement }
    }

    /// Returns the fence that must still be current at the provider.
    #[must_use]
    pub const fn fence(self) -> PlacementFence {
        self.fence
    }

    /// Returns the placement attempt this request controls.
    #[must_use]
    pub const fn placement(self) -> WorldInstancePlacementId {
        self.placement
    }

    /// Returns the claim an adapter rechecks before mutating lifecycle state.
    #[must_use]
    pub const fn claim(self) -> FenceClaim {
        FenceClaim::new(
            self.fence.world_instance(),
            self.fence.generation(),
            FencedEffect::MutateLifecycleState,
        )
    }
}

/// A request to converge one placement onto a sealed launch product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnsurePlacement {
    request: PlacementRequest,
    launch_product_digest: [u8; 32],
}

impl EnsurePlacement {
    /// Binds a placement attempt to the sealed product it must run.
    #[must_use]
    pub const fn new(request: PlacementRequest, launch_product_digest: [u8; 32]) -> Self {
        Self {
            request,
            launch_product_digest,
        }
    }

    /// Returns the fenced placement control request.
    #[must_use]
    pub const fn request(self) -> PlacementRequest {
        self.request
    }

    /// Returns the sealed launch-product digest the provider must run.
    #[must_use]
    pub const fn launch_product_digest(&self) -> &[u8; 32] {
        &self.launch_product_digest
    }
}

/// What the provider currently reports about one placement attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlacementObservation {
    /// The provider accepted the request and has not settled it.
    Pending,
    /// The placement owns an authoritative server process.
    Running {
        /// The process owning the authoritative mutable failure domain.
        process: ServerProcessId,
    },
    /// The placement stopped admitting and is converging on its barriers.
    Draining,
    /// The placement released its resources.
    Released,
}

/// Provider-neutral placement control consumed by the lifecycle reconciler.
pub trait PlacementProvider: Send + Sync {
    /// Converges one placement attempt onto its sealed launch product.
    fn ensure(
        &self,
        request: EnsurePlacement,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>>;

    /// Resolves an indeterminate result for the same placement attempt.
    fn observe(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>>;

    /// Stops new work on a placement without releasing it.
    fn begin_drain(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>>;

    /// Releases a placement's provider resources.
    fn release(
        &self,
        request: PlacementRequest,
    ) -> PortFuture<'_, Result<PlacementObservation, PlacementProviderError>>;
}
