//! `azoth.auth` — the provider-neutral authentication contract (ADR 0026).
//!
//! This gem owns the stable identity, credential, verification, and session
//! vocabulary that game code, provider gems, project adapters, and the
//! generated auth host all agree on. It never exposes Steam/EOS handles,
//! provider HTTP clients, callback types, or title-specific transport DTOs.
//!
//! # The reusable engine-service pattern
//!
//! Authentication is the first instance of a general shape that a later
//! `azoth.persistence` (or any standard service) can reuse without new
//! machinery:
//!
//! ```text
//! contract gem                      (this crate: neutral types + traits)
//!   + one selected provider gem     (azoth.auth.local / .steam / .eos)
//!   + the provider's platform gem   (azoth.steam / azoth.eos)
//!   + optional project adapter      (sample.auth-adapter: routes/DTOs/claims)
//!   + selected storage contribution (IdentityStore impl)
//!   -> generated role-isolated host (<project-target>/azoth/targets/auth-host)
//! ```
//!
//! The engine owns the pieces a security boundary must not delegate: the host
//! lifecycle, listener/TLS, request limits, telemetry, **key resolution and
//! session signing**, refresh-token protection, and revocation. Provider gems
//! only [`verify or exchange`](provider::AuthProvider::verify_or_exchange)
//! external credentials; they never receive session signing keys. See
//! [`service`] for the generic extension points a second service would
//! implement.
//!
//! Extension points a new standard service implements:
//! - a contract gem exposing its own neutral request/response vocabulary,
//! - a [`service::ServiceContract`] describing its config key and target role,
//! - one or more provider gems, and
//! - a host authority the engine composes (see the `host` feature).

pub mod credential;
pub mod extension;
pub mod host;
pub mod identity;
pub mod provider;
pub mod service;
pub mod session;
pub mod verification;

#[cfg(feature = "host")]
pub mod authority;
#[cfg(feature = "host")]
pub mod transport;

#[cfg(all(test, feature = "host"))]
mod tests;

pub use credential::{Audience, CredentialEnvelope, CredentialLease, CredentialScheme};
pub use extension::{AugmentedClaims, ClaimsAugmenter, IdentityStore};
pub use host::{AuthHost, HostHealth, HostMetadata, RequestLimits};
pub use identity::{Identity, IdentityState, ProviderId, ProviderSubject, SubjectId};
pub use provider::{AuthProvider, AuthProviderContext, CredentialSource, ProviderCapabilities};
pub use session::{Scope, SessionClaims, SessionGrant, SessionId};
pub use verification::{Assurance, AuthError, ProviderClaims, VerifiedExternalIdentity};

#[cfg(feature = "host")]
pub use authority::{
    Hs256SessionAuthority, ReferenceAuthHost, SessionAuthority, SessionAuthorityContext,
    session_authority,
};
#[cfg(feature = "host")]
pub use transport::serve;

use az_gem_contract::{Contribution, GemContext, contribution};

/// The gem's `contract` contribution.
///
/// ADR 0026's capability conventions — the `auth_provider` entry point a
/// provider gem exports, and `session_authority` here — are
/// *service* conventions the generated host calls by name. They are not
/// registration, and they stay exactly where they are. What this gem owns is
/// vocabulary: types and traits, no registry entry of any family. So the
/// registration surface is genuinely empty, and composing the contract claims
/// nothing.
///
/// Sealing is privacy: the generated `contract_contribution` is the only way in.
struct Contract;

#[contribution(contract)]
impl Contribution for Contract {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// The gem's `host` contribution: the reference session authority, linked into
/// the generated auth-host service.
///
/// The manifest declares this contribution unconditionally and the `host`
/// cargo feature decides what it *links*, so the entry item is unconditional
/// too and the feature shows up inside bodies, never in whether a declared
/// contribution exists. There is nothing inside this one: signing keys never
/// leave this side, and the authority reaches the host through
/// `session_authority`, a service convention rather than a registry.
struct Host;

#[contribution(host)]
impl Contribution for Host {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
