//! Typed gem contribution contract for Azoth (ADRs 0032, 0033, 0041).
//!
//! Registration is ordinary, typed Rust: each contribution crate exports one
//! entry item implementing [`Contribution`]; generated target glue constructs
//! it by name and `add`s it to a per-host [`Composer`]. Registries are owned
//! values composed per host — no process-global registry state, no linker
//! dependence, no anchors.
//!
//! Role conditioning is type-level: a contribution declares its capability
//! floor with [`declare_caps!`], registrars are bounded on the context's
//! capability parameter, and the [`CapabilityWitness`] containment test
//! (`provided ⊇ required`) runs before any registration. The miss paths are
//! loud: E0277 at the author's call site on the static seam, a recorded
//! refusal on the dynamic seam.
//!
//! Identity, roles, and that floor are one attribute on the entry item's
//! `impl Contribution` block: [`macro@contribution`] generates the same
//! declaration [`declare_caps!`] would have written, wires `type Caps` to it,
//! and emits the pure-data descriptor accessor. `register` stays hand-written.
//!
//! With no arguments the attribute reads all of it out of the gem's own
//! `gem.toml` — the entry whose `package` is the crate being compiled — and
//! emits the entry item generated glue calls as well, so a gem's entire
//! hand-written registration surface is a type and a `register` body:
//!
//! ```ignore
//! struct Runtime;
//!
//! #[contribution]
//! impl Contribution for Runtime {
//!     fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) { /* … */ }
//! }
//! ```
//!
//! A package that implements several of its gem's contributions names which one
//! each block is with a bare token — `#[contribution(runtime)]`,
//! `#[contribution(runtime_host)]` — matched against that package's own
//! declarations. The explicit
//! `#[contribution(gem = …, id = …, roles = […], caps = […])]` form spells the
//! same facts at the call site, for code with no manifest above it — the tests
//! in this crate, and spikes. All three forms produce one expansion.
//!
//! What the project *selected* is not part of that identity: [`ProductActivation`]
//! is per-instance composition data, so it arrives at [`Composer::add`] beside
//! the contribution and is read back through [`GemContext::activation`].
//!
//! # The `app` feature
//!
//! Bevy is the contract's edge, not its substance. The on-by-default `app`
//! feature carries the whole Bevy edge — the `App` re-export,
//! `GemContext::app`, and the `App` a `HostsWorld` host composes into.
//! Everything a registrar-only crate needs is outside it: ids, capabilities
//! and narrowing, descriptors, the contribution trait, registries, clocks,
//! the compose report, roles, and session mode. Model and domain crates that
//! only name registry entry types take this crate with
//! `default-features = false` and stay Bevy-free; hosts that own a world take
//! the default.
//!
//! [`ComposeReport`]'s shape does not move with the feature: without `app`,
//! the raw-access section is simply always empty.
//!
//! # Observability
//!
//! Composition emits `tracing` at the same points [`ComposeReport`] records,
//! never a second set of books, and the level is the contract a binary filters
//! on: `trace` is one event per registry entry, `debug` is the sanctioned
//! hatches — raw `&mut App` access and a declined `when` narrowing — `info` is
//! lifecycle (a composed contribution, a withdrawal, the finalize summary, and
//! the `compose` span each `add` runs inside), `warn` is a refusal, which is
//! recoverable by design, and `error` is a compose failure. The library is
//! unconditional: a ship binary sets its own static ceiling with `tracing`'s
//! `release_max_level_*` features, so the per-entry `trace` costs nothing —
//! and even without a ceiling, key formatting happens only inside the macro.

#![forbid(unsafe_code)]

mod capability;
mod clock;
mod composer;
mod context;
mod contribution;
mod descriptor;
mod error;
mod ids;
mod instance;
pub mod native;
mod process;
mod registry;
mod report;
mod role;
mod session;

pub use az_gem_contract_derive::contribution;
#[cfg(feature = "app")]
pub use bevy_app::App;

pub use capability::{
    CapSet, CapabilityWitness, CapsDecl, HasAuthority, HasClient, HostCapability,
    HostCapabilityKind, HostDecl, HostsWorld, NarrowTo, OwnsFixedClock, Provides, Refusal,
    Requirement, Satisfies, Unconditional, WithAuthority, WithClient, WithFixedClock,
    WithHostsWorld,
};
pub use clock::{ClockDefinition, ClockUse};
pub use composer::Composer;
pub use context::GemContext;
pub use contribution::Contribution;
pub use descriptor::{ContributionDescriptor, ProductActivation};
pub use error::ComposeError;
pub use ids::{CapabilityId, ClockName, ContributionId, GemId, GraphTypeId, NodeTypeId};
pub use instance::InstanceId;
pub use process::{
    CompositionShutdownGate, ProcessComposition, ProcessCompositionCleanupError, RegistryLease,
    RegistryLeaseError,
};
pub use registry::{Attributed, Registrar, Registries, Registry, RegistryEntry};
pub use report::{ComposeReport, DeclinedNarrowing, Registration, ResolvedClock};
pub use role::GemTargetRole;
pub use session::SessionMode;

/// The author-facing surface: everything a gem's entry item needs.
///
/// Small on purpose. Under the zero-argument [`macro@contribution`] a gem
/// author writes a type, a `register` body, and the registry entry types that
/// body names — identity, roles, the capability floor, and the entry item's own
/// name all arrive from `gem.toml`. [`GemId`] and [`ContributionId`] are here
/// for the one thing the manifest cannot supply: naming *another* gem's
/// contribution, which travels as a const from that gem's generated ids crate.
pub mod prelude {
    pub use crate::{
        CapabilityId, ClockDefinition, ClockName, ClockUse, Contribution, ContributionDescriptor,
        ContributionId, GemContext, GemId, GemTargetRole, HasAuthority, HasClient, HostCapability,
        HostsWorld, OwnsFixedClock, Provides, Registrar, RegistryEntry, Satisfies, Unconditional,
    };
    pub use crate::{contribution, declare_caps};
}
