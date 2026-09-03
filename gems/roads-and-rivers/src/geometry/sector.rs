//! Spline strip sector mesh generation.

mod builder;
mod core;
mod mesh;
mod polyline;

pub use builder::build_spline_geometry_sectors;
pub use core::SplineGeometrySector;
pub use mesh::no_degenerate_triangles;
pub use mesh::spline_geometry_sectors_to_mesh;
