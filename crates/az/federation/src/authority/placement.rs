//! The lifecycle boundary an authority commit reads before it fences.
//!
//! Linear ADR 0052 owns placement, admission, presence, transfer, checkpoint,
//! drain, and destruction, and its `WorldInstanceService` is the only control
//! surface for them. Federation needs exactly one fact from that owner: which
//! placement generation is currently live for an instance. It reads that fact
//! through this consumer-owned port and creates no second lifecycle, fence
//! type, or admission model.
//!
//! The port is deliberately read-only. Adding a mutation here would move
//! lifecycle authority into federation, which ADR 0053 forbids.

use std::{future::Future, pin::Pin};

use az_world_instance::{PlacementFence, PlacementGeneration, WorldInstanceId};

use crate::CertifiedExecutionBinding;

/// The lifecycle owner could not answer, so no placement decision exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("world instance lifecycle authority is unavailable")]
pub struct PlacementAuthorityUnavailable;

/// A lifecycle adapter's asynchronous current-fence result.
pub type PlacementFenceFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<Option<PlacementFence>, PlacementAuthorityUnavailable>>
            + Send
            + 'a,
    >,
>;

/// Consumer-owned read of the lifecycle owner's current placement fence.
pub trait PlacementAuthorityPort: Send + Sync {
    /// Returns the live fence for an instance, or `None` when it has none.
    fn current_placement_fence(&self, world_instance: WorldInstanceId) -> PlacementFenceFuture<'_>;
}

/// Whether a certified binding still names the live placement generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementCurrency {
    /// The binding names the live generation for its instance.
    Current,
    /// A newer placement generation replaced the one in the binding.
    Superseded {
        /// The live generation reported by the lifecycle owner.
        current: PlacementGeneration,
    },
    /// The instance currently has no placement able to produce effects.
    Unplaced,
}

/// Compares a certified binding's placement fence with the live one.
///
/// A caller uses the result to refuse stale work before it reaches an
/// authority atom. Unavailability stays an error rather than collapsing into
/// `Unplaced`, so a lifecycle outage can never read as "this placement lost
/// its authority".
///
/// # Errors
///
/// Returns [`PlacementAuthorityUnavailable`] when the lifecycle owner cannot
/// currently answer.
pub async fn verify_placement_currency(
    placement: &dyn PlacementAuthorityPort,
    binding: &CertifiedExecutionBinding,
) -> Result<PlacementCurrency, PlacementAuthorityUnavailable> {
    let claimed = binding.placement();
    let live = placement
        .current_placement_fence(claimed.world_instance())
        .await?;
    // Comparing the whole fence keeps a misbehaving adapter fail-closed: a
    // fence naming another instance reads as superseded, never as current.
    Ok(match live {
        Some(fence) if fence == claimed => PlacementCurrency::Current,
        Some(fence) => PlacementCurrency::Superseded {
            current: fence.generation(),
        },
        None => PlacementCurrency::Unplaced,
    })
}
