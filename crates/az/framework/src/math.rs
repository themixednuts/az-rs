//! AZ math bridges for Bevy-facing runtime data.
//!
//! Native Lumberyard/O3DE stores transforms in `AzCore/Math/Transform`.
//! The engine side of this repo uses Bevy's `Transform`, so this module
//! owns the small conversion surface instead of leaving it in importer tools.

use bevy::prelude::*;

/// Convert AZ's serialized 3x4 column transform into a Bevy [`Transform`].
///
/// The input layout is three basis columns followed by translation:
/// `[basis_x.xyz, basis_y.xyz, basis_z.xyz, translation.xyz]`.
#[inline]
#[must_use]
pub fn transform_columns_to_bevy(values: [f32; 12]) -> Transform {
    Transform::from_matrix(Mat4::from_cols_array(&[
        values[0], values[1], values[2], 0.0, values[3], values[4], values[5], 0.0, values[6],
        values[7], values[8], 0.0, values[9], values[10], values[11], 1.0,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_columns_map_translation_and_scale_to_bevy_transform() {
        let transform = transform_columns_to_bevy(transform_columns(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::splat(2.0),
        ));

        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, Vec3::splat(2.0));
    }

    #[test]
    fn identity_columns_map_to_default_bevy_transform() {
        let transform = transform_columns_to_bevy(transform_columns(Vec3::ZERO, Vec3::ONE));

        assert_eq!(transform.translation, Vec3::ZERO);
        assert_eq!(transform.scale, Vec3::ONE);
        assert_eq!(transform.rotation, Quat::IDENTITY);
    }

    fn transform_columns(translation: Vec3, scale: Vec3) -> [f32; 12] {
        [
            scale.x,
            0.0,
            0.0,
            0.0,
            scale.y,
            0.0,
            0.0,
            0.0,
            scale.z,
            translation.x,
            translation.y,
            translation.z,
        ]
    }
}
