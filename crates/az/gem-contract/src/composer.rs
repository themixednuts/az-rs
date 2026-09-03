use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "app")]
use bevy_app::App;

use crate::capability::{CapabilityWitness, HostDecl, Refusal};
use crate::clock::{ClockDefinition, ClockUse};
use crate::context::{ContextParts, GemContext};
use crate::contribution::{Contribution, ErasedContribution};
use crate::descriptor::ProductActivation;
use crate::error::ComposeError;
use crate::ids::{ContributionId, GemId};
use crate::instance::InstanceId;
use crate::registry::Registries;
use crate::report::{ComposeReport, DeclinedNarrowing, ResolvedClock};
use crate::role::GemTargetRole;

struct Composed {
    instance: InstanceId,
    contribution: Box<dyn ErasedContribution>,
}

/// Per-host owning composer (ADR 0032/0033).
///
/// Generated targets construct contributions in lock-closure order and `add`
/// them; the generated named call is the anchor, the inventory, and the
/// link-time check.
pub struct Composer {
    host: HostDecl,
    #[cfg(feature = "app")]
    app: Option<App>,
    // Registries become immutable once the composer has finalized. Keeping the
    // allocation shareable lets a process-owned service hand its worker tasks
    // an owned read lease without making the mutable composition shareable.
    registries: Arc<Registries>,
    contributions: Vec<Composed>,
    generations: HashMap<(GemId, ContributionId), u32>,
    raw_app_access: Vec<InstanceId>,
    declined_narrowings: Vec<DeclinedNarrowing>,
    refusals: Vec<Refusal>,
}

// Without `app` no role owns a Bevy `App`, so a composer is ordinary owned
// state and must stay movable across threads. This is the half of the
// invariant that is real, and it is asserted where the feature lives.
//
// With `app` the assertion is deliberately absent rather than merely
// unasserted: hosting an `App` is main-thread-bound by construction, because
// bevy's runner is an unconditionally non-`Send` `Box<dyn FnOnce(App) -> AppExit>`.
// No composer, and nothing built from one, may be moved off the thread that
// owns the world.
#[cfg(not(feature = "app"))]
const _: fn() = || {
    const fn assert_send<T: Send>() {}
    assert_send::<Composer>();
};

impl Composer {
    /// A composer for one host role. Under the `app` feature, hosts whose role
    /// provides `HostsWorld` own a Bevy `App`; the six App-less roles compose
    /// without one, and without the feature no role owns one at all.
    #[must_use]
    pub fn new(role: GemTargetRole) -> Self {
        Self {
            host: HostDecl::from_role(role),
            #[cfg(feature = "app")]
            app: role.hosts_world().then(App::new),
            registries: Arc::new(Registries::new()),
            contributions: Vec::new(),
            generations: HashMap::new(),
            raw_app_access: Vec::new(),
            declined_narrowings: Vec::new(),
            refusals: Vec::new(),
        }
    }

    #[must_use]
    pub const fn host(&self) -> HostDecl {
        self.host
    }

    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.registries.as_ref()
    }

    pub(crate) fn registry_lease(&self) -> Arc<Registries> {
        Arc::clone(&self.registries)
    }

    fn registries_mut(&mut self) -> &mut Registries {
        Arc::get_mut(&mut self.registries).expect(
            "a mutable composer cannot share registries; registry leases begin only after finalization",
        )
    }

    #[cfg(feature = "app")]
    #[must_use]
    pub const fn app_mut(&mut self) -> Option<&mut App> {
        self.app.as_mut()
    }

    /// Hand the composed `App` to the host's run loop.
    #[cfg(feature = "app")]
    #[must_use]
    pub const fn take_app(&mut self) -> Option<App> {
        self.app.take()
    }

    /// Claim the witness, register through the typed context, and take
    /// ownership for lifecycle and reload.
    ///
    /// `activation` is what the project selected for this instance — per-
    /// instance composition data, like the attribution the instance carries,
    /// which is why it arrives here rather than inside the contribution: the
    /// entry item is identity, the composition supplies the selection. The
    /// contribution reads it back through
    /// [`GemContext::activation`](crate::GemContext::activation). A host with
    /// nothing selected passes [`ProductActivation::default`].
    ///
    /// A containment failure is recorded as a refusal in the compose report
    /// and returned; the contribution is not composed. On the static seam
    /// this path is defence in depth behind the E0277 compile error; on the
    /// dynamic seam it is the load-time gate (ADR 0041).
    ///
    /// # Errors
    ///
    /// Returns [`Refusal`] when the host does not provide every capability
    /// the contribution declared.
    ///
    /// # Panics
    ///
    /// Panics if the registries are already shared. A `&mut Composer` proves
    /// they are not: registry leases are issued only after [`Self::finalize`]
    /// has consumed the mutable composer, so no lease can coexist with this
    /// receiver.
    pub fn add<C: Contribution>(
        &mut self,
        contribution: C,
        activation: ProductActivation,
    ) -> Result<InstanceId, Refusal> {
        let descriptor = contribution.descriptor();
        let span = tracing::info_span!(
            "compose",
            gem = %descriptor.gem,
            contribution = %descriptor.contribution,
            generation = tracing::field::Empty,
            role = %self.host.role(),
        );
        let _entered = span.enter();

        let witness = match CapabilityWitness::<C::Caps>::claim(
            descriptor.gem,
            descriptor.contribution,
            self.host,
        ) {
            Ok(witness) => witness,
            Err(refusal) => {
                tracing::warn!(
                    required = %refusal.required,
                    provided = %refusal.provided,
                    missing = %refusal.required.missing_from(refusal.provided),
                    "refused: {refusal}"
                );
                self.refusals.push(refusal.clone());
                return Err(refusal);
            }
        };

        let generation = self
            .generations
            .entry((descriptor.gem, descriptor.contribution))
            .and_modify(|generation| *generation += 1)
            .or_insert(0);
        let instance = InstanceId::new(descriptor.gem, descriptor.contribution, *generation);
        span.record("generation", instance.generation);

        let registries = Arc::get_mut(&mut self.registries).expect(
            "a mutable composer cannot share registries; registry leases begin only after finalization",
        );
        let mut ctx = GemContext::new(
            witness,
            instance,
            ContextParts {
                provided: self.host.provided(),
                role: self.host.role(),
                activation,
                #[cfg(feature = "app")]
                app: self.app.as_mut(),
                registries,
                raw_app_access: &mut self.raw_app_access,
                declined_narrowings: &mut self.declined_narrowings,
            },
        );
        contribution.register(&mut ctx);

        self.contributions.push(Composed {
            instance,
            contribution: Box::new(contribution),
        });
        tracing::info!(instance = %instance, "composed");
        Ok(instance)
    }

    /// Compose an unconditional floor contribution, if this host's role names
    /// it.
    ///
    /// The engine's own bundles arrive this way. They are not selectable, so
    /// there is nothing for a project to switch off and no activation to pass;
    /// role is the only filter, and a bundle that does not name this host's
    /// role is not this host's (asset-contract ticket 014, D6). Composing a
    /// floor is otherwise [`add`](Self::add) exactly — same witness claim, same
    /// attribution, same report line — which is the point: a floor is not a
    /// second registration path, it is the ordinary one with the selection
    /// removed.
    ///
    /// # Errors
    ///
    /// Returns [`Refusal`] when the role names the contribution but cannot
    /// provide the capability floor it declared. That is a manifest error
    /// rather than a runtime condition, and this is where it surfaces.
    pub fn floor<C: Contribution>(&mut self, contribution: C) -> Result<(), Refusal> {
        if contribution.descriptor().applies_to_role(self.host.role()) {
            self.add(contribution, ProductActivation::default())?;
        }
        Ok(())
    }

    /// Withdraw a contribution: every attributed entry across every registry
    /// in one bulk operation, plus its report lines and owned instance. The
    /// generation counter survives so a re-added instance is distinguishable.
    pub fn remove(&mut self, gem: GemId, contribution: ContributionId) -> usize {
        let removed = self.registries_mut().retain_not_owned(gem, contribution);
        let owned =
            |instance: &InstanceId| instance.gem == gem && instance.contribution == contribution;
        self.contributions
            .retain(|composed| !owned(&composed.instance));
        self.raw_app_access.retain(|instance| !owned(instance));
        self.declined_narrowings
            .retain(|declined| !owned(&declined.instance));
        self.refusals
            .retain(|refusal| refusal.gem != gem || refusal.contribution != contribution);
        tracing::info!(
            gem = %gem,
            contribution = %contribution,
            entries = removed,
            "withdrawn"
        );
        removed
    }

    /// Validate the composition and produce the operator-visible report.
    ///
    /// # Errors
    ///
    /// Returns [`ComposeError::Duplicate`] naming both culprits on a
    /// duplicate registry key, or [`ComposeError::UnresolvableClock`] when a
    /// declared clock has no composed definition.
    pub fn finalize(&self) -> Result<ComposeReport, ComposeError> {
        let role = self.host.role();
        if let Some(duplicate) = self.registries.duplicate_error() {
            tracing::error!(role = %role, "compose failed: {duplicate}");
            return Err(duplicate);
        }
        let clocks = self
            .resolve_clocks()
            .inspect_err(|error| tracing::error!(role = %role, "compose failed: {error}"))?;
        let report = ComposeReport {
            role,
            provided: self.host.provided(),
            composed: self
                .contributions
                .iter()
                .map(|composed| composed.instance)
                .collect(),
            entries: self.registries.registrations(),
            clocks,
            raw_app_access: self.raw_app_access.clone(),
            declined_narrowings: self.declined_narrowings.clone(),
            refusals: self.refusals.clone(),
        };
        tracing::info!(
            role = %report.role,
            composed = report.composed.len(),
            entries = report.entries.len(),
            clocks = report.clocks.len(),
            refusals = report.refusals.len(),
            raw = report.raw_app_access.len(),
            declined = report.declined_narrowings.len(),
            "finalized"
        );
        Ok(report)
    }

    fn resolve_clocks(&self) -> Result<Vec<ResolvedClock>, ComposeError> {
        let definitions: Vec<ResolvedClock> = self
            .registries
            .get::<ClockDefinition>()
            .map(|registry| {
                registry
                    .entries()
                    .map(|definition| ResolvedClock {
                        name: definition.name,
                        rate_hz: definition.rate_hz,
                    })
                    .collect()
            })
            .unwrap_or_default();

        if let Some(uses) = self.registries.get::<ClockUse>() {
            for attributed in uses {
                let declared = attributed.entry.clock;
                if !definitions.iter().any(|clock| clock.name == declared) {
                    return Err(ComposeError::UnresolvableClock {
                        clock: declared,
                        instance: attributed.instance,
                        role: self.host.role(),
                    });
                }
            }
        }
        Ok(definitions)
    }

    /// Horizontal lifecycle: true when every composed contribution is ready.
    #[must_use]
    pub fn all_ready(&self) -> bool {
        self.contributions
            .iter()
            .all(|composed| composed.contribution.ready())
    }

    pub fn finish(&self) {
        for composed in &self.contributions {
            composed.contribution.finish();
        }
    }

    pub fn cleanup(&self) {
        for composed in &self.contributions {
            composed.contribution.cleanup();
        }
    }
}

/// The emissions are the report's own facts, so they are asserted against a
/// capturing subscriber rather than a formatter: level, message, and fields.
#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Level, Metadata, Subscriber};

    use super::*;
    use crate::capability::Unconditional;
    use crate::descriptor::ContributionDescriptor;
    use crate::registry::RegistryEntry;

    crate::declare_caps!(BareCaps:);
    crate::declare_caps!(AuthorityCaps: HasAuthority);

    struct Component(&'static str);

    impl RegistryEntry for Component {
        type Key = &'static str;
        type Requires = Unconditional;

        fn registry_name() -> &'static str {
            "component"
        }

        fn key(&self) -> &'static str {
            self.0
        }
    }

    /// Composes into every role: two entries, no capability floor.
    struct Bare;

    impl Contribution for Bare {
        type Caps = BareCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.bare"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, BareCaps>) {
            let mut components = ctx.registrar::<Component>();
            components.register(Component("TransformComponent"));
            components.register(Component("NameComponent"));
        }
    }

    /// Refused by containment on any role without `HasAuthority`.
    struct Authority;

    impl Contribution for Authority {
        type Caps = AuthorityCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("sample.authority"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, AuthorityCaps>) {
            ctx.registrar::<Component>()
                .register(Component("MountComponent"));
        }
    }

    /// One captured event or span: what a log line or an editor row shows.
    #[derive(Clone, Debug)]
    struct Emission {
        name: &'static str,
        level: Level,
        fields: Vec<(&'static str, String)>,
    }

    impl Emission {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_str())
        }

        fn message(&self) -> &str {
            self.field("message").unwrap_or_default()
        }
    }

    struct Collect<'a>(&'a mut Vec<(&'static str, String)>);

    impl Visit for Collect<'_> {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name(), value.to_owned()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.push((field.name(), format!("{value:?}")));
        }
    }

    #[derive(Clone, Default)]
    struct Capture {
        events: Arc<Mutex<Vec<Emission>>>,
        spans: Arc<Mutex<Vec<(u64, Emission)>>>,
        next: Arc<AtomicU64>,
    }

    impl Capture {
        fn at(&self, level: Level, message: &str) -> Option<Emission> {
            let events = self.events.lock().expect("capture is not poisoned");
            events
                .iter()
                .find(|emission| emission.level == level && emission.message().starts_with(message))
                .cloned()
        }

        fn span(&self, name: &str) -> Option<Emission> {
            let spans = self.spans.lock().expect("capture is not poisoned");
            spans
                .iter()
                .find(|(_, emission)| emission.name == name)
                .map(|(_, emission)| emission.clone())
        }
    }

    impl Subscriber for Capture {
        /// Consult `enabled` per event instead of caching a verdict, so a
        /// callsite registered by one test's subscriber still reaches the
        /// next one.
        fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
            Interest::sometimes()
        }

        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, attributes: &Attributes<'_>) -> Id {
            let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            let mut fields = Vec::new();
            attributes.record(&mut Collect(&mut fields));
            self.spans.lock().expect("capture is not poisoned").push((
                id,
                Emission {
                    name: attributes.metadata().name(),
                    level: *attributes.metadata().level(),
                    fields,
                },
            ));
            Id::from_u64(id)
        }

        fn record(&self, id: &Id, values: &Record<'_>) {
            let mut spans = self.spans.lock().expect("capture is not poisoned");
            if let Some((_, emission)) = spans
                .iter_mut()
                .find(|(stored, _)| *stored == id.into_u64())
            {
                values.record(&mut Collect(&mut emission.fields));
            }
        }

        fn record_follows_from(&self, _id: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = Vec::new();
            event.record(&mut Collect(&mut fields));
            self.events
                .lock()
                .expect("capture is not poisoned")
                .push(Emission {
                    name: event.metadata().name(),
                    level: *event.metadata().level(),
                    fields,
                });
        }

        fn enter(&self, _id: &Id) {}

        fn exit(&self, _id: &Id) {}
    }

    fn capture(compose: impl FnOnce()) -> Capture {
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), compose);
        capture
    }

    #[test]
    fn a_refusal_warns_with_the_gem_and_the_missing_axis() {
        let log = capture(|| {
            let mut composer = Composer::new(GemTargetRole::AssetWorker);
            let _ = composer.add(Authority, ProductActivation::default());
        });

        let warning = log.at(Level::WARN, "refused").expect("a refusal is warned");
        assert!(
            warning.message().contains("sample.authority"),
            "{warning:?}"
        );
        assert_eq!(warning.field("missing"), Some("{HasAuthority}"));
        assert_eq!(warning.field("provided"), Some("{}"));
    }

    #[test]
    fn every_entry_traces_its_registry_and_key() {
        let log = capture(|| {
            let mut composer = Composer::new(GemTargetRole::Server);
            composer
                .add(Bare, ProductActivation::default())
                .expect("bare requires nothing");
        });

        let entry = log
            .at(Level::TRACE, "registered")
            .expect("each entry is traced");
        assert_eq!(entry.field("registry"), Some("component"));
        assert_eq!(entry.field("key"), Some("TransformComponent"));
        assert_eq!(entry.field("instance"), Some("azoth.bare/runtime#0"));
    }

    #[test]
    fn finalize_summarizes_the_composition_at_info() {
        let log = capture(|| {
            let mut composer = Composer::new(GemTargetRole::Server);
            composer
                .add(Bare, ProductActivation::default())
                .expect("bare requires nothing");
            composer.finalize().expect("composition is valid");
        });

        let summary = log
            .at(Level::INFO, "finalized")
            .expect("finalize summarizes");
        assert_eq!(summary.field("role"), Some("server"));
        assert_eq!(summary.field("composed"), Some("1"));
        assert_eq!(summary.field("entries"), Some("2"));
        assert_eq!(summary.field("refusals"), Some("0"));
    }

    #[test]
    fn the_compose_span_carries_identity_and_generation() {
        let log = capture(|| {
            let mut composer = Composer::new(GemTargetRole::Server);
            composer
                .add(Bare, ProductActivation::default())
                .expect("bare requires nothing");
        });

        let span = log.span("compose").expect("add opens a compose span");
        assert_eq!(span.field("gem"), Some("azoth.bare"));
        assert_eq!(span.field("contribution"), Some("runtime"));
        assert_eq!(span.field("role"), Some("server"));
        assert_eq!(span.field("generation"), Some("0"));
    }
}
