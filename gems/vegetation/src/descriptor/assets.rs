//! Vegetation descriptor asset and list components.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use super::descriptor::VegetationDescriptor;
use super::mode::VegetationDescriptorSourceType;

/// Ordered vegetation descriptor asset.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
pub struct VegetationCodexAsset {
    pub descriptors: Vec<VegetationDescriptor>,
}

/// Vegetation descriptor asset configuration.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct VegetationDescriptorAssetConfig {
    pub source_type: VegetationDescriptorSourceType,
    pub codex_asset_path: Option<String>,
    pub descriptors: Vec<VegetationDescriptor>,
}

/// Vegetation descriptor asset component.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct VegetationDescriptorAssetComponent {
    pub config: VegetationDescriptorAssetConfig,
}

/// Vegetation descriptor list configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DescriptorListComponent.h:31`.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
pub struct VegetationDescriptorListConfig {
    pub vegetation_descriptors: Vec<VegetationDescriptor>,
}

impl VegetationDescriptorListConfig {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.vegetation_descriptors.is_empty()
    }

    #[must_use]
    pub const fn descriptor_count(&self) -> usize {
        self.vegetation_descriptors.len()
    }

    #[must_use]
    pub fn descriptor(&self, index: usize) -> Option<&VegetationDescriptor> {
        self.vegetation_descriptors.get(index)
    }
}

/// Vegetation descriptor list component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DescriptorListComponent.h:58`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(
    tag = "azoth.vegetation.VegetationDescriptorListComponent",
    version = 1
)]
pub struct VegetationDescriptorListComponent {
    pub configuration: VegetationDescriptorListConfig,
}
