//! Stable animation target identities shared by authoring and runtime.
//!
//! Hierarchy paths are presentation details and can differ between skeletons
//! that intentionally consume the same animation. Controller IDs are the
//! authored binding identity, so controller-keyed clips and character joints
//! derive their Bevy target paths from this module.

use serde::{Deserialize, Serialize};

pub const CONTROLLER_TARGET_SPACE: &str = "azoth.animation.controller-id.v1";
pub const CONTROLLER_TARGET_ROOT_NAME: &str = "__azoth_animation_controllers";

#[must_use]
pub fn controller_target_node_name(controller_id: u32) -> String {
    format!("controller_{controller_id:08x}")
}

#[must_use]
pub fn controller_target_path(controller_id: u32) -> [String; 2] {
    [
        CONTROLLER_TARGET_ROOT_NAME.to_string(),
        controller_target_node_name(controller_id),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationControllerNodeExtras {
    pub azoth_animation_controller_id: u32,
}

impl AnimationControllerNodeExtras {
    #[must_use]
    pub const fn new(controller_id: u32) -> Self {
        Self {
            azoth_animation_controller_id: controller_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationControllerBindingExtras {
    pub azoth_animation_target_space: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azoth_root_controller_id: Option<u32>,
}

impl AnimationControllerBindingExtras {
    #[must_use]
    pub fn new(root_controller_id: Option<u32>) -> Self {
        Self {
            azoth_animation_target_space: CONTROLLER_TARGET_SPACE.to_string(),
            azoth_root_controller_id: root_controller_id,
        }
    }

    #[must_use]
    pub fn uses_controller_targets(&self) -> bool {
        self.azoth_animation_target_space == CONTROLLER_TARGET_SPACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_target_path_is_stable_and_fixed_width() {
        assert_eq!(
            controller_target_path(0x12ab),
            [
                "__azoth_animation_controllers".to_string(),
                "controller_000012ab".to_string(),
            ]
        );
    }

    #[test]
    fn extras_use_the_shared_gltf_field_names() {
        assert_eq!(
            serde_json::to_value(AnimationControllerNodeExtras::new(7)).unwrap(),
            serde_json::json!({ "azothAnimationControllerId": 7 })
        );
        assert_eq!(
            serde_json::to_value(AnimationControllerBindingExtras::new(Some(7))).unwrap(),
            serde_json::json!({
                "azothAnimationTargetSpace": CONTROLLER_TARGET_SPACE,
                "azothRootControllerId": 7
            })
        );
    }
}
