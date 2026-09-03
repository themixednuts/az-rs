//! Native AZ entity-component surface (`AZ_COMPONENT`).

mod identity;
#[cfg(feature = "bevy")]
mod index;
pub mod lowering;

use bevy_reflect::Reflect;

use crate::rtti::AzRtti;

#[cfg(feature = "bevy")]
use bevy_app::{App, Plugin};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use identity::{ComponentId, EntityId};
#[cfg(feature = "bevy")]
pub use index::{AzEntityIndex, AzEntityIndexPlugin};
pub use lowering::ComponentLoweringRegistration;

/// Native `AZ::Component` RTTI type id.
pub const COMPONENT_TYPE_ID: uuid::Uuid = uuid::uuid!("EDFCB2CF-F75D-43BE-B26B-F35821B29247");

/// Native `ComponentConfig` RTTI type id.
pub const COMPONENT_CONFIG_TYPE_ID: uuid::Uuid =
    uuid::uuid!("0A7929DF-2932-40EA-B2B3-79BC1C3490D0");

/// Marker base used by reflected component configuration objects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, az_derive::AzRtti)]
#[az_rtti(name = "ComponentConfig", COMPONENT_CONFIG_TYPE_ID, register)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ComponentConfig;

/// Serialized `AZ::Component` base payload.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect, az_derive::AzRtti,
)]
#[az_rtti(name = "AZ::Component", COMPONENT_TYPE_ID, register)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Component {
    #[cfg_attr(feature = "serde", serde(rename = "Id", default))]
    pub id: ComponentId,
}

/// Native AZ entity-component binding (`AZ_COMPONENT`).
///
/// In Lumberyard/O3DE this expands to `AZ_RTTI(..., AZ::Component)` plus a
/// component descriptor. In Azoth it requires [`AzRtti`] and registers into
/// the linked AZ type registry, where components are a filtered subset of all
/// registered AZ types. Runtime adapters place any construction requirements
/// on their concrete representation; authored component identity itself does
/// not require a synthetic [`Default`] value.
pub trait AzComponent: AzRtti {
    /// Serialized native component identity used by entity-local and
    /// Manifold Binding facet routes.
    ///
    /// Components whose authoring shape does not carry an `AZ::Component`
    /// base return the invalid id. Cookers omit invalid ids from target
    /// tables instead of manufacturing runtime identities.
    #[must_use]
    fn component_id(&self) -> ComponentId {
        ComponentId::INVALID
    }
}

impl AzComponent for Component {
    #[inline]
    fn component_id(&self) -> ComponentId {
        self.id
    }
}

/// Register core AZ entity/component identity types with Bevy reflection.
#[cfg(feature = "bevy")]
pub struct AzEntityComponentIdentityPlugin;

#[cfg(feature = "bevy")]
impl Plugin for AzEntityComponentIdentityPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Component>()
            .register_type::<ComponentConfig>()
            .register_type::<EntityId>()
            .register_type::<ComponentId>()
            .register_type::<Vec<ComponentId>>();
    }

    fn is_unique(&self) -> bool {
        false
    }
}

#[cfg(all(test, feature = "bevy"))]
mod tests {
    use super::*;
    use bevy_ecs::system::Commands;
    use bevy_ecs::world::{CommandQueue, World};

    #[derive(az_derive::AzTypeInfo)]
    #[az_type_info("22222222-2222-2222-2222-222222222222")]
    struct TestNativeBase;

    #[derive(bevy_ecs::component::Component, az_derive::AzComponent, Default)]
    #[az_component("11111111-1111-1111-1111-111111111111", TestNativeBase)]
    struct TestRegisteredComponent;

    az_gem_contract::declare_caps!(ComponentCaps:);

    /// Stands in for the contribution a component-owning crate writes: its
    /// lowerings, listed by the crate that defines the components.
    struct Components;

    impl az_gem_contract::Contribution for Components {
        type Caps = ComponentCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.core-test"),
                contribution: az_gem_contract::ContributionId::new("components"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, ComponentCaps>) {
            ctx.registrar::<lowering::ComponentLoweringRegistration>()
                .register(TestRegisteredComponent::REGISTRATION);
        }
    }

    fn composed_components() -> az_gem_contract::Composer {
        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
        composer
            .add(Components, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    /// A composed lowering is also an AZ type: `rtti::composed` chains the type
    /// half of every lowering, the way the linked iterator chained the two
    /// inventories.
    #[test]
    fn a_composed_lowering_is_chained_into_the_composed_types() {
        let composer = composed_components();

        assert!(
            crate::rtti::composed(composer.registries())
                .iter()
                .any(|registration| {
                    registration.name == "TestRegisteredComponent" && registration.is_component()
                })
        );
    }

    #[test]
    fn az_component_derive_registers_default_bevy_inserter() {
        let composer = composed_components();
        let registration = lowering::composed(composer.registries())
            .into_iter()
            .find(|registration| registration.type_registration.name == "TestRegisteredComponent")
            .expect("derived test component should be registered");

        assert_eq!(
            registration.type_registration.native_type_id,
            uuid::uuid!("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(
            (registration.type_registration.rust_type_id)(),
            std::any::TypeId::of::<TestRegisteredComponent>()
        );
        let direct_registration =
            lowering::ComponentLoweringRegistration::bevy_component::<TestRegisteredComponent>();
        assert_eq!(
            direct_registration.type_registration.name,
            "TestRegisteredComponent"
        );
        assert_eq!(
            direct_registration.type_registration.native_type_id,
            registration.type_registration.native_type_id
        );
        assert_eq!(
            (direct_registration.type_registration.rust_type_id)(),
            std::any::TypeId::of::<TestRegisteredComponent>()
        );
        assert_eq!(
            <TestRegisteredComponent as AzRtti>::BASE_TYPE_IDS,
            &[
                <TestNativeBase as crate::type_info::AzTypeInfo>::TYPE_ID,
                COMPONENT_TYPE_ID
            ]
        );
        let bevy_component = registration
            .bevy_component
            .expect("registered Bevy component should include an inserter");

        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let entity = {
            let mut commands = Commands::new(&mut queue, &world);
            let mut entity = commands.spawn_empty();
            (bevy_component.insert_default)(&mut entity);
            entity.id()
        };

        queue.apply(&mut world);
        assert!(world.get::<TestRegisteredComponent>(entity).is_some());
    }
}
