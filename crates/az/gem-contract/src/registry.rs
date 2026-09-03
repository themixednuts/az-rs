use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use crate::capability::Requirement;
use crate::error::ComposeError;
use crate::ids::{ContributionId, GemId};
use crate::instance::InstanceId;
use crate::report::Registration;

/// One registered entry plus the contributing instance that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributed<T> {
    pub instance: InstanceId,
    pub entry: T,
}

/// A domain registry entry type.
///
/// Names its registry, its duplicate key, and the host-capability floor
/// required to register into it.
///
/// `Send + Sync` is part of the contract, not an incidental bound: a composed
/// [`Registries`] is a plain description of what a host registered, and hosts
/// hand it to worker threads — the asset worker runs `create_jobs` /
/// `process_job` under `spawn_blocking` so a lease heartbeat keeps ticking
/// through a long build, and the job context carries `&Registries` across that
/// boundary. Entries are descriptors — ids, `&'static str`, plain data, `fn`
/// pointers — so this costs an author nothing and keeps the failure at the
/// `impl` site instead of surfacing as an auto-trait leak at some distant
/// `spawn_blocking` call.
pub trait RegistryEntry: 'static + Send + Sync {
    type Key: Clone + Eq + Hash + fmt::Display;
    type Requires: Requirement;

    fn registry_name() -> &'static str;
    fn key(&self) -> Self::Key;
}

/// Owned, attributed entries for one registry family.
///
/// Iteration order is registration (closure) order.
pub struct Registry<T: RegistryEntry> {
    entries: Vec<Attributed<T>>,
}

impl<T: RegistryEntry> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: RegistryEntry> Registry<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Attributed<T>> {
        self.entries.iter()
    }

    pub fn entries(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|attributed| &attributed.entry)
    }

    pub(crate) const fn registrar(&mut self, instance: InstanceId) -> Registrar<'_, T> {
        Registrar {
            instance,
            entries: &mut self.entries,
        }
    }

    fn retain_not_owned(&mut self, gem: GemId, contribution: ContributionId) -> usize {
        let before = self.entries.len();
        self.entries.retain(|attributed| {
            attributed.instance.gem != gem || attributed.instance.contribution != contribution
        });
        before - self.entries.len()
    }

    fn duplicate_error(&self) -> Option<ComposeError> {
        let mut seen: HashMap<T::Key, InstanceId> = HashMap::with_capacity(self.entries.len());
        for attributed in &self.entries {
            if let Some(first) = seen.insert(attributed.entry.key(), attributed.instance) {
                return Some(ComposeError::Duplicate {
                    registry: T::registry_name(),
                    key: attributed.entry.key().to_string(),
                    first,
                    second: attributed.instance,
                });
            }
        }
        None
    }

    fn append_registrations(&self, into: &mut Vec<Registration>) {
        into.extend(self.entries.iter().map(|attributed| Registration {
            instance: attributed.instance,
            registry: T::registry_name(),
            key: attributed.entry.key().to_string(),
        }));
    }
}

impl<'a, T: RegistryEntry> IntoIterator for &'a Registry<T> {
    type Item = &'a Attributed<T>;
    type IntoIter = std::slice::Iter<'a, Attributed<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Attribution-capturing write handle for one registry family.
///
/// Constructed only by the composer path; an author never names an owner.
pub struct Registrar<'a, T: RegistryEntry> {
    instance: InstanceId,
    entries: &'a mut Vec<Attributed<T>>,
}

impl<T: RegistryEntry> Registrar<'_, T> {
    pub fn register(&mut self, entry: T) {
        // `key()` and its `Display` stay inside the macro: an entry costs
        // nothing to register when the `trace` level is off.
        tracing::trace!(
            registry = T::registry_name(),
            key = %entry.key(),
            instance = %self.instance,
            "registered"
        );
        self.entries.push(Attributed {
            instance: self.instance,
            entry,
        });
    }

    pub fn register_many(&mut self, entries: impl IntoIterator<Item = T>) {
        for entry in entries {
            self.register(entry);
        }
    }
}

/// Object-safe view over one `Registry<T>`.
///
/// `Any` is a supertrait so the heterogeneous store can recover the concrete
/// type by dyn upcasting; `Send + Sync` carries [`RegistryEntry`]'s thread
/// bound through the erasure so `Registries` itself is `Send + Sync`.
trait AnyRegistry: Any + Send + Sync {
    fn registry_name(&self) -> &'static str;
    fn retain_not_owned(&mut self, gem: GemId, contribution: ContributionId) -> usize;
    fn duplicate_error(&self) -> Option<ComposeError>;
    fn append_registrations(&self, into: &mut Vec<Registration>);
}

impl<T: RegistryEntry> AnyRegistry for Registry<T> {
    fn registry_name(&self) -> &'static str {
        T::registry_name()
    }

    fn retain_not_owned(&mut self, gem: GemId, contribution: ContributionId) -> usize {
        self.retain_not_owned(gem, contribution)
    }

    fn duplicate_error(&self) -> Option<ComposeError> {
        self.duplicate_error()
    }

    fn append_registrations(&self, into: &mut Vec<Registration>) {
        self.append_registrations(into);
    }
}

/// The owned per-host registry set (ADR 0033).
///
/// One value per composed host; no process-global registry state exists.
#[derive(Default)]
pub struct Registries {
    stores: HashMap<TypeId, Box<dyn AnyRegistry>>,
}

impl Registries {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get<T: RegistryEntry>(&self) -> Option<&Registry<T>> {
        let store = self.stores.get(&TypeId::of::<Registry<T>>())?;
        let any: &dyn Any = store.as_ref();
        any.downcast_ref::<Registry<T>>()
    }

    /// # Panics
    ///
    /// Panics if the store at this `TypeId` is not a `Registry<T>`. The map is
    /// keyed by `TypeId::of::<Registry<T>>()`, so this is a programming bug.
    pub(crate) fn get_mut<T: RegistryEntry>(&mut self) -> &mut Registry<T> {
        let store = self
            .stores
            .entry(TypeId::of::<Registry<T>>())
            .or_insert_with(|| Box::new(Registry::<T>::new()));
        let any: &mut dyn Any = store.as_mut();
        any.downcast_mut::<Registry<T>>()
            .expect("store is keyed by the TypeId of Registry<T>")
    }

    /// Withdraw every entry owned by `(gem, contribution)` across all
    /// registries. Returns the number of entries removed.
    pub fn retain_not_owned(&mut self, gem: GemId, contribution: ContributionId) -> usize {
        self.stores
            .values_mut()
            .map(|store| store.retain_not_owned(gem, contribution))
            .sum()
    }

    pub(crate) fn duplicate_error(&self) -> Option<ComposeError> {
        self.sorted()
            .into_iter()
            .find_map(AnyRegistry::duplicate_error)
    }

    pub(crate) fn registrations(&self) -> Vec<Registration> {
        let mut registrations = Vec::new();
        for store in self.sorted() {
            store.append_registrations(&mut registrations);
        }
        registrations
    }

    /// Registry-name order, so reports and duplicate diagnostics do not
    /// inherit the store map's hash order.
    fn sorted(&self) -> Vec<&dyn AnyRegistry> {
        let mut stores: Vec<&dyn AnyRegistry> = self.stores.values().map(AsRef::as_ref).collect();
        stores.sort_by_key(|store| store.registry_name());
        stores
    }
}

/// Hosts hand a composed registry set to worker threads, so this is load
/// bearing, not incidental. Pinned here because the way it would break is a
/// future entry type quietly holding an `Rc` or a bare `dyn Fn`: the erased
/// store would stop being `Send + Sync` and the failure would surface far away,
/// at whatever `spawn_blocking` call moved the composition across a thread.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Registries>();
};
