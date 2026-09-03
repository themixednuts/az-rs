//! Primitive shape component data.

mod box_shape;
mod capsule;
mod cylinder;
mod ids;
mod sphere;

pub use box_shape::{BoxShapeComponent, BoxShapeConfig};
pub use capsule::{CapsuleShapeComponent, CapsuleShapeConfig};
pub use cylinder::{CylinderShapeComponent, CylinderShapeConfig};
pub use ids::*;
pub use sphere::{SphereShapeComponent, SphereShapeConfig};
