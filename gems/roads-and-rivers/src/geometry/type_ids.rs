//! Roads and rivers geometry reflected type identifiers.

use uuid::Uuid;

/// Lumberyard `RoadsAndRivers::SplineGeometry` type UUID.
pub const SPLINE_GEOMETRY_TYPE_ID: &str = "1E31B92F-5188-4074-8F71-810A3B59CC6B";
pub const SPLINE_GEOMETRY_TYPE_UUID: Uuid = Uuid::from_u128(0x1E31B92F_5188_4074_8F71_810A3B59CC6B);

/// Lumberyard `RoadsAndRivers::SplineGeometryWidthModifier` type UUID.
pub const SPLINE_GEOMETRY_WIDTH_MODIFIER_TYPE_ID: &str = "F69CC9C6-5B29-4C17-8028-3167165F9EC7";
pub const SPLINE_GEOMETRY_WIDTH_MODIFIER_TYPE_UUID: Uuid =
    Uuid::from_u128(0xF69CC9C6_5B29_4C17_8028_3167165F9EC7);
