//! Refined lifecycle identities and the placement fence.
//!
//! ADR 0052 requires that stable instance, placement, process, route,
//! admission, presence, transfer, checkpoint, and operation identities are
//! distinct refined types. They are stamped from one closed list below so the
//! set stays enumerable: a tenth identity cannot appear without joining
//! [`WorldInstanceIdentityKind`], and the compile-fail fixtures under
//! `tests/ui/fail` prove no two of them are interchangeable.

use std::{fmt, num::NonZeroU64};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A refined identity rejected the nil UUID.
///
/// The failing identity is carried so a protocol adapter can attribute the
/// rejection without re-deriving it from the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{kind} identity must not be nil")]
pub struct InvalidWorldIdentity {
    kind: WorldInstanceIdentityKind,
}

impl InvalidWorldIdentity {
    /// Returns which refined identity rejected the value.
    #[must_use]
    pub const fn kind(self) -> WorldInstanceIdentityKind {
        self.kind
    }
}

/// Stamps one refined UUID identity per entry of the closed ADR 0052 list.
///
/// A declarative macro is used because the identities are a genuinely closed
/// set with identical construction rules; each still becomes its own nominal
/// type, so passing one where another is expected fails to compile.
macro_rules! refined_identities {
    ($(
        $(#[$identity_doc:meta])*
        $name:ident => $kind:ident, $label:literal;
    )+) => {
        /// Closed enumeration of every refined world-instance identity.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum WorldInstanceIdentityKind {
            $(
                #[doc = concat!("The ", $label, " identity.")]
                $kind,
            )+
        }

        impl WorldInstanceIdentityKind {
            /// Every refined identity the lifecycle owns, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$kind),+];
        }

        impl fmt::Display for WorldInstanceIdentityKind {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(match *self {
                    $(Self::$kind => $label,)+
                })
            }
        }

        $(
            $(#[$identity_doc])*
            #[derive(
                Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
            )]
            #[serde(transparent)]
            pub struct $name(Uuid);

            impl $name {
                /// Allocates a time-ordered identity.
                #[must_use]
                pub fn new() -> Self {
                    Self(Uuid::now_v7())
                }

                /// Restores a protocol or persistence identity after rejecting nil.
                ///
                /// # Errors
                ///
                /// Returns [`InvalidWorldIdentity`] for the nil UUID.
                pub const fn try_from_uuid(value: Uuid) -> Result<Self, InvalidWorldIdentity> {
                    if value.is_nil() {
                        Err(InvalidWorldIdentity {
                            kind: WorldInstanceIdentityKind::$kind,
                        })
                    } else {
                        Ok(Self(value))
                    }
                }

                /// Returns the representation used by protocol and persistence adapters.
                #[must_use]
                pub const fn as_uuid(self) -> Uuid {
                    self.0
                }
            }

            impl Default for $name {
                fn default() -> Self {
                    Self::new()
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    self.0.fmt(formatter)
                }
            }
        )+
    };
}

refined_identities! {
    /// Stable identity of one logical authoritative simulation occurrence.
    ///
    /// It carries no execution fact and survives every placement, process, and
    /// provider replacement.
    WorldInstanceId => Instance, "world instance";

    /// Identity of one replaceable execution attempt for a world instance.
    WorldInstancePlacementId => Placement, "world instance placement";

    /// Identity of one operating-system process owning an authoritative
    /// mutable simulation failure domain.
    ServerProcessId => ServerProcess, "server process";

    /// Identity of one opaque virtual ingress route.
    ///
    /// A route is never admission; publishing one grants no membership.
    IngressRouteId => IngressRoute, "ingress route";

    /// Identity of one single-use, client-key-bound admission ticket.
    AdmissionTicketId => AdmissionTicket, "admission ticket";

    /// Identity of one player's presence record.
    ///
    /// Presence is the no-double-presence authority; it is not a transport
    /// session and not an admission ticket.
    PlayerPresenceId => PlayerPresence, "player presence";

    /// Identity of one durable transfer saga between world instances.
    WorldTransferId => WorldTransfer, "world transfer";

    /// Identity of one committed placement-local checkpoint.
    CheckpointId => Checkpoint, "checkpoint";

    /// Durable idempotency identity for one requested lifecycle change.
    WorldInstanceOperationId => Operation, "world-instance operation";
}

/// A placement generation must be nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("placement generation must be nonzero")]
pub struct InvalidPlacementGeneration;

/// Monotonic fencing generation scoped to one [`WorldInstanceId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlacementGeneration(NonZeroU64);

impl PlacementGeneration {
    /// Returns the first placement generation.
    #[must_use]
    pub const fn initial() -> Self {
        Self(NonZeroU64::MIN)
    }

    /// Constructs a generation from an already checked nonzero value.
    #[must_use]
    pub const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }

    /// Returns the integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Returns the next generation, or `None` when the counter is exhausted.
    #[must_use]
    pub fn checked_next(self) -> Option<Self> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
    }
}

impl TryFrom<u64> for PlacementGeneration {
    type Error = InvalidPlacementGeneration;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(InvalidPlacementGeneration)
    }
}

impl fmt::Display for PlacementGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The lifecycle effects a stale placement must never perform.
///
/// ADR 0052 names five: publish readiness, admit players, checkpoint, submit
/// outcomes, and mutate lifecycle state. The accepted migration additionally
/// requires a placement fence on ingress `stage`/`publish`/`withdraw`, so route
/// publication is the sixth member rather than an unfenced special case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FencedEffect {
    /// Reporting a readiness gate for the placement.
    PublishReadiness,
    /// Publishing an ingress route that reaches the placement.
    PublishIngressRoute,
    /// Admitting a player by redeeming an admission ticket.
    AdmitPlayer,
    /// Committing a checkpoint header.
    CommitCheckpoint,
    /// Submitting the single terminal outcome.
    SubmitOutcome,
    /// Mutating durable lifecycle state.
    MutateLifecycleState,
}

impl fmt::Display for FencedEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match *self {
            Self::PublishReadiness => "readiness publication",
            Self::PublishIngressRoute => "ingress route publication",
            Self::AdmitPlayer => "player admission",
            Self::CommitCheckpoint => "checkpoint commit",
            Self::SubmitOutcome => "terminal outcome submission",
            Self::MutateLifecycleState => "lifecycle state mutation",
        })
    }
}

/// One placement's claim to perform a fenced lifecycle effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FenceClaim {
    world_instance: WorldInstanceId,
    generation: PlacementGeneration,
    effect: FencedEffect,
}

impl FenceClaim {
    /// Records which instance and generation wants to perform which effect.
    #[must_use]
    pub const fn new(
        world_instance: WorldInstanceId,
        generation: PlacementGeneration,
        effect: FencedEffect,
    ) -> Self {
        Self {
            world_instance,
            generation,
            effect,
        }
    }

    /// Returns the claimed logical instance.
    #[must_use]
    pub const fn world_instance(self) -> WorldInstanceId {
        self.world_instance
    }

    /// Returns the claimed placement generation.
    #[must_use]
    pub const fn generation(self) -> PlacementGeneration {
        self.generation
    }

    /// Returns the effect the claim wants to perform.
    #[must_use]
    pub const fn effect(self) -> FencedEffect {
        self.effect
    }
}

/// A fenced effect was refused because the claim is not the current placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "{effect} refused: world instance {claimed_instance} generation {claimed_generation} is not \
     the current fence (world instance {current_instance} generation {current_generation})"
)]
pub struct StalePlacement {
    effect: FencedEffect,
    claimed_instance: WorldInstanceId,
    claimed_generation: PlacementGeneration,
    current_instance: WorldInstanceId,
    current_generation: PlacementGeneration,
}

impl StalePlacement {
    /// Returns the refused effect.
    #[must_use]
    pub const fn effect(self) -> FencedEffect {
        self.effect
    }

    /// Returns the claim that was refused.
    #[must_use]
    pub const fn claim(self) -> FenceClaim {
        FenceClaim::new(self.claimed_instance, self.claimed_generation, self.effect)
    }

    /// Returns the fence that refused the claim.
    #[must_use]
    pub const fn current(self) -> PlacementFence {
        PlacementFence::new(self.current_instance, self.current_generation)
    }
}

/// Proof that one placement generation owns lifecycle effects for an instance.
///
/// A fence authorizes only its exact logical instance at its exact generation.
/// An older generation is superseded, a newer generation has not been granted,
/// and another instance is never in scope, so every mismatch fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlacementFence {
    world_instance: WorldInstanceId,
    generation: PlacementGeneration,
}

impl PlacementFence {
    /// Binds an instance to its current placement generation.
    #[must_use]
    pub const fn new(world_instance: WorldInstanceId, generation: PlacementGeneration) -> Self {
        Self {
            world_instance,
            generation,
        }
    }

    /// Returns the stable logical instance protected by this fence.
    #[must_use]
    pub const fn world_instance(self) -> WorldInstanceId {
        self.world_instance
    }

    /// Returns the exact placement generation protected by this fence.
    #[must_use]
    pub const fn generation(self) -> PlacementGeneration {
        self.generation
    }

    /// Returns the fence a recovery or replacement placement receives.
    ///
    /// Recovery advances the generation for the same logical instance, which is
    /// what makes the previous placement stale. `None` means the generation
    /// counter is exhausted and no further placement may be fenced.
    #[must_use]
    pub fn advanced(self) -> Option<Self> {
        self.generation
            .checked_next()
            .map(|generation| Self::new(self.world_instance, generation))
    }

    /// Returns whether this fence supersedes `earlier` for the same instance.
    ///
    /// Generations from different logical instances never order against each
    /// other, so a fence never supersedes another instance's placement.
    #[must_use]
    pub fn supersedes(self, earlier: Self) -> bool {
        self.world_instance == earlier.world_instance && self.generation > earlier.generation
    }

    /// Authorizes one fenced lifecycle effect.
    ///
    /// # Errors
    ///
    /// Returns [`StalePlacement`] whenever the claim names another logical
    /// instance or any generation other than this fence's own.
    pub fn authorize(self, claim: FenceClaim) -> Result<(), StalePlacement> {
        if claim.world_instance == self.world_instance && claim.generation == self.generation {
            return Ok(());
        }

        Err(StalePlacement {
            effect: claim.effect,
            claimed_instance: claim.world_instance,
            claimed_generation: claim.generation,
            current_instance: self.world_instance,
            current_generation: self.generation,
        })
    }
}
