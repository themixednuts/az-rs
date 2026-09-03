//! Core type definitions
//!
//! Basic types used throughout the engine.

/// Result type for engine operations
pub type EngineResult<T> = Result<T, EngineError>;

/// Common engine error type
///
/// All engine operations that can fail return this error type.
/// Use the `?` operator for ergonomic error handling.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Engine initialization failed: {message}")]
    Initialization { message: String },

    #[error("System operation failed: {message}")]
    System { message: String },

    #[error("Asset operation failed: {message}")]
    Asset { message: String },

    #[error("Network operation failed: {message}")]
    Network { message: String },

    #[error("Module operation failed: {message}")]
    Module { message: String },

    #[error("Entity operation failed: {message}")]
    Entity { message: String },

    #[error("Component operation failed: {message}")]
    Component { message: String },

    #[error("Geometry operation failed: {message}")]
    Geometry { message: String },

    #[error("Terrain operation failed: {message}")]
    Terrain { message: String },
}

// Note: EngineState removed - use Bevy's States/SubStates instead
// See: https://docs.rs/bevy/latest/bevy/state/index.html
//
// Example:
// #[derive(States, Debug, Clone, PartialEq, Eq, Hash)]
// pub enum AppState {
//     Loading,
//     Running,
//     Paused,
// }

// Note: Priority removed - use Bevy's system ordering instead
// See: https://docs.rs/bevy/latest/bevy/ecs/schedule/trait.IntoSystemConfigs.html
//
// Example:
// app.add_systems(Update, (
//     high_priority_system.before(normal_system),
//     normal_system,
//     low_priority_system.after(normal_system),
// ));
