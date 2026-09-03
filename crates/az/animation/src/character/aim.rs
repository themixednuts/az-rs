//! Renderer-independent Cry aim-pose procedural behavior.
//!
//! The character/animation backend implements [`AimPoseRuntime`]; Mannequin
//! owns the installed state and Cry's token/layer rules. This keeps pose
//! blending independent from Bevy graph representation and render skinning.

use bevy_math::Vec3;

use crate::{
    mannequin::{AnimationLane, AnimationPlayback, AnimationStartParameters},
    playback::AnimationFlags,
};

pub const AIM_POSE_TOKEN_BASE: u32 = 140_000;
pub const AIM_POSE_ACTIVE_BLEND_THRESHOLD: f32 = 0.01;

/// Resolves a typed authored reference into the backend's animation key.
pub trait AnimationResolver<N> {
    type Animation;

    fn resolve_animation(&self, reference: &N) -> Option<Self::Animation>;
}

/// Character capabilities used by Cry's built-in `AimPose` procedural clip.
///
/// Implementations normally delegate animation methods to a
/// `CharacterAnimationRuntime`/Bevy adapter and pose methods to the character's
/// directional pose-blender component.
pub trait AimPoseRuntime<K>: AnimationPlayback<K> {
    /// Current `AimTarget` action parameter, if the action supplied one.
    fn aim_target(&self, lane: AnimationLane) -> Option<Vec3>;

    /// Entity position plus its forward direction multiplied by ten.
    fn default_aim_target(&self, lane: AnimationLane) -> Vec3;

    /// Returns `None` when this character has no aim pose blender.
    fn aim_pose_blend(&self, lane: AnimationLane) -> Option<f32>;

    fn set_aim_pose_enabled(&mut self, lane: AnimationLane, enabled: bool);

    fn set_aim_pose_target(&mut self, lane: AnimationLane, target: Vec3);

    fn set_aim_pose_smooth_time(&mut self, lane: AnimationLane, seconds: f32);

    fn set_aim_pose_layer(&mut self, lane: AnimationLane);

    fn set_aim_pose_fade_in(&mut self, lane: AnimationLane, seconds: f32);

    fn set_aim_pose_fade_out(&mut self, lane: AnimationLane, seconds: f32);

    /// Supplies the next token from the runtime's [`AimPoseTokenGenerator`].
    fn next_aim_pose_token(&mut self) -> u32;

    /// Finds `token` anywhere in `layer`, but stops the layer only when that
    /// entry is the final FIFO animation. Returns whether the token was found.
    fn stop_animation_if_user_token_is_top(
        &mut self,
        lane: AnimationLane,
        token: u32,
        blend_time: f32,
    ) -> bool;
}

/// Shipping `GetNextToken`: a wrapping `u8` counter added to 140000.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AimPoseTokenGenerator {
    current: u8,
}

impl From<u8> for AimPoseTokenGenerator {
    fn from(current: u8) -> Self {
        Self { current }
    }
}

#[expect(
    clippy::copy_iterator,
    reason = "the shipping GetNextToken counter is a plain copyable u8; callers snapshot \
              and resume it deliberately"
)]
impl Iterator for AimPoseTokenGenerator {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        self.current = self.current.wrapping_add(1);
        Some(AIM_POSE_TOKEN_BASE + u32::from(self.current))
    }
}

/// State retained by one installed `AimPose` procedural clip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AimPoseState {
    lane: AnimationLane,
    token: Option<u32>,
}

impl AimPoseState {
    #[must_use]
    pub fn enter<K>(
        runtime: &mut impl AimPoseRuntime<K>,
        animation: Option<&K>,
        smooth_time: f32,
        lane: AnimationLane,
        blend_time: f32,
    ) -> Self {
        let target = runtime
            .aim_target(lane)
            .unwrap_or_else(|| runtime.default_aim_target(lane));
        let Some(current_blend) = runtime.aim_pose_blend(lane) else {
            return Self { lane, token: None };
        };

        runtime.set_aim_pose_enabled(lane, true);
        if current_blend <= AIM_POSE_ACTIVE_BLEND_THRESHOLD {
            runtime.set_aim_pose_target(lane, target);
        }
        runtime.set_aim_pose_smooth_time(lane, smooth_time);
        runtime.set_aim_pose_layer(lane);
        runtime.set_aim_pose_fade_in(lane, blend_time);

        let token = animation.map(|animation| {
            let token = runtime.next_aim_pose_token();
            let _started = runtime.start_animation(
                animation,
                AnimationStartParameters {
                    lane,
                    transition_time: blend_time,
                    key_time: 0.0,
                    playback_speed: 1.0,
                    playback_weight: 1.0,
                    blend_channels: [0.0; 4],
                    weight_list: 0,
                    user_token: token,
                    flags: AnimationFlags::LOOP_ANIMATION | AnimationFlags::ALLOW_ANIMATION_RESTART,
                },
            );
            token
        });

        Self { lane, token }
    }

    pub fn update<K>(&mut self, runtime: &mut impl AimPoseRuntime<K>) {
        if let Some(target) = runtime.aim_target(self.lane)
            && runtime.aim_pose_blend(self.lane).is_some()
        {
            runtime.set_aim_pose_target(self.lane, target);
        }
    }

    pub fn exit<K>(self, runtime: &mut impl AimPoseRuntime<K>, blend_time: f32) {
        let has_pose_blender = runtime.aim_pose_blend(self.lane).is_some();
        if let Some(token) = self.token {
            runtime.stop_animation_if_user_token_is_top(self.lane, token, blend_time);
        }
        if has_pose_blender {
            runtime.set_aim_pose_enabled(self.lane, false);
            runtime.set_aim_pose_fade_out(self.lane, blend_time);
        }
    }

    #[must_use]
    pub const fn lane(self) -> AnimationLane {
        self.lane
    }

    #[must_use]
    pub const fn token(self) -> Option<u32> {
        self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mannequin::ActiveAnimationState;

    fn lane(layer: u32) -> AnimationLane {
        AnimationLane::new(crate::mannequin::ScopeId::new(0).unwrap(), layer)
    }

    #[derive(Default)]
    struct Runtime {
        target: Option<Vec3>,
        blend: Option<f32>,
        enabled: bool,
        pose_target: Vec3,
        smooth_time: f32,
        layer: u32,
        fade_in: f32,
        fade_out: f32,
        tokens: AimPoseTokenGenerator,
        started: Vec<(u32, AnimationStartParameters)>,
        fifo_tokens: Vec<u32>,
        stopped: bool,
    }

    impl AnimationPlayback<u32> for Runtime {
        fn animation_duration(&self, _animation: &u32) -> Option<f32> {
            Some(1.0)
        }

        fn top_animation(&self, _lane: AnimationLane) -> Option<ActiveAnimationState> {
            None
        }

        fn start_animation(
            &mut self,
            animation: &u32,
            parameters: AnimationStartParameters,
        ) -> bool {
            self.fifo_tokens.push(parameters.user_token);
            self.started.push((*animation, parameters));
            true
        }

        fn stop_animation(&mut self, _lane: AnimationLane, _blend_time: f32) {
            self.stopped = true;
        }

        fn clear_layer(&mut self, _lane: AnimationLane) {}

        fn set_layer_playback_scale(&mut self, _lane: AnimationLane, _scale: f32) {}

        fn set_layer_blend_weight(&mut self, _lane: AnimationLane, _weight: f32) {}

        fn set_top_animation_weight(&mut self, _lane: AnimationLane, _weight: f32) {}

        fn set_top_animation_normalized_time(
            &mut self,
            _lane: AnimationLane,
            _normalized_time: f32,
        ) {
        }

        fn advance_layer_animations(
            &mut self,
            _lane: AnimationLane,
            _time_passed: f32,
            _queued_increments: &[f32],
        ) {
        }
    }

    impl AimPoseRuntime<u32> for Runtime {
        fn aim_target(&self, _lane: AnimationLane) -> Option<Vec3> {
            self.target
        }

        fn default_aim_target(&self, _lane: AnimationLane) -> Vec3 {
            Vec3::new(0.0, 10.0, 0.0)
        }

        fn aim_pose_blend(&self, _lane: AnimationLane) -> Option<f32> {
            self.blend
        }

        fn set_aim_pose_enabled(&mut self, _lane: AnimationLane, enabled: bool) {
            self.enabled = enabled;
        }

        fn set_aim_pose_target(&mut self, _lane: AnimationLane, target: Vec3) {
            self.pose_target = target;
        }

        fn set_aim_pose_smooth_time(&mut self, _lane: AnimationLane, seconds: f32) {
            self.smooth_time = seconds;
        }

        fn set_aim_pose_layer(&mut self, lane: AnimationLane) {
            self.layer = lane.layer;
        }

        fn set_aim_pose_fade_in(&mut self, _lane: AnimationLane, seconds: f32) {
            self.fade_in = seconds;
        }

        fn set_aim_pose_fade_out(&mut self, _lane: AnimationLane, seconds: f32) {
            self.fade_out = seconds;
        }

        fn next_aim_pose_token(&mut self) -> u32 {
            self.tokens
                .next()
                .expect("AimPose token generator is infinite")
        }

        fn stop_animation_if_user_token_is_top(
            &mut self,
            _lane: AnimationLane,
            token: u32,
            _blend_time: f32,
        ) -> bool {
            let Some(index) = self
                .fifo_tokens
                .iter()
                .position(|candidate| *candidate == token)
            else {
                return false;
            };
            if index + 1 == self.fifo_tokens.len() {
                self.stopped = true;
            }
            true
        }
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the runtime stores these floats verbatim, so an exact round-trip is the \
                  property under test"
    )]
    fn enter_sets_pose_and_starts_looping_animation_with_shipping_token() {
        let mut runtime = Runtime {
            blend: Some(0.0),
            ..Default::default()
        };

        let state = AimPoseState::enter(&mut runtime, Some(&7), 1.25, lane(4), 0.2);

        assert!(runtime.enabled);
        assert_eq!(runtime.pose_target, Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(runtime.smooth_time, 1.25);
        assert_eq!(runtime.layer, 4);
        assert_eq!(runtime.fade_in, 0.2);
        assert_eq!(state.token(), Some(140_001));
        assert_eq!(runtime.started[0].0, 7);
        assert_eq!(runtime.started[0].1.user_token, 140_001);
        assert_eq!(
            runtime.started[0].1.flags,
            AnimationFlags::LOOP_ANIMATION | AnimationFlags::ALLOW_ANIMATION_RESTART
        );
    }

    #[test]
    fn active_pose_preserves_target_and_update_tracks_new_action_target() {
        let original = Vec3::new(1.0, 2.0, 3.0);
        let mut runtime = Runtime {
            blend: Some(0.02),
            pose_target: original,
            ..Default::default()
        };
        let mut state = AimPoseState::enter(&mut runtime, None, 1.0, lane(4), 0.2);
        assert_eq!(runtime.pose_target, original);

        runtime.target = Some(Vec3::new(4.0, 5.0, 6.0));
        state.update(&mut runtime);
        assert_eq!(runtime.pose_target, runtime.target.unwrap());
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the runtime stores the fade-out time verbatim, so an exact round-trip is \
                  the property under test"
    )]
    fn exit_stops_only_when_its_token_is_top_and_disables_pose() {
        let mut runtime = Runtime {
            blend: Some(0.0),
            ..Default::default()
        };
        let state = AimPoseState::enter(&mut runtime, Some(&7), 1.0, lane(4), 0.2);
        runtime.fifo_tokens.push(99);

        state.exit(&mut runtime, 0.4);

        assert!(!runtime.stopped);
        assert!(!runtime.enabled);
        assert_eq!(runtime.fade_out, 0.4);
    }

    #[test]
    fn token_generator_wraps_like_shipping_u8_counter() {
        let mut tokens = AimPoseTokenGenerator::from(u8::MAX);
        assert_eq!(tokens.next(), Some(AIM_POSE_TOKEN_BASE));
        assert_eq!(tokens.next(), Some(AIM_POSE_TOKEN_BASE + 1));
    }
}
