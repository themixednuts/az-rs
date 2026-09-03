//! Vegetation descriptor and spawner data.

mod assets;
#[allow(clippy::module_inception)]
mod descriptor;
mod mode;
mod spawner;
mod type_ids;
mod weight_selector;

use bevy::prelude::*;

pub use assets::*;
pub use descriptor::*;
pub use mode::*;
pub use spawner::*;
pub use type_ids::*;
pub use weight_selector::*;

pub fn register_descriptor_components(app: &mut App) {
    app.register_type::<BoundMode>()
        .register_type::<OverrideMode>()
        .register_type::<SortBehavior>()
        .register_type::<VegetationDescriptorSourceType>()
        .register_type::<InstanceSpawnerKind>()
        .register_type::<EmptyInstanceSpawner>()
        .register_type::<LegacyVegetationInstanceSpawner>()
        .register_type::<DynamicSliceInstanceSpawner>()
        .register_type::<InstanceSpawner>()
        .register_type::<VegetationDescriptor>()
        .register_type::<VegetationDescriptorUserData>()
        .register_type::<VegetationCodexAsset>()
        .register_type::<VegetationDescriptorAssetConfig>()
        .register_type::<VegetationDescriptorAssetComponent>()
        .register_type::<VegetationDescriptorListConfig>()
        .register_type::<VegetationDescriptorListComponent>()
        .register_type::<DescriptorWeightSelectorConfig>()
        .register_type::<DescriptorWeightSelectorComponent>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_gradient_signal::{GradientLookup, GradientSampleParams, GradientSampler};

    #[test]
    // `float_cmp`: The assertion is that `Default` hands back these exact Lumberyard constants.
    #[allow(clippy::float_cmp)]
    fn descriptor_defaults_match_source_in_bevy_axes() {
        let descriptor = VegetationDescriptor::default();

        assert_eq!(descriptor.weight, 1.0);
        assert!(!descriptor.advanced);
        assert_eq!(
            descriptor.spawner_kind(),
            InstanceSpawnerKind::LegacyVegetation
        );
        assert_eq!(
            descriptor.surface_filter_override_mode,
            OverrideMode::Disable
        );
        assert_eq!(descriptor.position_min, Vec3::new(-0.3, 0.0, -0.3));
        assert_eq!(descriptor.rotation_max_degrees, Vec3::new(0.0, 180.0, 0.0));
        assert_eq!(descriptor.scale_min, 0.1);
        assert_eq!(descriptor.altitude_filter_max, 128.0);
        assert_eq!(descriptor.slope_filter_max, 20.0);
    }

    #[test]
    fn descriptor_scene_asset_path_uses_spawner_asset_reference() {
        let legacy = VegetationDescriptor {
            instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let dynamic_slice = VegetationDescriptor {
            instance_spawner: InstanceSpawner::DynamicSlice(DynamicSliceInstanceSpawner {
                slice_asset_path: Some("Slices/Nature/Bush.slice".to_string()),
                slice_variant: Some("OakTree_a".to_string()),
            }),
            ..Default::default()
        };

        assert_eq!(legacy.scene_asset_path(), Some("Objects/Nature/Oak.cgf"));
        assert_eq!(legacy.scene_asset_variant(), None);
        assert_eq!(
            dynamic_slice.scene_asset_path(),
            Some("Slices/Nature/Bush.slice")
        );
        assert_eq!(dynamic_slice.scene_asset_variant(), Some("OakTree_a"));
        assert_eq!(VegetationDescriptor::default().scene_asset_path(), None);
    }

    #[test]
    fn descriptor_weight_selector_matches_source_behavior() {
        assert_eq!(
            VEGETATION_DESCRIPTOR_WEIGHT_SELECTOR_COMPONENT_TYPE_ID,
            uuid::uuid!("FDCE3BCD-E2FF-4E7A-A240-E3CCC359138C")
        );
        assert_eq!(
            VEGETATION_DESCRIPTOR_WEIGHT_SELECTOR_CONFIG_TYPE_ID,
            uuid::uuid!("D0B1B48C-CFB1-4007-AEF1-6BCB386E6D16")
        );
        assert_eq!(
            SortBehavior::from_native_value(0),
            Some(SortBehavior::Unsorted)
        );
        assert_eq!(SortBehavior::from_native_value(3), None);

        let descriptors = [
            VegetationDescriptor {
                weight: 1.0,
                ..Default::default()
            },
            VegetationDescriptor {
                weight: 2.0,
                ..Default::default()
            },
            VegetationDescriptor {
                weight: 3.0,
                ..Default::default()
            },
        ];
        let selector = DescriptorWeightSelectorConfig {
            sort_behavior: SortBehavior::Ascending,
            ..Default::default()
        };

        assert_eq!(
            selector.select_descriptor_indices(&descriptors, Vec3::ZERO, &FixedGradient(0.5)),
            vec![1, 2]
        );

        let selector = DescriptorWeightSelectorConfig {
            sort_behavior: SortBehavior::Descending,
            ..Default::default()
        };

        assert_eq!(
            selector.select_descriptor_indices(&descriptors, Vec3::ZERO, &FixedGradient(0.75)),
            vec![1, 0]
        );
    }

    struct FixedGradient(f32);

    impl GradientLookup for FixedGradient {
        fn sample_gradient(
            &self,
            _sampler: &GradientSampler,
            _params: GradientSampleParams,
        ) -> f32 {
            self.0
        }
    }
}
