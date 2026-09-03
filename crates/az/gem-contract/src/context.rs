use std::marker::PhantomData;

#[cfg(feature = "app")]
use bevy_app::App;

use crate::capability::{CapSet, CapabilityWitness, CapsDecl, NarrowTo, Satisfies};
#[cfg(feature = "app")]
use crate::capability::{HostsWorld, Provides};
use crate::descriptor::ProductActivation;
use crate::instance::InstanceId;
use crate::registry::{Registrar, Registries, RegistryEntry};
use crate::report::DeclinedNarrowing;
use crate::role::GemTargetRole;

/// Borrowed composer state a context writes into. Composer-side plumbing, not
/// part of the author-facing surface.
pub struct ContextParts<'a> {
    pub provided: CapSet,
    pub role: GemTargetRole,
    pub activation: ProductActivation,
    #[cfg(feature = "app")]
    pub app: Option<&'a mut App>,
    pub registries: &'a mut Registries,
    pub raw_app_access: &'a mut Vec<InstanceId>,
    pub declined_narrowings: &'a mut Vec<DeclinedNarrowing>,
}

/// The typed registration surface handed to a contribution's `register`.
///
/// `D` is the contribution's declared capability set; registrars and hatches
/// that need a host capability are bounded on it, so an invalid registration
/// is a compile error at the author's call site (E0277), never a silent skip.
pub struct GemContext<'a, D> {
    instance: InstanceId,
    provided: CapSet,
    role: GemTargetRole,
    activation: ProductActivation,
    #[cfg(feature = "app")]
    app: Option<&'a mut App>,
    registries: &'a mut Registries,
    raw_app_access: &'a mut Vec<InstanceId>,
    declined_narrowings: &'a mut Vec<DeclinedNarrowing>,
    _caps: PhantomData<D>,
}

impl<'a, D: CapsDecl> GemContext<'a, D> {
    /// The witness is the only constructor: claiming it already proved
    /// containment, so a context implies a passed containment test.
    pub(crate) const fn new(
        _witness: CapabilityWitness<D>,
        instance: InstanceId,
        parts: ContextParts<'a>,
    ) -> Self {
        // Destructured rather than read field by field: without the `app`
        // feature every remaining field is a `Copy` scalar or a `&mut` that a
        // struct expression would silently reborrow, and the composer's parts
        // must actually move into the context.
        let ContextParts {
            provided,
            role,
            activation,
            #[cfg(feature = "app")]
            app,
            registries,
            raw_app_access,
            declined_narrowings,
        } = parts;
        Self {
            instance,
            provided,
            role,
            activation,
            #[cfg(feature = "app")]
            app,
            registries,
            raw_app_access,
            declined_narrowings,
            _caps: PhantomData,
        }
    }
}

impl<D> GemContext<'_, D> {
    #[must_use]
    pub const fn instance(&self) -> InstanceId {
        self.instance
    }

    #[must_use]
    pub const fn role(&self) -> GemTargetRole {
        self.role
    }

    /// What the project selected for this instance, in the runtime form.
    ///
    /// Composition data, exactly like the attribution a registrar carries: it
    /// arrives at [`Composer::add`](crate::Composer::add) beside the
    /// contribution and is read here. Product capabilities are the other
    /// vocabulary (ADR 0041) — "what did we build", not "what does the host
    /// provide" — so this is feature-free and carries no capability bound: a
    /// contribution asks what is enabled, then registers accordingly.
    #[must_use]
    pub const fn activation(&self) -> ProductActivation {
        self.activation
    }

    /// Typed registrar for one registry family. Attribution is captured
    /// automatically; the entry type's own capability floor is enforced here.
    pub fn registrar<T>(&mut self) -> Registrar<'_, T>
    where
        T: RegistryEntry,
        D: Satisfies<T::Requires>,
    {
        self.registries.get_mut::<T>().registrar(self.instance)
    }

    /// The sanctioned hatch: bounded by `HostsWorld` and reported. Every
    /// contribution that touches raw `&mut App` is named in the compose
    /// report (ADR 0041).
    ///
    /// Part of the `app` feature. Declaring `HostsWorld` is still the
    /// capability question and stays feature-free; this is the only place the
    /// declaration cashes out into a Bevy value.
    ///
    /// # Panics
    ///
    /// Never panics when constructed by the composer: hosts providing
    /// `HostsWorld` always own an `App`.
    #[cfg(feature = "app")]
    pub fn app(&mut self) -> &mut App
    where
        D: Provides<HostsWorld>,
    {
        tracing::debug!(instance = %self.instance, "raw &mut App access");
        if !self.raw_app_access.contains(&self.instance) {
            self.raw_app_access.push(self.instance);
        }
        self.app
            .as_deref_mut()
            .expect("hosts providing HostsWorld construct an App")
    }

    /// Optional narrowing above the declared floor. Typed inside the closure;
    /// a declined narrowing is visible in the compose report, never silent.
    pub fn when<C>(&mut self, f: impl FnOnce(&mut GemContext<'_, C::Out>))
    where
        C: NarrowTo<D>,
    {
        if self.provided.has(C::VALUE) {
            let mut narrowed = GemContext {
                instance: self.instance,
                provided: self.provided,
                role: self.role,
                activation: self.activation,
                #[cfg(feature = "app")]
                app: self.app.as_deref_mut(),
                registries: &mut *self.registries,
                raw_app_access: &mut *self.raw_app_access,
                declined_narrowings: &mut *self.declined_narrowings,
                _caps: PhantomData,
            };
            f(&mut narrowed);
        } else {
            tracing::debug!(
                instance = %self.instance,
                capability = %C::VALUE,
                "narrowing declined"
            );
            self.declined_narrowings.push(DeclinedNarrowing {
                instance: self.instance,
                capability: C::VALUE,
            });
        }
    }
}
