//! Shared local bounds helpers.

use bevy::ecs::system::SystemParam;
use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;

use super::{
    BoxShapeComponent, CapsuleShapeComponent, CylinderShapeComponent, PolygonPrismShapeComponent,
    SphereShapeComponent, SplineComponent,
};

/// Entity-backed local bounds for `LmbrCentral` shape components.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/ShapeComponentBus.h:125`.
#[derive(SystemParam)]
pub struct ShapeLocalBoundsQuery<'w, 's> {
    boxes: Query<'w, 's, &'static BoxShapeComponent>,
    spheres: Query<'w, 's, &'static SphereShapeComponent>,
    capsules: Query<'w, 's, &'static CapsuleShapeComponent>,
    cylinders: Query<'w, 's, &'static CylinderShapeComponent>,
    splines: Query<'w, 's, &'static SplineComponent>,
    polygon_prisms: Query<'w, 's, &'static PolygonPrismShapeComponent>,
}

impl ShapeLocalBoundsQuery<'_, '_> {
    #[must_use]
    pub fn local_bounds(&self, entity: Entity) -> Option<Aabb3d> {
        if let Ok(component) = self.boxes.get(entity) {
            return Some(component.local_bounds());
        }
        if let Ok(component) = self.spheres.get(entity) {
            let radius = component.configuration.radius.max(0.0);
            return Some(Aabb3d::new(Vec3::ZERO, Vec3::splat(radius)));
        }
        if let Ok(component) = self.capsules.get(entity) {
            return Some(component.local_bounds());
        }
        if let Ok(component) = self.cylinders.get(entity) {
            return Some(component.local_bounds());
        }
        if let Ok(component) = self.splines.get(entity) {
            return component.configuration.spline.local_bounds();
        }
        if let Ok(component) = self.polygon_prisms.get(entity) {
            return component.configuration.local_bounds();
        }

        None
    }
}

pub(super) fn aabb_from_vec3_points(points: &[Vec3]) -> Option<Aabb3d> {
    let first = *points.first()?;
    let mut min = first;
    let mut max = first;

    for point in &points[1..] {
        min = min.min(*point);
        max = max.max(*point);
    }

    Some(aabb_from_min_max(min, max))
}

pub(super) fn aabb_from_min_max(min: Vec3, max: Vec3) -> Aabb3d {
    Aabb3d::new((min + max) * 0.5, (max - min) * 0.5)
}
