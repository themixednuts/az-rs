//! Contract registration for Bevy-native Prefab types.
//!
//! A gem's authored component types reach Prefab processing as ordinary
//! composed registry entries: one [`PrefabType`] per reflected type, keyed on
//! that type's path and attributed to the contributing instance by the
//! composer. Reflection registration is therefore shaped like every other
//! registrar — `ctx.registrar::<PrefabType>()` — instead of a hidden
//! `fn(&mut TypeRegistry)` hook that only survives when the linker keeps the
//! contributing crate.
//!
//! Two contributions claiming one type path fail composition, naming both,
//! rather than racing for last-writer-wins inside a `TypeRegistry`.

use az_gem_contract::{RegistryEntry, Unconditional};
use bevy_reflect::{GetTypeRegistration, TypePath, TypeRegistry};

/// One reflected Prefab type contributed to a host's processing registry.
#[derive(Clone, Copy, Debug)]
pub struct PrefabType {
    path: &'static str,
    apply: fn(&mut TypeRegistry),
}

impl PrefabType {
    /// The entry for `T`.
    ///
    /// Applying it registers `T` and the types `T` reflects through, exactly
    /// as `TypeRegistry::register::<T>` does — the reflected dependency
    /// closure is not something a contribution has to enumerate.
    #[must_use]
    pub fn of<T: GetTypeRegistration + TypePath>() -> Self {
        Self {
            path: T::type_path(),
            apply: TypeRegistry::register::<T>,
        }
    }

    /// The entry for a type whose registration needs more than
    /// [`TypeRegistry::register`] — non-derive `TypeData` such as
    /// `ReflectSerialize` on `Arc<T>`, or editor policy callbacks.
    ///
    /// The hook stays a per-type composed entry: `path` is still the
    /// duplicate key, attribution still lands on the contributing instance,
    /// and two contributions claiming one path still fail composition. The
    /// hook is the sanctioned home for registration `of` cannot express,
    /// not a bulk side door — register one type (and its data) per entry.
    #[must_use]
    pub const fn hook(path: &'static str, apply: fn(&mut TypeRegistry)) -> Self {
        Self { path, apply }
    }

    /// Fully qualified reflected type path. This is the duplicate key.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Add this type, and the types it reflects through, to `registry`.
    pub fn apply(&self, registry: &mut TypeRegistry) {
        (self.apply)(registry);
    }
}

impl RegistryEntry for PrefabType {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "prefab-type"
    }

    fn key(&self) -> Self::Key {
        self.path
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::Reflect;

    use super::*;

    #[derive(Reflect)]
    struct Leaf {
        value: u32,
    }

    #[derive(Reflect)]
    struct Branch {
        leaf: Leaf,
    }

    #[test]
    fn the_key_is_the_reflected_type_path() {
        let entry = PrefabType::of::<Branch>();
        assert_eq!(entry.key(), Branch::type_path());
        assert_eq!(entry.path(), entry.key());
        assert_eq!(PrefabType::registry_name(), "prefab-type");
    }

    #[test]
    fn a_hook_entry_carries_registration_of_cannot_express() {
        fn arc_leaf(registry: &mut TypeRegistry) {
            registry.register::<Leaf>();
            registry.register_type_data::<Leaf, bevy_reflect::ReflectFromPtr>();
        }

        let entry = PrefabType::hook(Leaf::type_path(), arc_leaf);
        assert_eq!(entry.key(), Leaf::type_path());

        let mut registry = TypeRegistry::empty();
        entry.apply(&mut registry);
        let registration = registry.get(std::any::TypeId::of::<Leaf>()).unwrap();
        assert!(
            registration
                .data::<bevy_reflect::ReflectFromPtr>()
                .is_some()
        );
    }

    #[test]
    fn applying_an_entry_registers_its_reflected_dependencies() {
        let mut registry = TypeRegistry::empty();
        PrefabType::of::<Branch>().apply(&mut registry);

        assert!(registry.get(std::any::TypeId::of::<Branch>()).is_some());
        assert!(
            registry.get(std::any::TypeId::of::<Leaf>()).is_some(),
            "a contribution registers one type, not its whole field closure"
        );
    }
}
