//! The reusable engine-service pattern (ADR 0026), expressed as a small
//! contract a *second* standard service (e.g. `azoth.persistence`) implements
//! to get the same composition without new machinery.
//!
//! A standard service is: a contract gem + a selected provider gem + the
//! provider's platform gem + optional adapter/claims + a storage contribution,
//! lowered by the target generator into one role-isolated host target. The
//! engine owns the host lifecycle and keys; the provider is one step.

/// Describes how a standard service plugs into project config and target
/// generation. Auth implements this as [`AuthService`]; a persistence service
/// would implement it analogously.
pub trait ServiceContract {
    /// The `[services.<key>]` table this service reads (e.g. `auth`).
    fn config_key() -> &'static str;

    /// The generated target directory name under
    /// `<project-target>/azoth/targets/` (e.g. `auth-host`).
    fn target_name() -> &'static str;

    /// The gem target role contributions are filtered to. Standard services use
    /// the shared `named-service` role (ADR 0024); a service instance is
    /// distinguished by its config selection and [`ServiceContract::target_name`],
    /// not by a bespoke role — so a later persistence service reuses this
    /// machinery without a new role variant.
    #[must_use]
    fn target_role() -> &'static str {
        "named-service"
    }
}

/// The auth service's binding of the pattern.
pub struct AuthService;

impl ServiceContract for AuthService {
    fn config_key() -> &'static str {
        "auth"
    }

    fn target_name() -> &'static str {
        "auth-host"
    }
}
