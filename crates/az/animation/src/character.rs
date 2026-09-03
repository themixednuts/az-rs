//! Renderer-independent Cry character-animation FIFO and layer blending.
//!
//! Mannequin schedules clips into this runtime. Render backends consume the
//! resulting active instances, normalized times, and weights; they do not own
//! Cry's queue, transition, or layer semantics.

use arrayvec::ArrayVec;
use std::borrow::Borrow;

use crate::playback::AnimationFlags;

pub mod aim;
pub mod attachment;
pub mod definition;
pub mod positioning;

pub const ANIMATION_LAYER_COUNT: usize = 16;
pub const MAX_ANIMATIONS_PER_LAYER: usize = 16;
pub const MAX_EXECUTED_ANIMATIONS_PER_LAYER: usize = 8;
pub const ANIMATION_USER_DATA_SLOT_COUNT: usize = 8;
pub const MANNEQUIN_BLEND_CHANNEL_COUNT: usize = 4;

/// User data blended with an animation's transition weight.
///
/// Mannequin writes its four blend channels into slots `0..4` and leaves the
/// remaining slots at zero.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnimationUserData([f32; ANIMATION_USER_DATA_SLOT_COUNT]);

impl AnimationUserData {
    #[must_use]
    pub const fn new(values: [f32; ANIMATION_USER_DATA_SLOT_COUNT]) -> Self {
        Self(values)
    }

    /// Borrows the four Mannequin blend channels stored in the leading slots.
    ///
    /// # Panics
    ///
    /// Never in practice: the borrowed slice is a fixed
    /// `MANNEQUIN_BLEND_CHANNEL_COUNT` prefix of an
    /// `ANIMATION_USER_DATA_SLOT_COUNT`-element array, so the array conversion
    /// cannot fail.
    #[must_use]
    pub fn blend_channels(&self) -> &[f32; MANNEQUIN_BLEND_CHANNEL_COUNT] {
        self.0[..MANNEQUIN_BLEND_CHANNEL_COUNT]
            .try_into()
            .expect("the Cry user-data prefix always contains four blend channels")
    }

    #[must_use]
    pub const fn into_inner(self) -> [f32; ANIMATION_USER_DATA_SLOT_COUNT] {
        self.0
    }
}

impl From<[f32; ANIMATION_USER_DATA_SLOT_COUNT]> for AnimationUserData {
    fn from(values: [f32; ANIMATION_USER_DATA_SLOT_COUNT]) -> Self {
        Self::new(values)
    }
}

impl From<[f32; MANNEQUIN_BLEND_CHANNEL_COUNT]> for AnimationUserData {
    fn from(blend_channels: [f32; MANNEQUIN_BLEND_CHANNEL_COUNT]) -> Self {
        let mut values = [0.0; ANIMATION_USER_DATA_SLOT_COUNT];
        values[..MANNEQUIN_BLEND_CHANNEL_COUNT].copy_from_slice(&blend_channels);
        Self(values)
    }
}

impl AsRef<[f32; ANIMATION_USER_DATA_SLOT_COUNT]> for AnimationUserData {
    fn as_ref(&self) -> &[f32; ANIMATION_USER_DATA_SLOT_COUNT] {
        &self.0
    }
}

impl Borrow<[f32; ANIMATION_USER_DATA_SLOT_COUNT]> for AnimationUserData {
    fn borrow(&self) -> &[f32; ANIMATION_USER_DATA_SLOT_COUNT] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnimationInstanceId(u64);

impl AnimationInstanceId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterAnimationParameters {
    pub layer: u32,
    pub transition_time: f32,
    pub key_time: f32,
    pub playback_speed: f32,
    pub playback_weight: f32,
    pub user_data: AnimationUserData,
    pub expected_duration: f32,
    pub allow_multi_layer_animation: f32,
    pub user_token: u32,
    pub flags: AnimationFlags,
}

impl Default for CharacterAnimationParameters {
    fn default() -> Self {
        Self {
            layer: 0,
            transition_time: 0.0,
            key_time: 0.0,
            playback_speed: 1.0,
            playback_weight: 1.0,
            user_data: AnimationUserData::default(),
            expected_duration: 0.0,
            allow_multi_layer_animation: 1.0,
            user_token: 0,
            flags: AnimationFlags::empty(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StartAnimationError {
    #[error("animation layer {layer} is outside the shipping range 0..{ANIMATION_LAYER_COUNT}")]
    InvalidLayer { layer: u32 },
    #[error("animation layer {layer} already contains {MAX_ANIMATIONS_PER_LAYER} entries")]
    QueueFull { layer: u32 },
    #[error("the same animation is already at the top of layer {layer}")]
    RestartNotAllowed { layer: u32 },
    #[error("track-view-exclusive playback owns the character animation queues")]
    TrackViewExclusive,
    #[error("reverse character-animation playback is unsupported")]
    ReversePlayback,
}

/// Asset-dependent transition gates used by Cry's FIFO.
///
/// A backend can defer activation while a cooked clip is loading and provide
/// blend-space-specific idle-to-move / move-to-idle gates without putting
/// asset handles or renderer types in the core runtime.
pub trait AnimationTransitionPolicy<K, S = ()> {
    fn is_ready(&self, animation: &K) -> bool;

    fn animation_time_step(
        &mut self,
        animation: &mut TransitionAnimation<K, S>,
        delta_time: f32,
    ) -> Option<AnimationTimeStep> {
        (animation.expected_duration > 0.0).then(|| AnimationTimeStep {
            normalized_delta: delta_time.max(0.0) * animation.playback_scale
                / animation.expected_duration,
            expected_segment_duration: animation.expected_duration,
            expected_total_duration: animation.expected_duration,
            segment_count: 1,
        })
    }

    fn idle_to_move_ready(
        &self,
        _previous: &TransitionAnimation<K, S>,
        _next: &TransitionAnimation<K, S>,
    ) -> bool {
        true
    }

    fn entire_normalized_time(&self, animation: &TransitionAnimation<K, S>) -> Option<f32> {
        Some(animation.normalized_time)
    }

    fn synchronize_animation_state(
        &mut self,
        _previous: &TransitionAnimation<K, S>,
        _next: &mut TransitionAnimation<K, S>,
    ) {
    }

    fn shares_timewarp_group(
        &self,
        _previous: &TransitionAnimation<K, S>,
        _next: &TransitionAnimation<K, S>,
    ) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationTimeStep {
    pub normalized_delta: f32,
    pub expected_segment_duration: f32,
    pub expected_total_duration: f32,
    pub segment_count: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadyAnimationTransitions;

impl<K, S> AnimationTransitionPolicy<K, S> for ReadyAnimationTransitions {
    fn is_ready(&self, _animation: &K) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub struct TransitionAnimation<K, S = ()> {
    id: AnimationInstanceId,
    animation: K,
    flags: AnimationFlags,
    normalized_time: f32,
    previous_normalized_time: f32,
    segment_index: u8,
    previous_segment_index: u8,
    transition_time: f32,
    transition_priority: f32,
    transition_weight: f32,
    playback_scale: f32,
    playback_weight: f32,
    user_data: AnimationUserData,
    expected_duration: f32,
    expected_segment_duration: f32,
    start_time: f32,
    allow_multi_layer_animation: f32,
    user_token: u32,
    activated: bool,
    repeated: bool,
    loops_this_update: u32,
    segment_advances_this_update: u32,
    evaluation_count: u32,
    remove_from_queue: bool,
    state: S,
}

impl<K, S> TransitionAnimation<K, S> {
    #[must_use]
    pub const fn id(&self) -> AnimationInstanceId {
        self.id
    }

    #[must_use]
    pub const fn animation(&self) -> &K {
        &self.animation
    }

    #[must_use]
    pub const fn state(&self) -> &S {
        &self.state
    }

    pub const fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    #[must_use]
    pub const fn flags(&self) -> AnimationFlags {
        self.flags
    }

    #[must_use]
    pub const fn normalized_time(&self) -> f32 {
        self.normalized_time
    }

    #[must_use]
    pub const fn previous_normalized_time(&self) -> f32 {
        self.previous_normalized_time
    }

    #[must_use]
    pub const fn segment_index(&self) -> usize {
        self.segment_index as usize
    }

    #[must_use]
    pub const fn previous_segment_index(&self) -> usize {
        self.previous_segment_index as usize
    }

    #[must_use]
    pub const fn transition_weight(&self) -> f32 {
        self.transition_weight
    }

    #[must_use]
    pub const fn playback_scale(&self) -> f32 {
        self.playback_scale
    }

    #[must_use]
    pub const fn playback_weight(&self) -> f32 {
        self.playback_weight
    }

    #[must_use]
    pub const fn user_data(&self) -> &AnimationUserData {
        &self.user_data
    }

    #[must_use]
    pub fn blend_channels(&self) -> &[f32; MANNEQUIN_BLEND_CHANNEL_COUNT] {
        self.user_data.blend_channels()
    }

    #[must_use]
    pub const fn expected_duration(&self) -> f32 {
        self.expected_duration
    }

    #[must_use]
    pub const fn expected_segment_duration(&self) -> f32 {
        self.expected_segment_duration
    }

    #[must_use]
    pub const fn allow_multi_layer_animation(&self) -> f32 {
        self.allow_multi_layer_animation
    }

    #[must_use]
    pub const fn user_token(&self) -> u32 {
        self.user_token
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }

    #[must_use]
    pub const fn has_repeated(&self) -> bool {
        self.repeated
    }

    #[must_use]
    pub const fn loops_this_update(&self) -> u32 {
        self.loops_this_update
    }

    #[must_use]
    pub const fn segment_advances_this_update(&self) -> u32 {
        self.segment_advances_this_update
    }

    #[must_use]
    pub const fn is_first_evaluation(&self) -> bool {
        self.evaluation_count == 1
    }

    #[must_use]
    pub const fn effective_weight(&self, layer_weight: f32) -> f32 {
        self.transition_weight * self.playback_weight * layer_weight
    }
}

#[derive(Debug, Clone)]
pub struct AnimationTransitionQueue<K, S = ()> {
    animations: ArrayVec<TransitionAnimation<K, S>, MAX_ANIMATIONS_PER_LAYER>,
    layer_playback_scale: f32,
    layer_transition_time: f32,
    layer_transition_weight: f32,
    layer_blend_weight: f32,
    manual_mixing_weight: f32,
    active: bool,
}

impl<K, S> Default for AnimationTransitionQueue<K, S> {
    fn default() -> Self {
        Self {
            animations: ArrayVec::new(),
            layer_playback_scale: 1.0,
            layer_transition_time: 0.0,
            layer_transition_weight: 0.0,
            layer_blend_weight: 1.0,
            manual_mixing_weight: 0.0,
            active: false,
        }
    }
}

impl<K, S> AnimationTransitionQueue<K, S> {
    #[must_use]
    pub fn animations(&self) -> &[TransitionAnimation<K, S>] {
        &self.animations
    }

    pub fn animations_mut(&mut self) -> &mut [TransitionAnimation<K, S>] {
        &mut self.animations
    }

    /// FIFO prefix eligible for transition, sampling, and pose execution.
    #[must_use]
    pub fn executed_animations(&self) -> &[TransitionAnimation<K, S>] {
        &self.animations[..self.animations.len().min(MAX_EXECUTED_ANIMATIONS_PER_LAYER)]
    }

    pub fn executed_animations_mut(&mut self) -> &mut [TransitionAnimation<K, S>] {
        let count = self.animations.len().min(MAX_EXECUTED_ANIMATIONS_PER_LAYER);
        &mut self.animations[..count]
    }

    /// Activated, contiguous prefix of the executable FIFO.
    #[must_use]
    pub fn active_animations(&self) -> &[TransitionAnimation<K, S>] {
        let count = self
            .executed_animations()
            .iter()
            .take_while(|animation| animation.activated)
            .count();
        &self.animations[..count]
    }

    pub fn active_animations_mut(&mut self) -> &mut [TransitionAnimation<K, S>] {
        let count = self
            .executed_animations()
            .iter()
            .take_while(|animation| animation.activated)
            .count();
        &mut self.animations[..count]
    }

    #[must_use]
    pub fn top(&self) -> Option<&TransitionAnimation<K, S>> {
        self.executed_animations()
            .iter()
            .rev()
            .find(|animation| animation.activated)
    }

    pub fn top_mut(&mut self) -> Option<&mut TransitionAnimation<K, S>> {
        self.executed_animations_mut()
            .iter_mut()
            .rev()
            .find(|animation| animation.activated)
    }

    #[must_use]
    pub const fn layer_playback_scale(&self) -> f32 {
        self.layer_playback_scale
    }

    #[must_use]
    pub const fn layer_transition_weight(&self) -> f32 {
        self.layer_transition_weight
    }

    #[must_use]
    pub const fn layer_blend_weight(&self) -> f32 {
        self.layer_blend_weight
    }

    #[must_use]
    pub const fn manual_mixing_weight(&self) -> f32 {
        self.manual_mixing_weight
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    fn layer_weight(&self, layer: usize) -> f32 {
        if layer == 0 {
            1.0
        } else {
            self.layer_transition_weight * self.layer_blend_weight
        }
    }

    fn activate_ready_prefix(&mut self, policy: &impl AnimationTransitionPolicy<K, S>) {
        if let Some(mut forced_index) = self.executed_animations().iter().position(|animation| {
            animation
                .flags
                .contains(AnimationFlags::FORCE_TRANSITION_TO_ANIMATION)
        }) {
            let mut index = 0;
            while index < forced_index {
                if policy.is_ready(&self.animations[index].animation) {
                    self.animations[index].flags.remove(
                        AnimationFlags::START_AT_KEY_TIME
                            | AnimationFlags::START_AFTER
                            | AnimationFlags::IDLE_TO_MOVE
                            | AnimationFlags::MOVE_TO_IDLE,
                    );
                    self.animations[index].activated = true;
                    index += 1;
                } else {
                    self.animations.remove(index);
                    forced_index -= 1;
                }
            }
        }

        if let Some(first) = self.animations.first_mut()
            && !first.activated
            && policy.is_ready(&first.animation)
        {
            first.activated = true;
        }

        let executed_count = self.animations.len().min(MAX_EXECUTED_ANIMATIONS_PER_LAYER);
        for index in 1..executed_count {
            let (previous, next) = {
                let (before, after) = self.animations.split_at_mut(index);
                (&before[index - 1], &mut after[0])
            };
            if next.activated {
                continue;
            }
            if !previous.activated || !policy.is_ready(&next.animation) {
                break;
            }

            let start_at_key_time = next.flags.contains(AnimationFlags::START_AT_KEY_TIME);
            let start_after = next.flags.contains(AnimationFlags::START_AFTER)
                && !previous.flags.contains(AnimationFlags::LOOP_ANIMATION);
            let idle_to_move = next.flags.contains(AnimationFlags::IDLE_TO_MOVE);
            let delayed = start_at_key_time || start_after || idle_to_move;
            let activate = !delayed
                || (start_at_key_time
                    && policy.entire_normalized_time(previous).is_some_and(|time| {
                        time - 0.000_001 < next.start_time && next.start_time < time
                    }))
                || (start_after && previous.repeated)
                || (idle_to_move && policy.idle_to_move_ready(previous, next));
            if !activate {
                break;
            }
            next.activated = true;
        }
    }

    #[expect(
        clippy::suboptimal_flops,
        reason = "bit-exact port of CryAnimation CSkeletonAnim::UpdateTransitionWeights; \
                  folding the smoothstep denominator into mul_add changes the rounding \
                  and therefore the observable blend weights"
    )]
    fn update_transition_weights(&mut self, delta_time: f32) {
        let active_count = self.active_animations().len();
        if active_count == 0 {
            return;
        }

        self.animations[0].transition_priority = 1.0;
        for animation in self.animations.iter_mut().take(active_count).skip(1) {
            if delta_time == 0.0 && animation.transition_time == 0.0 {
                animation.transition_priority = 1.0;
                continue;
            }
            let transition_time = if animation.transition_time == 0.0 {
                0.0001
            } else {
                animation.transition_time
            };
            animation.transition_priority =
                (animation.transition_priority + delta_time.abs() / transition_time).min(1.0);
        }

        self.animations[0].transition_weight = 1.0;
        for index in 1..active_count {
            let priority = self.animations[index].transition_priority;
            self.animations[index].transition_weight = priority;
            let previous_scale = 1.0 - priority;
            for previous in &mut self.animations[..index] {
                previous.transition_weight *= previous_scale;
            }
        }

        let mut sum = 0.0;
        for animation in &mut self.animations[..active_count] {
            let centered = animation.transition_weight.clamp(0.0, 1.0) - 0.5;
            animation.transition_weight = centered / (0.5 + 2.0 * centered * centered) + 0.5;
            sum += animation.transition_weight;
        }
        if sum > 0.0 {
            for animation in &mut self.animations[..active_count] {
                animation.transition_weight /= sum;
            }
        }
    }

    fn apply_manual_mixing_weight(&mut self) {
        let active_count = self.active_animations().len();
        match active_count {
            1 => self.animations[0].transition_weight = 1.0,
            2 => {
                self.animations[0].transition_weight = 1.0 - self.manual_mixing_weight;
                self.animations[1].transition_weight = self.manual_mixing_weight;
            }
            _ => {}
        }
    }

    fn synchronize_time_warped_animations(
        &mut self,
        policy: &mut impl AnimationTransitionPolicy<K, S>,
    ) where
        K: PartialEq,
    {
        let active_count = self.active_animations().len();
        for index in 1..active_count {
            if !self.animations[index]
                .flags
                .contains(AnimationFlags::TRANSITION_TIME_WARPING)
            {
                continue;
            }

            if self.animations[index].transition_priority == 0.0 {
                self.animations[index].previous_normalized_time =
                    self.animations[index - 1].previous_normalized_time;
                self.animations[index].normalized_time = self.animations[index - 1].normalized_time;
            }

            for previous in (0..index).rev() {
                let (before, after) = self.animations.split_at_mut(index);
                let previous_animation = &before[previous];
                let next_animation = &mut after[0];
                if previous_animation.animation == next_animation.animation
                    && previous_animation.transition_weight > f32::EPSILON
                {
                    next_animation.previous_normalized_time =
                        previous_animation.previous_normalized_time;
                    next_animation.normalized_time = previous_animation.normalized_time;
                    policy.synchronize_animation_state(previous_animation, next_animation);
                }
            }
        }
    }

    fn animation_time_steps(
        &mut self,
        delta_time: f32,
        policy: &mut impl AnimationTransitionPolicy<K, S>,
    ) -> [Option<AnimationTimeStep>; MAX_ANIMATIONS_PER_LAYER] {
        let mut steps = [None; MAX_ANIMATIONS_PER_LAYER];
        for (index, animation) in self.executed_animations_mut().iter_mut().enumerate() {
            if !animation.activated {
                break;
            }
            steps[index] = policy.animation_time_step(animation, delta_time);
        }
        steps
    }

    fn synchronize_parametric_timewarp_groups(
        &self,
        layer: usize,
        steps: &mut [Option<AnimationTimeStep>; MAX_ANIMATIONS_PER_LAYER],
        policy: &impl AnimationTransitionPolicy<K, S>,
    ) {
        if layer == 0 {
            return;
        }

        let active_count = self.active_animations().len();
        #[expect(
            clippy::needless_range_loop,
            reason = "the index also bounds the `self.animations[..index]` search for the preceding group member"
        )]
        for index in 1..active_count {
            let current = &self.animations[index];
            let Some(previous) = self.animations[..index]
                .iter()
                .rev()
                .find(|previous| policy.shares_timewarp_group(previous, current))
            else {
                continue;
            };
            let Some(step) = &mut steps[index] else {
                continue;
            };

            let phase_delta = previous.normalized_time - current.normalized_time;
            step.normalized_delta = if phase_delta < 0.0 {
                phase_delta + 1.0
            } else {
                phase_delta
            };
        }
    }

    #[expect(
        clippy::suboptimal_flops,
        reason = "bit-exact port of CryAnimation's transition time-warping accumulation; \
                  fusing the multiply into the accumulator changes the rounding of the \
                  weighted normalized delta"
    )]
    fn adjust_time_warped_animation_steps(
        &self,
        steps: &mut [Option<AnimationTimeStep>; MAX_ANIMATIONS_PER_LAYER],
    ) {
        let active_count = self.active_animations().len();
        if active_count == 0 {
            return;
        }

        let mut use_time_warping = [false; MAX_ANIMATIONS_PER_LAYER];
        for index in 1..active_count {
            if self.animations[index]
                .flags
                .contains(AnimationFlags::TRANSITION_TIME_WARPING)
            {
                use_time_warping[index - 1] = true;
                use_time_warping[index] = true;
            } else {
                use_time_warping[index] = false;
            }
        }

        let mut start = 0;
        let mut count = 0;
        let mut weighted_delta = 0.0;
        let mut total_weight = 0.0;
        for index in 0..active_count {
            if !use_time_warping[index] {
                continue;
            }
            if count == 0 {
                start = index;
            }
            let weight = self.animations[index].transition_weight;
            total_weight += weight;
            weighted_delta += steps[index].map_or(0.0, |step| step.normalized_delta) * weight;
            count += 1;
        }
        let normalized_delta = if total_weight == 0.0 {
            0.0
        } else {
            weighted_delta / total_weight
        };
        for step in steps.iter_mut().skip(start).take(count).flatten() {
            step.normalized_delta = normalized_delta;
        }
    }

    /// Advances the normalized time of every executed entry.
    ///
    /// Ports `CSkeletonAnim::UpdateAnimationTime` from
    /// Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_BlendMan.cpp`.
    /// See the unwind-loop comment for the one deliberate divergence.
    #[expect(
        clippy::while_float,
        reason = "Cry unwinds one whole normalized cycle per iteration, so the loop \
                  condition must stay an exact float comparison against 1.0"
    )]
    fn update_animation_times(
        &mut self,
        layer: usize,
        steps: &[Option<AnimationTimeStep>; MAX_ANIMATIONS_PER_LAYER],
        track_view_exclusive: bool,
    ) {
        let queue_len = self.animations.len();
        let mut deactivate_layer = false;
        for (index, animation) in self.executed_animations_mut().iter_mut().enumerate() {
            if !animation.activated {
                break;
            }
            animation.loops_this_update = 0;
            animation.segment_advances_this_update = 0;
            if animation.flags.contains(AnimationFlags::MANUAL_UPDATE) {
                animation.evaluation_count = animation.evaluation_count.saturating_add(1);
                if !track_view_exclusive && index == 0 && animation.transition_weight == 0.0 {
                    animation.remove_from_queue = true;
                }
                continue;
            }

            animation.previous_normalized_time = animation.normalized_time;
            animation.previous_segment_index = animation.segment_index;
            if let Some(time_step) = steps[index] {
                animation.expected_segment_duration =
                    time_step.expected_segment_duration.max(0.0001);
                animation.expected_duration = time_step.expected_total_duration.max(0.0001);
                animation.normalized_time += time_step.normalized_delta;
                let segment_count = time_step.segment_count.max(1);
                // Cry takes the whole part in one shot and then steps the
                // segment counter exactly ONCE, however many whole units were
                // crossed:
                //
                //     int numLoops = (int)m_fAnimTime[idx];        // BlendMan:394
                //     if (numLoops > 0) {
                //         m_fAnimTime[idx] -= (float)numLoops;     // BlendMan:397
                //         ...
                //         ++m_currentSegmentIndex[idx];            // BlendMan:443
                //     }
                //
                // Crossing more than one unit is outside Cry's contract:
                // `SkeletonAnim_BlendMan.cpp:349` asserts `m_fAnimTime[idx] <= 2.0f`
                // on entry, and neither consumer can express a second wrap.
                // `ParseLayer0` pins `m_nEOC` to -1/0/+1
                // (`SkeletonAnim_Locator.cpp:294`, `:378`, `:382`) so the root
                // delta is reconstructed for exactly one cycle (`:310`, `:399`),
                // and `AnimCallback` emits exactly one `[prev,1] + [0,new]` pair
                // (`:758`, `:760`). Past one wrap Cry therefore drops whole
                // cycles of root motion and whole passes of animation events.
                //
                // We match Cry exactly inside its contract - with a normalized
                // delta of at most 1.0 this loop runs at most once - and keep
                // unwinding past it, because pinning the segment index one step
                // behind the clock while discarding motion is a frame-hitch bug,
                // not a behaviour worth reproducing. `loops_this_update` and
                // `segment_advances_this_update` carry the counts so the motion
                // runtime can compose every crossed cycle.
                while animation.normalized_time >= 1.0 {
                    animation.segment_advances_this_update += 1;
                    if animation.segment_index + 1 < segment_count {
                        animation.normalized_time -= 1.0;
                        animation.segment_index += 1;
                        continue;
                    }

                    if animation.flags.contains(AnimationFlags::LOOP_ANIMATION) {
                        animation.normalized_time -= 1.0;
                        animation.segment_index = 0;
                        animation.loops_this_update += 1;
                        continue;
                    }

                    animation.normalized_time = 1.0;
                    animation.segment_index = segment_count - 1;
                    animation.repeated = true;
                    if !animation.flags.contains(AnimationFlags::REPEAT_LAST_KEY) {
                        animation.remove_from_queue = true;
                    }

                    if queue_len == 1
                        && layer != 0
                        && animation.flags.contains(AnimationFlags::REPEAT_LAST_KEY)
                        && animation.flags.contains(AnimationFlags::FADE_OUT)
                    {
                        deactivate_layer = true;
                    }
                    break;
                }
            }

            if !track_view_exclusive && index == 0 && animation.transition_weight == 0.0 {
                animation.remove_from_queue = true;
            }
            animation.evaluation_count = animation.evaluation_count.saturating_add(1);
        }
        if deactivate_layer {
            self.active = false;
            self.layer_transition_time = 0.5;
        }
    }

    fn update_layer_blend(&mut self, layer: usize, delta_time: f32) {
        if layer == 0 {
            self.layer_transition_weight = 1.0;
            return;
        }
        let transition_time = self.layer_transition_time.max(0.00001);
        let delta = delta_time / transition_time;
        self.layer_transition_weight = if self.active {
            (self.layer_transition_weight + delta).min(1.0)
        } else {
            (self.layer_transition_weight - delta).max(0.0)
        };
        if !self.active
            && self.layer_transition_weight == 0.0
            && let Some(first) = self.animations.first_mut()
        {
            first.remove_from_queue = true;
        }
    }

    fn finish_update(&mut self) {
        if self
            .animations
            .first()
            .is_some_and(|animation| animation.remove_from_queue)
        {
            self.animations.remove(0);
        }
        if self.animations.is_empty() {
            self.active = false;
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharacterAnimationRuntime<K, S = ()> {
    layers: [AnimationTransitionQueue<K, S>; ANIMATION_LAYER_COUNT],
    next_instance: u64,
    track_view_exclusive: bool,
}

impl<K, S> Default for CharacterAnimationRuntime<K, S> {
    fn default() -> Self {
        Self {
            layers: std::array::from_fn(|_| AnimationTransitionQueue::default()),
            next_instance: 1,
            track_view_exclusive: false,
        }
    }
}

impl<K, S> CharacterAnimationRuntime<K, S>
where
    K: PartialEq,
{
    #[must_use]
    pub const fn layers(&self) -> &[AnimationTransitionQueue<K, S>; ANIMATION_LAYER_COUNT] {
        &self.layers
    }

    pub fn layer(&self, layer: u32) -> Option<&AnimationTransitionQueue<K, S>> {
        self.layers.get(layer as usize)
    }

    pub fn layer_mut(&mut self, layer: u32) -> Option<&mut AnimationTransitionQueue<K, S>> {
        self.layers.get_mut(layer as usize)
    }

    pub const fn set_track_view_exclusive(&mut self, enabled: bool) {
        self.track_view_exclusive = enabled;
    }

    /// Queues `animation` on its layer with a default per-instance state.
    ///
    /// # Errors
    ///
    /// Forwards every error of [`Self::start_animation_with_state`]:
    /// [`StartAnimationError::ReversePlayback`],
    /// [`StartAnimationError::InvalidLayer`],
    /// [`StartAnimationError::TrackViewExclusive`],
    /// [`StartAnimationError::QueueFull`], and
    /// [`StartAnimationError::RestartNotAllowed`].
    pub fn start_animation(
        &mut self,
        animation: K,
        parameters: CharacterAnimationParameters,
    ) -> Result<AnimationInstanceId, StartAnimationError>
    where
        S: Default,
    {
        self.start_animation_with_state(animation, parameters, S::default())
    }

    /// Queues `animation` on its layer alongside caller-supplied state.
    ///
    /// # Errors
    ///
    /// Returns [`StartAnimationError::ReversePlayback`] for a negative playback
    /// speed, [`StartAnimationError::InvalidLayer`] when the requested layer is
    /// out of range, [`StartAnimationError::TrackViewExclusive`] while a
    /// `TrackView`-exclusive animation owns the runtime,
    /// [`StartAnimationError::QueueFull`] when the layer FIFO is full, and
    /// [`StartAnimationError::RestartNotAllowed`] when the same animation is
    /// already on top and restarting was not requested.
    pub fn start_animation_with_state(
        &mut self,
        animation: K,
        mut parameters: CharacterAnimationParameters,
        state: S,
    ) -> Result<AnimationInstanceId, StartAnimationError> {
        if parameters.playback_speed < 0.0 {
            return Err(StartAnimationError::ReversePlayback);
        }
        let layer_index = parameters.layer as usize;
        let Some(layer) = self.layers.get_mut(layer_index) else {
            return Err(StartAnimationError::InvalidLayer {
                layer: parameters.layer,
            });
        };
        if self.track_view_exclusive
            && !parameters
                .flags
                .contains(AnimationFlags::TRACK_VIEW_EXCLUSIVE)
        {
            return Err(StartAnimationError::TrackViewExclusive);
        }
        if layer.animations.is_full() {
            return Err(StartAnimationError::QueueFull {
                layer: parameters.layer,
            });
        }
        if !parameters
            .flags
            .contains(AnimationFlags::ALLOW_ANIMATION_RESTART)
            && layer
                .animations
                .last()
                .is_some_and(|current| current.animation == animation)
        {
            return Err(StartAnimationError::RestartNotAllowed {
                layer: parameters.layer,
            });
        }

        if layer_index > 0
            && !parameters
                .flags
                .intersects(AnimationFlags::LOOP_ANIMATION | AnimationFlags::REPEAT_LAST_KEY)
        {
            parameters
                .flags
                .insert(AnimationFlags::REPEAT_LAST_KEY | AnimationFlags::FADE_OUT);
        }
        if parameters
            .flags
            .contains(AnimationFlags::DISABLE_MULTI_LAYER)
        {
            parameters.allow_multi_layer_animation = 0.0;
        }
        if parameters.flags.contains(AnimationFlags::REMOVE_FROM_FIFO)
            && !layer.animations.is_empty()
        {
            layer.animations.remove(0);
        }

        let id = AnimationInstanceId(self.next_instance);
        self.next_instance = self.next_instance.wrapping_add(1).max(1);
        layer.animations.push(TransitionAnimation {
            id,
            animation,
            flags: parameters.flags,
            normalized_time: parameters.key_time.clamp(0.0, 1.0),
            previous_normalized_time: parameters.key_time.clamp(0.0, 1.0),
            segment_index: 0,
            previous_segment_index: 0,
            transition_time: parameters.transition_time.max(0.0),
            transition_priority: 0.0,
            transition_weight: 0.0,
            playback_scale: parameters.playback_speed,
            playback_weight: parameters.playback_weight,
            user_data: parameters.user_data,
            expected_duration: parameters.expected_duration.max(0.0),
            expected_segment_duration: parameters.expected_duration.max(0.0),
            start_time: parameters.key_time.clamp(0.0, 1.0),
            allow_multi_layer_animation: parameters.allow_multi_layer_animation,
            user_token: parameters.user_token,
            activated: false,
            repeated: false,
            loops_this_update: 0,
            segment_advances_this_update: 0,
            evaluation_count: 0,
            remove_from_queue: false,
            state,
        });
        layer.active = true;
        layer.layer_transition_time = parameters.transition_time.abs();
        Ok(id)
    }

    pub fn stop_animation(&mut self, layer: u32, blend_out_time: f32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        queue.active = false;
        queue.layer_transition_time = blend_out_time;
        if layer == 0 {
            queue.animations.clear();
        }
        true
    }

    /// Cry FIFO lookup used by procedural clips that own an animation through
    /// a user token. A found non-top entry is deliberately left alone because
    /// a newer animation is already blending in above it.
    pub fn stop_animation_if_user_token_is_top(
        &mut self,
        layer: u32,
        user_token: u32,
        blend_out_time: f32,
    ) -> bool {
        let Some(queue) = self.layers.get(layer as usize) else {
            return false;
        };
        let Some(index) = queue
            .animations
            .iter()
            .position(|animation| animation.user_token == user_token)
        else {
            return false;
        };
        let is_top = index + 1 == queue.animations.len();
        if is_top {
            self.stop_animation(layer, blend_out_time);
        }
        true
    }

    pub fn clear_layer(&mut self, layer: u32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        queue.animations.clear();
        queue.active = false;
        true
    }

    pub fn clear_all(&mut self) {
        for layer in &mut self.layers {
            layer.animations.clear();
            layer.active = false;
        }
    }

    pub fn set_layer_playback_scale(&mut self, layer: u32, scale: f32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        queue.layer_playback_scale = scale.max(0.0);
        true
    }

    pub fn set_layer_blend_weight(&mut self, layer: u32, weight: f32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        queue.layer_blend_weight = weight;
        true
    }

    pub fn set_manual_mixing_weight(&mut self, layer: u32, weight: f32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        queue.manual_mixing_weight = weight.clamp(0.0, 1.0);
        true
    }

    pub fn set_top_animation_weight(&mut self, layer: u32, weight: f32) -> bool {
        let Some(animation) = self
            .layer_mut(layer)
            .and_then(AnimationTransitionQueue::top_mut)
        else {
            return false;
        };
        animation.playback_weight = weight;
        true
    }

    pub fn set_top_animation_flags(&mut self, layer: u32, flags: AnimationFlags) -> bool {
        let Some(animation) = self
            .layer_mut(layer)
            .and_then(AnimationTransitionQueue::top_mut)
        else {
            return false;
        };
        animation.flags.insert(flags);
        true
    }

    pub fn set_top_animation_normalized_time(&mut self, layer: u32, time: f32) -> bool {
        self.set_top_animation_segment_time(layer, 0, time)
    }

    pub fn set_top_animation_segment_time(
        &mut self,
        layer: u32,
        segment: usize,
        phase: f32,
    ) -> bool {
        let Some(animation) = self
            .layer_mut(layer)
            .and_then(AnimationTransitionQueue::top_mut)
        else {
            return false;
        };
        animation.previous_normalized_time = animation.normalized_time;
        animation.previous_segment_index = animation.segment_index;
        animation.segment_index = u8::try_from(segment).unwrap_or(u8::MAX);
        animation.normalized_time = phase.clamp(0.0, 1.0);
        true
    }

    /// Replace the top FIFO entry's motion while preserving its timing,
    /// transition weights, flags, user data, token, and queue position.
    /// This is the shipping `SetAnimationByTag` operation.
    pub fn replace_top_animation(
        &mut self,
        layer: u32,
        animation: K,
        expected_duration: f32,
    ) -> bool {
        let Some(top) = self
            .layer_mut(layer)
            .and_then(AnimationTransitionQueue::top_mut)
        else {
            return false;
        };
        top.animation = animation;
        top.expected_duration = expected_duration.max(0.0);
        top.expected_segment_duration = expected_duration.max(0.0);
        top.segment_index = 0;
        top.previous_segment_index = 0;
        true
    }

    pub fn advance_layer(&mut self, layer: u32, elapsed: f32) -> bool {
        let Some(queue) = self.layers.get_mut(layer as usize) else {
            return false;
        };
        for animation in queue.active_animations_mut() {
            if animation.expected_duration <= 0.0 {
                continue;
            }
            animation.previous_normalized_time = animation.normalized_time;
            animation.previous_segment_index = animation.segment_index;
            animation.normalized_time = (animation.normalized_time
                + elapsed * animation.playback_scale / animation.expected_segment_duration)
                .clamp(0.0, 1.0);
        }
        true
    }

    pub fn update(&mut self, delta_time: f32, policy: &mut impl AnimationTransitionPolicy<K, S>) {
        for (index, layer) in self.layers.iter_mut().enumerate() {
            layer.finish_update();
            layer.activate_ready_prefix(policy);
            let scaled_delta_time = delta_time * layer.layer_playback_scale;
            if self.track_view_exclusive {
                layer.apply_manual_mixing_weight();
            } else {
                layer.synchronize_time_warped_animations(policy);
                layer.update_transition_weights(scaled_delta_time);
            }
            let mut steps = layer.animation_time_steps(scaled_delta_time, policy);
            if !self.track_view_exclusive {
                layer.synchronize_parametric_timewarp_groups(index, &mut steps, policy);
                layer.adjust_time_warped_animation_steps(&mut steps);
            }
            layer.update_animation_times(index, &steps, self.track_view_exclusive);
            layer.update_layer_blend(index, delta_time);
        }
    }

    pub fn active_instances(&self) -> impl Iterator<Item = &TransitionAnimation<K, S>> {
        self.layers
            .iter()
            .flat_map(AnimationTransitionQueue::active_animations)
    }

    pub fn effective_weight(&self, layer: u32, animation: &TransitionAnimation<K, S>) -> f32 {
        self.layers.get(layer as usize).map_or(0.0, |queue| {
            animation.effective_weight(queue.layer_weight(layer as usize))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    struct TwoSegments;

    impl<K, S> AnimationTransitionPolicy<K, S> for TwoSegments {
        fn is_ready(&self, _animation: &K) -> bool {
            true
        }

        fn animation_time_step(
            &mut self,
            _animation: &mut TransitionAnimation<K, S>,
            delta_time: f32,
        ) -> Option<AnimationTimeStep> {
            Some(AnimationTimeStep {
                normalized_delta: delta_time / 0.5,
                expected_segment_duration: 0.5,
                expected_total_duration: 1.0,
                segment_count: 2,
            })
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct GroupedTransitions;

    impl AnimationTransitionPolicy<&'static str> for GroupedTransitions {
        fn is_ready(&self, _animation: &&'static str) -> bool {
            true
        }

        fn shares_timewarp_group(
            &self,
            previous: &TransitionAnimation<&'static str>,
            next: &TransitionAnimation<&'static str>,
        ) -> bool {
            previous.animation().eq_ignore_ascii_case(next.animation())
        }
    }

    fn parameters(layer: u32) -> CharacterAnimationParameters {
        CharacterAnimationParameters {
            layer,
            transition_time: 0.2,
            expected_duration: 1.0,
            ..Default::default()
        }
    }

    fn runtime<K: PartialEq>() -> CharacterAnimationRuntime<K> {
        CharacterAnimationRuntime::default()
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "Cry clamps the normalized time to exactly 1.0 when a non-looping clip \
                  repeats, so the exact value is the property under test"
    )]
    fn upper_layer_once_animation_repeats_then_fades() {
        let mut runtime = runtime();
        runtime.start_animation(7_u32, parameters(1)).unwrap();

        runtime.update(1.0, &mut ReadyAnimationTransitions);

        let animation = runtime.layer(1).unwrap().animations().first().unwrap();
        assert!(animation.flags().contains(AnimationFlags::REPEAT_LAST_KEY));
        assert!(animation.flags().contains(AnimationFlags::FADE_OUT));
        assert_eq!(animation.normalized_time(), 1.0);
        assert!(!runtime.layer(1).unwrap().is_active());
    }

    #[test]
    fn transition_weights_use_cry_normalized_curve() {
        let mut runtime = runtime();
        let mut first = parameters(0);
        first.flags = AnimationFlags::ALLOW_ANIMATION_RESTART | AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation(1_u32, first).unwrap();
        runtime.update(0.0, &mut ReadyAnimationTransitions);
        runtime.start_animation(2_u32, parameters(0)).unwrap();

        runtime.update(0.1, &mut ReadyAnimationTransitions);

        let animations = runtime.layer(0).unwrap().animations();
        assert_eq!(animations.len(), 2);
        assert!((animations[0].transition_weight() - 0.5).abs() < 0.0001);
        assert!((animations[1].transition_weight() - 0.5).abs() < 0.0001);
    }

    #[test]
    fn root_stop_clears_but_upper_stop_blends_out() {
        let mut runtime = runtime();
        runtime.start_animation(1_u32, parameters(0)).unwrap();
        runtime.start_animation(2_u32, parameters(1)).unwrap();

        assert!(runtime.stop_animation(0, 0.5));
        assert!(runtime.stop_animation(1, 0.5));

        assert!(runtime.layer(0).unwrap().animations().is_empty());
        assert_eq!(runtime.layer(1).unwrap().animations().len(), 1);
    }

    #[test]
    fn token_owned_animation_stops_only_when_it_is_top_of_the_fifo() {
        let mut runtime = runtime();
        let mut first = parameters(1);
        first.user_token = 10;
        runtime.start_animation(1_u32, first).unwrap();
        let mut second = parameters(1);
        second.user_token = 20;
        runtime.start_animation(2_u32, second).unwrap();

        assert!(runtime.stop_animation_if_user_token_is_top(1, 10, 0.5));
        assert!(runtime.layer(1).unwrap().is_active());
        assert!(runtime.stop_animation_if_user_token_is_top(1, 20, 0.5));
        assert!(!runtime.layer(1).unwrap().is_active());
    }

    #[test]
    fn reverse_playback_is_rejected() {
        let mut runtime = runtime();
        let mut reverse = parameters(0);
        reverse.playback_speed = -1.0;

        assert_eq!(
            runtime.start_animation(1_u32, reverse),
            Err(StartAnimationError::ReversePlayback)
        );
    }

    /// Inside Cry's contract
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_BlendMan.cpp:349`),
    /// normalized time stays `<= 2.0f`. We unwind exactly once, matching
    /// `++m_currentSegmentIndex[idx]` at line 443.
    #[test]
    fn single_crossed_cycle_matches_cry_exactly() {
        let mut runtime = runtime();
        let mut looping = parameters(0);
        looping.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation(1_u32, looping).unwrap();

        runtime.update(1.25, &mut ReadyAnimationTransitions);

        let animation = runtime.layer(0).unwrap().animations().first().unwrap();
        assert_eq!(animation.segment_advances_this_update(), 1);
        assert_eq!(animation.loops_this_update(), 1);
        assert_eq!(animation.segment_index(), 0);
        assert!((animation.normalized_time() - 0.25).abs() < 0.0001);
    }

    /// Past one wrap we deliberately diverge: Cry steps the segment counter once
    /// and discards the rest, so the extra cycles never reach `ParseLayer0`
    /// (`m_nEOC` is pinned to -1/0/+1, `SkeletonAnim_Locator.cpp:294`) or
    /// `AnimCallback` (one `[prev,1] + [0,new]` pair,
    /// `SkeletonAnim_BlendMan.cpp:758`). We keep unwinding and record the count.
    #[test]
    fn looping_animation_records_every_crossed_cycle() {
        let mut runtime = runtime();
        let mut looping = parameters(0);
        looping.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation(1_u32, looping).unwrap();

        runtime.update(2.25, &mut ReadyAnimationTransitions);

        let animation = runtime.layer(0).unwrap().animations().first().unwrap();
        assert_eq!(animation.loops_this_update(), 2);
        assert_eq!(animation.segment_advances_this_update(), 2);
        assert!((animation.normalized_time() - 0.25).abs() < 0.0001);
    }

    /// The same divergence on a multi-segment clip: Cry would leave the segment
    /// index at 1 with the clip a whole segment behind the clock, we land on the
    /// segment the elapsed time actually selects.
    #[test]
    fn segmented_looping_animation_crosses_every_segment_boundary() {
        let mut runtime = runtime();
        let mut looping = parameters(0);
        looping.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation(1_u32, looping).unwrap();

        runtime.update(1.25, &mut TwoSegments);

        let animation = runtime.layer(0).unwrap().animations().first().unwrap();
        assert_eq!(animation.segment_advances_this_update(), 2);
        assert_eq!(animation.loops_this_update(), 1);
        assert_eq!(animation.segment_index(), 0);
        assert_eq!(animation.previous_segment_index(), 0);
        assert!((animation.normalized_time() - 0.5).abs() < 0.0001);
    }

    #[test]
    fn segmented_animation_advances_phase_before_completing_the_clip() {
        let mut runtime = runtime();
        runtime.start_animation(1_u32, parameters(0)).unwrap();

        runtime.update(0.75, &mut TwoSegments);

        let animation = runtime.layer(0).unwrap().animations().first().unwrap();
        assert_eq!(animation.segment_index(), 1);
        assert!((animation.normalized_time() - 0.5).abs() < 0.0001);
        assert!(!animation.has_repeated());
    }

    #[test]
    fn matching_parametric_groups_synchronize_on_upper_layers() {
        let mut runtime = runtime();
        let mut previous = parameters(1);
        previous.key_time = 0.75;
        previous.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation("Turn", previous).unwrap();
        runtime.update(0.0, &mut GroupedTransitions);

        let mut current = parameters(1);
        current.key_time = 0.25;
        current.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation("turn", current).unwrap();
        runtime.update(0.0, &mut GroupedTransitions);

        let animations = runtime.layer(1).unwrap().animations();
        assert!((animations[0].normalized_time() - 0.75).abs() < 0.0001);
        assert!((animations[1].normalized_time() - 0.75).abs() < 0.0001);
    }

    #[test]
    fn parametric_groups_do_not_synchronize_on_the_root_layer() {
        let mut runtime = runtime();
        let mut previous = parameters(0);
        previous.key_time = 0.75;
        previous.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation("Turn", previous).unwrap();
        runtime.update(0.0, &mut GroupedTransitions);

        let mut current = parameters(0);
        current.key_time = 0.25;
        current.flags = AnimationFlags::LOOP_ANIMATION;
        runtime.start_animation("turn", current).unwrap();
        runtime.update(0.0, &mut GroupedTransitions);

        let animations = runtime.layer(0).unwrap().animations();
        assert!((animations[1].normalized_time() - 0.25).abs() < 0.0001);
    }

    #[test]
    fn top_animation_flags_modify_only_the_newest_active_entry() {
        let mut runtime = runtime();
        runtime.start_animation(1_u32, parameters(0)).unwrap();
        runtime.update(0.0, &mut ReadyAnimationTransitions);
        runtime.start_animation(2_u32, parameters(0)).unwrap();
        runtime.update(0.0, &mut ReadyAnimationTransitions);

        assert!(runtime.set_top_animation_flags(0, AnimationFlags::FULL_ROOT_PRIORITY));

        let animations = runtime.layer(0).unwrap().animations();
        assert!(
            !animations[0]
                .flags()
                .contains(AnimationFlags::FULL_ROOT_PRIORITY)
        );
        assert!(
            animations[1]
                .flags()
                .contains(AnimationFlags::FULL_ROOT_PRIORITY)
        );
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the ninth entry must not be advanced at all, so its normalized time has \
                  to stay bit-identical to the 0.0 it was queued with"
    )]
    fn only_the_first_eight_fifo_entries_execute() {
        let mut runtime = runtime();
        for animation_id in 0..9_u32 {
            let mut animation_parameters = parameters(0);
            animation_parameters.flags = AnimationFlags::LOOP_ANIMATION;
            runtime
                .start_animation(animation_id, animation_parameters)
                .unwrap();
        }

        runtime.update(0.1, &mut ReadyAnimationTransitions);

        let queue = runtime.layer(0).unwrap();
        assert_eq!(queue.animations().len(), 9);
        assert_eq!(queue.executed_animations().len(), 8);
        assert_eq!(queue.active_animations().len(), 8);
        assert!(!queue.animations()[8].is_activated());
        assert_eq!(queue.animations()[8].normalized_time(), 0.0);
        assert_eq!(runtime.active_instances().count(), 8);
    }
}
