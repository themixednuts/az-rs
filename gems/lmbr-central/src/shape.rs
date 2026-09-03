//! Shape components and math data for `LmbrCentral`.
//!
//! O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/SplineComponent.cpp`.

mod bounds;
mod compound;
mod polygon_prism;
mod primitives;
mod spline;
mod vertex_shape;

use bevy::prelude::*;

pub use bounds::ShapeLocalBoundsQuery;
pub use compound::*;
pub use polygon_prism::*;
pub use primitives::*;
pub use spline::*;
pub use vertex_shape::*;

pub fn register_shape_components(app: &mut App) {
    app.register_type::<BoxShapeConfig>()
        .register_type_data::<BoxShapeConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<BoxShapeConfig, az_core::ReflectAzRtti>()
        .register_type::<BoxShapeComponent>()
        .register_type_data::<BoxShapeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<BoxShapeComponent, az_core::ReflectAzRtti>()
        .register_type::<SphereShapeConfig>()
        .register_type_data::<SphereShapeConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SphereShapeConfig, az_core::ReflectAzRtti>()
        .register_type::<SphereShapeComponent>()
        .register_type_data::<SphereShapeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SphereShapeComponent, az_core::ReflectAzRtti>()
        .register_type::<CapsuleShapeConfig>()
        .register_type_data::<CapsuleShapeConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CapsuleShapeConfig, az_core::ReflectAzRtti>()
        .register_type::<CapsuleShapeComponent>()
        .register_type_data::<CapsuleShapeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CapsuleShapeComponent, az_core::ReflectAzRtti>()
        .register_type::<CylinderShapeConfig>()
        .register_type_data::<CylinderShapeConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CylinderShapeConfig, az_core::ReflectAzRtti>()
        .register_type::<CylinderShapeComponent>()
        .register_type_data::<CylinderShapeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CylinderShapeComponent, az_core::ReflectAzRtti>()
        .register_type::<CompoundShapeConfiguration>()
        .register_type_data::<CompoundShapeConfiguration, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CompoundShapeConfiguration, az_core::ReflectAzRtti>()
        .register_type::<CompoundShapeComponent>()
        .register_type_data::<CompoundShapeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CompoundShapeComponent, az_core::ReflectAzRtti>()
        .register_type::<Vec<Vec3>>()
        .register_type::<Vec<BezierData>>()
        .register_type::<SplineType>()
        .register_type::<SplineData>()
        .register_type::<LinearSpline>()
        .register_type::<BezierData>()
        .register_type::<BezierSpline>()
        .register_type::<CatmullRomSpline>()
        .register_type::<Spline>()
        .register_type::<SplineCommon>()
        .register_type::<SplineComponent>()
        .register_type::<PolygonPrism>()
        .register_type::<PolygonPrismCommon>()
        .register_type::<PolygonPrismShapeComponent>()
        .register_type::<VertexShapeAsset>()
        .register_type::<VertexShapeMetadata>()
        .register_type::<VertexShapeReserved>();
}

#[cfg(test)]
mod tests;
