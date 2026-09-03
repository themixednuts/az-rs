use std::fmt;
use std::marker::PhantomData;

use crate::ids::{ContributionId, GemId};
use crate::role::GemTargetRole;

/// Closed host-capability lattice (ADR 0041).
///
/// Roles are named bundles of these four axes. Set membership is expressed
/// with this enum; bit packing is private to [`CapSet`].
///
/// The serde form is the variant name verbatim — `"HostsWorld"`, not a
/// kebab-cased rename — because one spelling has to serve `gem.toml`'s
/// `caps = [...]`, the entry item's `#[contribution(caps = [...])]` tokens,
/// and the `capabilities` array of a generated load manifest.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(u8)]
pub enum HostCapability {
    /// The composition exposes a Bevy `App`.
    HostsWorld = 0,
    /// Client-side presentation / projection.
    HasClient = 1,
    /// Authoritative simulation.
    HasAuthority = 2,
    /// Owns a fixed-rate clock, independent of authority.
    OwnsFixedClock = 3,
}

impl HostCapability {
    pub const ALL: [Self; 4] = [
        Self::HostsWorld,
        Self::HasClient,
        Self::HasAuthority,
        Self::OwnsFixedClock,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::HostsWorld => "HostsWorld",
            Self::HasClient => "HasClient",
            Self::HasAuthority => "HasAuthority",
            Self::OwnsFixedClock => "OwnsFixedClock",
        }
    }

    const fn mask(self) -> u8 {
        1 << (self as u8)
    }
}

impl fmt::Display for HostCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Value-level capability set.
///
/// Membership uses [`HostCapability`]; on the dynamic seam this set is a
/// digest input alongside toolchain and contract version (ADR 0035
/// fingerprint extension).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CapSet {
    bits: u8,
}

impl CapSet {
    pub const EMPTY: Self = Self { bits: 0 };

    #[must_use]
    pub const fn with(self, capability: HostCapability) -> Self {
        Self {
            bits: self.bits | capability.mask(),
        }
    }

    #[must_use]
    pub const fn has(self, capability: HostCapability) -> bool {
        self.bits & capability.mask() == capability.mask()
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    #[must_use]
    pub const fn missing_from(self, provided: Self) -> Self {
        Self {
            bits: self.bits & !provided.bits,
        }
    }

    pub fn capabilities(self) -> impl Iterator<Item = HostCapability> {
        HostCapability::ALL
            .into_iter()
            .filter(move |capability| self.has(*capability))
    }
}

impl From<HostCapability> for CapSet {
    fn from(capability: HostCapability) -> Self {
        Self::EMPTY.with(capability)
    }
}

impl FromIterator<HostCapability> for CapSet {
    fn from_iter<I: IntoIterator<Item = HostCapability>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl fmt::Display for CapSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{{")?;
        let mut first = true;
        for capability in self.capabilities() {
            if !first {
                write!(formatter, "+")?;
            }
            first = false;
            write!(formatter, "{capability}")?;
        }
        write!(formatter, "}}")
    }
}

impl fmt::Debug for CapSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Type-level token for one [`HostCapability`] variant.
///
/// Bounds like `where D: Provides<HasClient>` and `ctx.when::<HasClient>(…)`
/// can name an axis in the type system. The value-level currency is the enum.
pub trait HostCapabilityKind: sealed::Sealed + 'static {
    const VALUE: HostCapability;
}

#[diagnostic::on_unimplemented(
    message = "host capability `{C}` is not declared by `{Self}`",
    label = "requires host capability `{C}`",
    note = "declare the capability in this contribution's floor (`caps = [...]` or `declare_caps!`), or use `GemContext::when` for optional narrowing"
)]
pub trait Provides<C: HostCapabilityKind> {}

/// A contribution's declared capability floor.
///
/// The associated const carries the value the witness containment test and
/// the dynamic-seam fingerprint digest use; the trait impls carry the same
/// facts for the type system.
pub trait CapsDecl: 'static {
    const REQUIRED: CapSet;
}

#[macro_export]
macro_rules! declare_caps {
    ($vis:vis $name:ident: $($cap:ident),* $(,)?) => {
        $vis struct $name(());

        $(impl $crate::Provides<$crate::$cap> for $name {})*

        impl $crate::CapsDecl for $name {
            const REQUIRED: $crate::CapSet = {
                let set = $crate::CapSet::EMPTY;
                $(let set = set.with(<$crate::$cap as $crate::HostCapabilityKind>::VALUE);)*
                set
            };
        }
    };
}

/// Capability floor for a registry family.
///
/// [`Unconditional`] or one of the four capability tokens. Carried by
/// [`crate::RegistryEntry::Requires`] so the bound is on the entry type
/// itself and cannot be bypassed at a call site.
pub trait Requirement: sealed::Sealed + 'static {}

/// No capability floor: the registry is valid under every role.
pub struct Unconditional(());

impl sealed::Sealed for Unconditional {}
impl Requirement for Unconditional {}

#[diagnostic::on_unimplemented(
    message = "this registrar's capability floor `{R}` is not declared by `{Self}`",
    label = "this registry requires host capability `{R}`",
    note = "declare the capability in this contribution's floor (`caps = [...]` or `declare_caps!`), or move the call inside `GemContext::when`"
)]
pub trait Satisfies<R: Requirement> {}

impl<D> Satisfies<Unconditional> for D {}

/// Maps a capability token to the narrowed caps type.
///
/// [`crate::GemContext::when`] hands this associated type to its closure.
pub trait NarrowTo<D>: HostCapabilityKind {
    type Out;
}

/// The type-level mirror of the closed lattice: one row per axis.
///
/// A row is `token => narrowing wrapper forwards [the other three axes]`, and
/// it stamps everything the type system knows about that axis — the token and
/// its [`HostCapabilityKind`] value, its [`Requirement`]/[`Satisfies`] pair,
/// and the wrapper [`NarrowTo`] hands to a `when` closure. Growing the lattice
/// is one enum variant plus one row.
macro_rules! capabilities {
    ($($token:ident => $wrapper:ident forwards [$($other:ident),*]),+ $(,)?) => {$(
        #[doc = concat!("Type token for [`HostCapability::", stringify!($token), "`].")]
        pub struct $token(());

        impl sealed::Sealed for $token {}

        impl HostCapabilityKind for $token {
            const VALUE: HostCapability = HostCapability::$token;
        }

        impl Requirement for $token {}

        impl<D: Provides<$token>> Satisfies<$token> for D {}

        #[doc = concat!(
            "Caps type inside `ctx.when::<", stringify!($token),
            ">(…)`: the base declaration `D` plus `", stringify!($token), "`."
        )]
        pub struct $wrapper<D>(PhantomData<D>);

        impl<D> Provides<$token> for $wrapper<D> {}
        $(impl<D: Provides<$other>> Provides<$other> for $wrapper<D> {})*

        impl<D> NarrowTo<D> for $token {
            type Out = $wrapper<D>;
        }
    )+};
}

capabilities! {
    HostsWorld => WithHostsWorld forwards [HasClient, HasAuthority, OwnsFixedClock],
    HasClient => WithClient forwards [HostsWorld, HasAuthority, OwnsFixedClock],
    HasAuthority => WithAuthority forwards [HostsWorld, HasClient, OwnsFixedClock],
    OwnsFixedClock => WithFixedClock forwards [HostsWorld, HasClient, HasAuthority],
}

/// A refused claim: the containment failure, loud by construction.
///
/// `Display` names the gem, the contribution, the host role, both sets, and
/// the exact missing axes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub gem: GemId,
    pub contribution: ContributionId,
    pub role: GemTargetRole,
    pub required: CapSet,
    pub provided: CapSet,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "gem `{}` contribution `{}` requires {} but host `{}` provides {} (missing {})",
            self.gem,
            self.contribution,
            self.required,
            self.role,
            self.provided,
            self.required.missing_from(self.provided),
        )
    }
}

impl std::error::Error for Refusal {}

/// The role a host composes under plus the capability set that role implies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostDecl {
    role: GemTargetRole,
    provided: CapSet,
}

impl HostDecl {
    #[must_use]
    pub const fn from_role(role: GemTargetRole) -> Self {
        Self {
            role,
            provided: role.provided_caps(),
        }
    }

    #[must_use]
    pub const fn role(self) -> GemTargetRole {
        self.role
    }

    #[must_use]
    pub const fn provided(self) -> CapSet {
        self.provided
    }
}

/// The only constructor of a typed [`crate::GemContext`].
///
/// Claiming performs exactly one total narrowing — the containment test
/// `provided ⊇ required` — before any registration runs (ADR 0041).
pub struct CapabilityWitness<D: CapsDecl> {
    _decl: PhantomData<D>,
}

impl<D: CapsDecl> CapabilityWitness<D> {
    /// # Errors
    ///
    /// Returns [`Refusal`] when the host set does not contain every
    /// capability in `D::REQUIRED`.
    pub const fn claim(
        gem: GemId,
        contribution: ContributionId,
        host: HostDecl,
    ) -> Result<Self, Refusal> {
        if host.provided().contains(D::REQUIRED) {
            Ok(Self { _decl: PhantomData })
        } else {
            Err(Refusal {
                gem,
                contribution,
                role: host.role(),
                required: D::REQUIRED,
                provided: host.provided(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    declare_caps!(TestCaps: HasClient, HostsWorld);

    #[test]
    fn set_membership_uses_the_enum() {
        let set = CapSet::EMPTY
            .with(HostCapability::HostsWorld)
            .with(HostCapability::HasClient);
        assert!(set.has(HostCapability::HasClient));
        assert!(!set.has(HostCapability::HasAuthority));
        assert_eq!(set.to_string(), "{HostsWorld+HasClient}");
        assert_eq!(set.capabilities().count(), 2);
    }

    /// One spelling serves `gem.toml`, the `#[contribution]` attribute, and
    /// the generated load manifest, so the serde form is pinned here.
    #[test]
    fn the_serde_form_is_the_token_name() {
        assert_eq!(
            serde_json::to_string(&HostCapability::ALL).unwrap(),
            r#"["HostsWorld","HasClient","HasAuthority","OwnsFixedClock"]"#
        );
        for capability in HostCapability::ALL {
            let json = serde_json::to_string(&capability).unwrap();
            assert_eq!(json, format!("\"{}\"", capability.name()));
            assert_eq!(
                serde_json::from_str::<HostCapability>(&json).unwrap(),
                capability
            );
        }
    }

    #[test]
    fn an_axis_outside_the_sealed_four_is_a_load_error() {
        let error = serde_json::from_str::<HostCapability>("\"IsFast\"").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("unknown variant `IsFast`"), "{message}");
        for capability in HostCapability::ALL {
            assert!(message.contains(capability.name()), "{message}");
        }
    }

    #[test]
    fn containment_passes_when_host_is_a_superset() {
        let host = HostDecl::from_role(GemTargetRole::Client);
        assert!(
            CapabilityWitness::<TestCaps>::claim(
                GemId::new("gem.test"),
                ContributionId::new("runtime"),
                host,
            )
            .is_ok()
        );
    }

    #[test]
    fn containment_fails_loudly() {
        let host = HostDecl::from_role(GemTargetRole::AssetWorker);
        let Err(error) = CapabilityWitness::<TestCaps>::claim(
            GemId::new("gem.test"),
            ContributionId::new("runtime"),
            host,
        ) else {
            panic!("expected containment failure on AssetWorker");
        };
        let message = error.to_string();
        assert!(message.contains("HasClient"));
        assert!(message.contains("asset-worker"));
    }
}
