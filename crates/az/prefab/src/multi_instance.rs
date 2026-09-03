//! Engine-owned marker for multi-instance component overflow.
//!
//! Native Lumberyard entities may carry several instances of the same component
//! class on one `AZ::Entity`. Bevy allows only one component of type `T` per
//! entity, so the importer keeps instance 0 on the source entity and overflows
//! instances 1..N onto synthetic child entities. Each such child carries
//! `ChildOf(parent)`, the full component set for that instance, and this
//! [`MultiInstanceOf`] marker recording which component the child is an extra
//! instance of. The mechanism is game-agnostic: "overflow instance of a
//! component onto a child entity" is a generic prefab shape, so the marker lives
//! in the engine rather than any single game.

use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, std_traits::ReflectDefault};

use crate::{Prefab, ReflectPrefab};

/// Stable authored schema tag for the multi-instance overflow marker.
pub const MULTI_INSTANCE_OF_TAG: &str = "azoth.prefab.MultiInstanceOf";

/// Marks a synthetic child entity as carrying an overflow instance of a
/// multi-instance component that lives on its parent.
#[derive(Component, Reflect, Clone, Default, PartialEq, Eq, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "azoth.prefab.MultiInstanceOf", version = 1)]
pub struct MultiInstanceOf {
    /// Canonical Prefab tag of the component this synthetic entity carries an
    /// extra instance of.
    pub component: String,
    /// Source-order index of this instance among its siblings (>= 1 for children).
    pub index: u32,
    /// Native Lumberyard `ComponentId`, retained for fidelity/debug. Not used as an
    /// address (proven unreferenced in the corpus).
    pub component_id: u64,
}

/// Registers engine-core Prefab component types in a caller-owned registry.
///
/// Runtime roles, the builder, and the project host all compose the same
/// registration so cooked scenes can instantiate these engine types uniformly.
pub fn register_core_prefab_types(registry: &mut TypeRegistry) {
    registry.register::<MultiInstanceOf>();
}
