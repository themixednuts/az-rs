//! Residual Spawnable-to-Bevy lowering registrations.
//!
//! This registry is intentionally limited to the legacy Spawnable runtime
//! path. Authoring metadata, applicability, validation, and editor projection
//! live on the normal Bevy `TypeRegistry` and must not be added here.

use crate::rtti::{AzTypeKind, AzTypeRegistration};
#[cfg(feature = "bevy")]
use crate::rtti::{BevyComponentRegistration, BevyComponentValue, BevyComponentValueError};
use crate::{AzComponent, AzTypeInfo};

/// Products in which a residual Spawnable lowering adapter is available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComponentExportPolicy {
    #[default]
    RuntimeAndEditor,
    RuntimeOnly,
    EditorOnly,
    Excluded,
}

impl ComponentExportPolicy {
    #[must_use]
    pub const fn editor_available(self) -> bool {
        matches!(self, Self::RuntimeAndEditor | Self::EditorOnly)
    }

    #[must_use]
    pub const fn runtime_available(self) -> bool {
        matches!(self, Self::RuntimeAndEditor | Self::RuntimeOnly)
    }
}

/// Composed adapter used only by the residual Spawnable entity lowerer.
#[derive(Debug, Clone, Copy)]
pub struct ComponentLoweringRegistration {
    pub type_registration: AzTypeRegistration,
    pub export_policy: ComponentExportPolicy,
    #[cfg(feature = "bevy")]
    pub bevy_component: Option<BevyComponentRegistration>,
}

impl ComponentLoweringRegistration {
    /// Selects the lowering metadata emitted by `#[derive(AzComponent)]`.
    ///
    /// This decision belongs to `az-core`, where the `bevy` feature is
    /// defined. Proc-macro output must not test a feature on the consumer
    /// crate, because that can silently choose different registrations for
    /// the same component type.
    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn derived_component<T>() -> Self
    where
        T: AzComponent + bevy_ecs::component::Component + Default + 'static,
    {
        Self::bevy_component::<T>()
    }

    /// Registers derived AZ component identity when Bevy lowering is not part
    /// of the selected `az-core` feature set.
    #[cfg(not(feature = "bevy"))]
    #[must_use]
    pub const fn derived_component<T>() -> Self
    where
        T: AzComponent + 'static,
    {
        Self::component::<T>()
    }

    #[must_use]
    pub const fn component<T>() -> Self
    where
        T: AzComponent + 'static,
    {
        Self::from_type::<T>(T::BASE_TYPE_IDS, ComponentExportPolicy::RuntimeAndEditor)
    }

    /// # Panics
    ///
    /// If [`Self::mapped_bevy_component`] stopped installing Bevy metadata,
    /// which would leave this registration with no component to insert.
    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn bevy_component<T>() -> Self
    where
        T: AzComponent + bevy_ecs::component::Component + Default + 'static,
    {
        let mut registration = Self::mapped_bevy_component::<T, T>();
        let Some(mut bevy_component) = registration.bevy_component else {
            panic!("direct Bevy component lowering must install Bevy metadata");
        };
        bevy_component.component_id = Some(component_id::<T>);
        registration.bevy_component = Some(bevy_component);
        registration
    }

    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn mapped_bevy_component<T, B>() -> Self
    where
        T: AzComponent + 'static,
        B: bevy_ecs::component::Component + Default + 'static,
    {
        let mut registration = Self::component::<T>();
        registration.bevy_component = Some(BevyComponentRegistration {
            insert_default: insert_default_component::<B>,
            component_id: None,
            apply_values: None,
            finalize_entity_table: None,
        });
        registration
    }

    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn mapped_bevy_component_with_values<T, B>(
        apply_values: fn(
            &mut bevy_ecs::system::EntityCommands<'_>,
            &dyn BevyComponentValue,
        ) -> Result<(), BevyComponentValueError>,
    ) -> Self
    where
        T: AzComponent + 'static,
        B: bevy_ecs::component::Component + Default + 'static,
    {
        let mut registration = Self::component::<T>();
        registration.bevy_component = Some(BevyComponentRegistration {
            insert_default: insert_default_component::<B>,
            component_id: None,
            apply_values: Some(apply_values),
            finalize_entity_table: None,
        });
        registration
    }

    /// # Panics
    ///
    /// If this registration has no mapped Bevy component; an entity-table
    /// finalizer has nothing to run against without one.
    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn with_bevy_entity_table_finalizer(
        mut self,
        finalize: fn(&mut bevy_ecs::system::Commands<'_, '_>, &[bevy_ecs::entity::Entity]),
    ) -> Self {
        let Some(mut bevy_component) = self.bevy_component else {
            panic!("a Bevy entity-table finalizer requires a mapped Bevy component");
        };
        bevy_component.finalize_entity_table = Some(finalize);
        self.bevy_component = Some(bevy_component);
        self
    }

    #[must_use]
    pub const fn component_shell<T>() -> Self
    where
        T: AzTypeInfo + 'static,
    {
        Self::from_type::<T>(&[], ComponentExportPolicy::EditorOnly)
    }

    #[cfg(feature = "bevy")]
    #[must_use]
    pub const fn mapped_shell_with_values<T, B>(
        apply_values: fn(
            &mut bevy_ecs::system::EntityCommands<'_>,
            &dyn BevyComponentValue,
        ) -> Result<(), BevyComponentValueError>,
    ) -> Self
    where
        T: AzTypeInfo + 'static,
        B: bevy_ecs::component::Component + Default + 'static,
    {
        let mut registration = Self::from_type::<T>(&[], ComponentExportPolicy::RuntimeAndEditor);
        registration.bevy_component = Some(BevyComponentRegistration {
            insert_default: insert_default_component::<B>,
            component_id: None,
            apply_values: Some(apply_values),
            finalize_entity_table: None,
        });
        registration
    }

    #[must_use]
    pub const fn with_export_policy(mut self, export_policy: ComponentExportPolicy) -> Self {
        self.export_policy = export_policy;
        self
    }

    const fn from_type<T>(
        base_type_ids: &'static [uuid::Uuid],
        export_policy: ComponentExportPolicy,
    ) -> Self
    where
        T: AzTypeInfo + 'static,
    {
        Self {
            type_registration: AzTypeRegistration {
                name: T::NAME,
                native_type_id: T::TYPE_ID,
                base_type_ids,
                rust_type_id: rust_type_id::<T>,
                kind: AzTypeKind::Component,
            },
            export_policy,
            #[cfg(feature = "bevy")]
            bevy_component: None,
        }
    }
}

const fn rust_type_id<T: 'static>() -> std::any::TypeId {
    std::any::TypeId::of::<T>()
}

#[cfg(feature = "bevy")]
fn insert_default_component<T>(entity: &mut bevy_ecs::system::EntityCommands<'_>)
where
    T: bevy_ecs::component::Component + Default,
{
    entity.insert(T::default());
}

#[cfg(feature = "bevy")]
fn component_id<T>(
    world: &bevy_ecs::world::World,
    entity: bevy_ecs::entity::Entity,
) -> Option<crate::component::ComponentId>
where
    T: AzComponent + bevy_ecs::component::Component,
{
    world
        .get::<T>(entity)
        .map(AzComponent::component_id)
        .filter(|id| id.is_valid())
}

/// One lowering per AZ type id, matching the type registration it carries. A
/// second adapter for the same native component is two answers to "how do these
/// bytes become an entity", which the residual lowerer resolved by whichever
/// crate the linker reached first.
impl az_gem_contract::RegistryEntry for ComponentLoweringRegistration {
    type Key = uuid::Uuid;
    type Requires = az_gem_contract::Unconditional;

    fn registry_name() -> &'static str {
        "component-lowering"
    }

    fn key(&self) -> Self::Key {
        self.type_registration.native_type_id
    }
}

/// Every component lowering a host composed, in composition order.
///
/// `az-core` contributes none of its own: it defines the adapter shape, and the
/// crates that own components register theirs.
#[must_use]
pub fn composed(registries: &az_gem_contract::Registries) -> Vec<ComponentLoweringRegistration> {
    registries
        .get::<ComponentLoweringRegistration>()
        .into_iter()
        .flat_map(az_gem_contract::Registry::entries)
        .copied()
        .collect()
}
