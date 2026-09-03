//! Engine-side core utilities.
//!
//! AZ math/geometry/color types come straight from Bevy where possible
//! (`bevy::math::bounding::Aabb3d`, `bevy::color::Color`, etc.) and from
//! `az-core` for the AZ-only surface (`AzName`, `Crc32`). This module
//! holds engine-specific glue: the engine error type and Bevy-bound
//! math/time re-exports.

pub mod math;
pub mod time;
pub mod types;

pub mod prelude {
    //! Engine-side prelude.
    pub use super::types::*;
}
