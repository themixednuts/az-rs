use az_core::component::{Component as AzComponentBase, EntityId};
use az_derive::{AzComponent, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

mod runtime;

pub(super) use runtime::register_attachment_runtime;
pub use runtime::{
    AttachmentComponentAction, AttachmentComponentNotification, AttachmentComponentRequest,
    AttachmentRuntime, AttachmentTarget, compose_attachment_transform,
};

pub const ATTACHMENT_COMPONENT_TYPE_ID: Uuid = uuid!("2D17A64A-7AC5-4C02-AC36-C5E8141FFDDF");
pub const ATTACHMENT_CONFIGURATION_TYPE_ID: Uuid = uuid!("74B5DC69-DE44-4640-836A-55339E116795");

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default, Serialize, Deserialize)]
pub enum AttachmentScaleSource {
    #[default]
    WorldScale = 0,
    TargetEntityScale = 1,
    TargetBoneScale = 2,
}

impl AttachmentScaleSource {
    #[inline]
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::WorldScale),
            1 => Some(Self::TargetEntityScale),
            2 => Some(Self::TargetBoneScale),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn native_value(self) -> u8 {
        self as u8
    }
}

/// Configuration data for [`AttachmentComponent`].
#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "AttachmentConfiguration", ATTACHMENT_CONFIGURATION_TYPE_ID)]
#[reflect(Default)]
pub struct AttachmentConfiguration {
    #[serde(rename = "Target ID", default)]
    pub target_id: EntityId,
    #[serde(rename = "Target Bone Name", default)]
    pub target_bone_name: String,
    #[serde(rename = "Target Offset", default)]
    pub target_offset: Transform,
    #[serde(rename = "Attached Initially", default)]
    pub attached_initially: bool,
    #[serde(rename = "Scale Source", default)]
    pub scale_source: AttachmentScaleSource,
    #[serde(rename = "Update Tolerance", default)]
    pub update_tolerance: f32,
}

impl Default for AttachmentConfiguration {
    fn default() -> Self {
        Self {
            target_id: EntityId::default(),
            target_bone_name: String::new(),
            target_offset: Transform::IDENTITY,
            attached_initially: true,
            scale_source: AttachmentScaleSource::WorldScale,
            update_tolerance: 0.0,
        }
    }
}

impl AttachmentConfiguration {
    #[inline]
    #[must_use]
    pub const fn has_target(&self) -> bool {
        self.target_id.is_valid()
    }
}

/// Attaches an entity to a target entity or target bone.
#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Default,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(name = "AttachmentComponent", ATTACHMENT_COMPONENT_TYPE_ID)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.AttachmentComponent", version = 1)]
#[require(Transform, GlobalTransform, AttachmentRuntime)]
pub struct AttachmentComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponentBase,
    #[serde(rename = "Configuration", default)]
    pub configuration: AttachmentConfiguration,
}

impl AttachmentComponent {
    #[inline]
    #[must_use]
    pub const fn attached_initially(&self) -> bool {
        self.configuration.attached_initially
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_scale_source_maps_native_values() {
        assert_eq!(
            AttachmentScaleSource::from_native_value(0),
            Some(AttachmentScaleSource::WorldScale)
        );
        assert_eq!(
            AttachmentScaleSource::from_native_value(1),
            Some(AttachmentScaleSource::TargetEntityScale)
        );
        assert_eq!(
            AttachmentScaleSource::from_native_value(2),
            Some(AttachmentScaleSource::TargetBoneScale)
        );
        assert_eq!(AttachmentScaleSource::from_native_value(3), None);
        assert_eq!(AttachmentScaleSource::TargetBoneScale.native_value(), 2);
    }

    #[test]
    fn attachment_configuration_defaults_to_world_scale_attachment() {
        let config = AttachmentConfiguration::default();

        assert!(!config.has_target());
        assert_eq!(config.target_offset, Transform::IDENTITY);
        assert!(config.attached_initially);
        assert_eq!(config.scale_source, AttachmentScaleSource::WorldScale);
    }
}
