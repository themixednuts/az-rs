//! Renderer-independent animation playback contracts shared by simple and
//! Mannequin animation systems.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Cry `CA_AnimationFlags` bit layout from Lumberyard.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct AnimationFlags: u32 {
        const MANUAL_UPDATE = 0x0000_0001;
        const LOOP_ANIMATION = 0x0000_0002;
        const REPEAT_LAST_KEY = 0x0000_0004;
        const TRANSITION_TIME_WARPING = 0x0000_0008;
        const START_AT_KEY_TIME = 0x0000_0010;
        const START_AFTER = 0x0000_0020;
        const IDLE_TO_MOVE = 0x0000_0040;
        const MOVE_TO_IDLE = 0x0000_0080;
        const ALLOW_ANIMATION_RESTART = 0x0000_0100;
        const KEYFRAME_SAMPLE_30_HZ = 0x0000_0200;
        const DISABLE_MULTI_LAYER = 0x0000_0400;
        const FORCE_SKELETON_UPDATE = 0x0000_0800;
        const TRACK_VIEW_EXCLUSIVE = 0x0000_1000;
        const REMOVE_FROM_FIFO = 0x0000_2000;
        const FULL_ROOT_PRIORITY = 0x0000_4000;
        const FORCE_TRANSITION_TO_ANIMATION = 0x0000_8000;
        /// Continue accepting desired motion parameters after a newer FIFO
        /// entry begins blending in.
        const UPDATE_MOTION_PARAMETERS_WHILE_BLENDING_OUT = 0x0002_0000;
        const FADE_OUT = 0x4000_0000;
    }
}

impl AnimationFlags {
    /// Cry resolves mutually-exclusive playback modes in this priority order.
    #[must_use]
    pub const fn playback_mode(self) -> AnimationPlaybackMode {
        if self.contains(Self::MANUAL_UPDATE) {
            AnimationPlaybackMode::Manual
        } else if self.contains(Self::LOOP_ANIMATION) {
            AnimationPlaybackMode::Loop
        } else if self.contains(Self::REPEAT_LAST_KEY) {
            AnimationPlaybackMode::RepeatLastKey
        } else {
            AnimationPlaybackMode::Once
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimationPlaybackMode {
    Manual,
    Loop,
    RepeatLastKey,
    #[default]
    Once,
}
