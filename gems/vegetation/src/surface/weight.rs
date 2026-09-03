use bevy::prelude::*;

use super::VegetationSurfaceTag;

/// A vegetation surface tag contribution weight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
pub struct VegetationSurfaceTagWeight {
    pub tag: VegetationSurfaceTag,
    pub weight: f32,
}

impl VegetationSurfaceTagWeight {
    #[must_use]
    pub const fn new(tag: VegetationSurfaceTag, weight: f32) -> Self {
        Self { tag, weight }
    }
}

/// Add a surface mask weight, keeping the maximum value for an existing tag.
///
/// O3DE reference: `Gems/SurfaceData/Code/Include/SurfaceData/Utility/SurfaceDataUtility.h:114`.
pub fn add_max_surface_weight(
    masks: &mut Vec<VegetationSurfaceTagWeight>,
    tag: VegetationSurfaceTag,
    weight: f32,
) {
    if let Some(mask) = masks.iter_mut().find(|mask| mask.tag == tag) {
        mask.weight = mask.weight.max(weight);
    } else {
        masks.push(VegetationSurfaceTagWeight::new(tag, weight));
    }
}

/// Merge surface mask weights, keeping the maximum value for each tag.
pub fn merge_max_surface_weights(
    masks: &mut Vec<VegetationSurfaceTagWeight>,
    source: impl IntoIterator<Item = VegetationSurfaceTagWeight>,
) {
    for mask in source {
        add_max_surface_weight(masks, mask.tag, mask.weight);
    }
}

#[must_use]
pub fn has_valid_surface_tags(tags: &[VegetationSurfaceTag]) -> bool {
    tags.iter()
        .any(|tag| *tag != VegetationSurfaceTag::UNASSIGNED)
}

#[must_use]
pub fn has_matching_surface_tag_weight(
    masks: &[VegetationSurfaceTagWeight],
    tags: &[VegetationSurfaceTag],
    weight_min: f32,
    weight_max: f32,
) -> bool {
    let min = weight_min.min(weight_max);
    let max = weight_min.max(weight_max);
    tags.iter()
        .filter(|tag| **tag != VegetationSurfaceTag::UNASSIGNED)
        .any(|tag| {
            masks
                .iter()
                .any(|mask| mask.tag == *tag && mask.weight >= min && mask.weight <= max)
        })
}

/// Surface tag offset data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
pub struct VegetationSurfaceTagOffset {
    pub surface_tag: VegetationSurfaceTag,
    pub offset: Vec3,
}

/// Surface tag depth data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
pub struct VegetationSurfaceTagDepth {
    pub surface_tag: VegetationSurfaceTag,
    pub min_depth_in_meters: f32,
}
