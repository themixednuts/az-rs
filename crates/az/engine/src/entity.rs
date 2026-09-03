//! Entity components - domain-specific functionality
//!
//! This module provides CryEngine/Lumberyard-inspired entity functionality that
//! complements Bevy's built-in ECS. For basic entity naming, use `bevy::prelude::Name`.
//! For lifecycle events, use Bevy's `Observer` system with `Add`, `Remove`, etc.
//!
//! # Examples
//!
//! Basic entity spawning with Bevy's Name:
//! ```rust,no_run
//! use bevy::prelude::*;
//! use az_engine::prelude::*;
//!
//! fn spawn_entity(mut commands: Commands) {
//!     commands.spawn((
//!         Name::new("MyEntity"),
//!         EntityFlags { no_save: false },
//!     ));
//! }
//! ```
//!
//! Using observers for lifecycle events:
//! ```rust,no_run
//! use bevy::prelude::*;
//!
//! fn setup(mut commands: Commands) {
//!     // Observe entity spawns with Name component
//!     commands.add_observer(|add: On<Add, Name>| {
//!         println!("Entity spawned: {:?}", add.entity);
//!     });
//! }
//! ```

use bevy::prelude::*;

/// Entity save/load flags inspired by `CryEngine`
///
/// Stores domain-specific metadata that doesn't have direct Bevy equivalents.
/// Most entity behavior should use Bevy's built-in components:
/// - For visibility: Use `Visibility` component
/// - For shadows: Use `bevy_pbr::NotShadowCaster` / `NotShadowReceiver`
/// - For active/inactive state: Use entity enable/disable or custom marker components
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct EntityFlags {
    /// Entity should not be saved to disk
    ///
    /// Set to `true` for temporary/runtime entities like debug visualizations,
    /// particle effects, or procedurally generated helpers that should be
    /// recreated on load rather than serialized.
    pub no_save: bool,
}

impl EntityFlags {
    /// Create default flags (entity will be saved)
    #[must_use]
    pub const fn new() -> Self {
        Self { no_save: false }
    }

    /// Create flags for a temporary entity (not saved)
    ///
    /// Use for debug helpers, runtime effects, or anything that should
    /// be recreated procedurally rather than loaded from disk.
    #[must_use]
    pub const fn temporary() -> Self {
        Self { no_save: true }
    }
}

impl Default for EntityFlags {
    fn default() -> Self {
        Self::new()
    }
}

/// Prelude for entity types
pub mod prelude {
    pub use super::EntityFlags;

    // Re-export Bevy's Name for convenience
    // Use this instead of a custom EntityName component
    pub use bevy::prelude::Name;
}

/// Plugin to register entity components with reflection
pub struct EntityPlugin;

impl Plugin for EntityPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<EntityFlags>();
    }
}
