//! Renderer-independent Lumberyard Simple Animation runtime.

use std::array;

use bitflags::bitflags;

use crate::playback::AnimationFlags;

/// Cry's Simple Animation component owns exactly sixteen animation layers.
pub const SIMPLE_ANIMATION_LAYER_COUNT: usize = 16;
pub const DEFAULT_TRANSITION_TIME: f32 = 0.15;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimpleAnimationLayerId(u8);

impl SimpleAnimationLayerId {
    pub const BASE: Self = Self(0);

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        self.0 as i32
    }
}

impl TryFrom<i32> for SimpleAnimationLayerId {
    type Error = InvalidSimpleAnimationLayerId;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let index = u8::try_from(value).map_err(|_| InvalidSimpleAnimationLayerId(value))?;
        (usize::from(index) < SIMPLE_ANIMATION_LAYER_COUNT)
            .then_some(Self(index))
            .ok_or(InvalidSimpleAnimationLayerId(value))
    }
}

impl TryFrom<usize> for SimpleAnimationLayerId {
    type Error = InvalidSimpleAnimationLayerId;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        let native = i32::try_from(value).map_err(|_| InvalidSimpleAnimationLayerId(i32::MAX))?;
        Self::try_from(native)
    }
}

impl From<SimpleAnimationLayerId> for usize {
    fn from(value: SimpleAnimationLayerId) -> Self {
        value.index()
    }
}

impl From<SimpleAnimationLayerId> for i32 {
    fn from(value: SimpleAnimationLayerId) -> Self {
        value.native_value()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("simple-animation layer {0} is outside 0..16")]
pub struct InvalidSimpleAnimationLayerId(pub i32);

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct SimpleAnimationLayerMask: u16 {
        const ALL = u16::MAX;
    }

}

/// Simple Animation's domain name for the shared Cry playback flags.
pub type SimpleAnimationPlaybackFlags = AnimationFlags;

impl From<SimpleAnimationLayerId> for SimpleAnimationLayerMask {
    fn from(value: SimpleAnimationLayerId) -> Self {
        Self::from_bits_retain(1_u16 << value.index())
    }
}

/// Public result values and discriminants match the shipping request bus.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SimpleAnimationResult {
    #[default]
    Success = 0,
    SuccessWithErrors = 1,
    Failure = 2,
    AnimationAlreadyPlaying = 3,
    AnimationNotFound = 4,
    NoAnimationPlayingOnLayer = 5,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleAnimationPlaybackParameters {
    pub layer: SimpleAnimationLayerId,
    pub playback_speed: f32,
    pub transition_time: f32,
    pub playback_weight: f32,
    pub allow_multilayer_animation: bool,
    pub flags: SimpleAnimationPlaybackFlags,
}

/// A resolved animation request. `K` is the runtime clip key selected while cooking.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleAnimationRequest<K> {
    pub animation: K,
    pub layer: SimpleAnimationLayerId,
    pub looping: bool,
    pub playback_speed: f32,
    pub transition_time: f32,
    pub interrupt_if_already_playing: bool,
    pub layer_weight: f32,
    pub animation_driven_root_motion: bool,
}

impl<K> SimpleAnimationRequest<K> {
    #[must_use]
    pub const fn playback_parameters(&self) -> SimpleAnimationPlaybackParameters {
        let playback_mode = if self.looping {
            SimpleAnimationPlaybackFlags::LOOP_ANIMATION
        } else {
            SimpleAnimationPlaybackFlags::from_bits_retain(
                SimpleAnimationPlaybackFlags::REPEAT_LAST_KEY.bits()
                    | SimpleAnimationPlaybackFlags::FADE_OUT.bits(),
            )
        };
        let flags = SimpleAnimationPlaybackFlags::from_bits_retain(
            playback_mode.bits()
                | SimpleAnimationPlaybackFlags::ALLOW_ANIMATION_RESTART.bits()
                | SimpleAnimationPlaybackFlags::FORCE_TRANSITION_TO_ANIMATION.bits(),
        );

        SimpleAnimationPlaybackParameters {
            layer: self.layer,
            playback_speed: self.playback_speed,
            transition_time: self.transition_time,
            playback_weight: self.layer_weight,
            allow_multilayer_animation: true,
            flags,
        }
    }
}

impl<K: PartialEq> SimpleAnimationRequest<K> {
    /// Reproduces Lumberyard's `AnimatedLayer::operator==`, including its
    /// inverted approximate-float comparisons.
    ///
    /// Lumberyard returns false when each float is close. A request therefore counts as
    /// already playing only when all three float deltas are strictly greater than
    /// `f32::EPSILON`.
    #[must_use]
    pub fn shipping_already_playing_match(&self, other: &Self) -> bool {
        self.animation == other.animation
            && self.looping == other.looping
            && (self.playback_speed - other.playback_speed).abs() > f32::EPSILON
            && (self.transition_time - other.transition_time).abs() > f32::EPSILON
            && self.layer == other.layer
            && (self.layer_weight - other.layer_weight).abs() > f32::EPSILON
    }
}

/// Operations supplied by a concrete animation runtime such as Bevy's animation graph.
///
/// Request data stays borrowed across the boundary. The animator clones it only after a
/// successful start, because Cry retains one active request per occupied layer.
pub trait SimpleAnimationBackend<K> {
    fn has_character(&self) -> bool;
    fn is_under_cinematic_control(&self) -> bool;
    fn is_character_visible(&self) -> bool;
    fn set_entity_visible(&mut self, visible: bool);
    fn has_animation(&self, animation: &K) -> bool;
    fn start_animation(
        &mut self,
        animation: &K,
        parameters: SimpleAnimationPlaybackParameters,
    ) -> bool;
    fn stop_animation(&mut self, layer: SimpleAnimationLayerId, blend_out_time: f32) -> bool;
    fn stop_all_animations(&mut self);
    fn set_layer_playback_speed(&mut self, layer: SimpleAnimationLayerId, speed: f32);
    fn set_layer_weight(&mut self, layer: SimpleAnimationLayerId, weight: f32);
    fn layer_normalized_time(&self, layer: SimpleAnimationLayerId) -> f32;
    fn set_animation_driven_motion(&mut self, enabled: bool);

    fn animation_started(&mut self, _request: &SimpleAnimationRequest<K>) {}
    fn animation_stopped(&mut self, _layer: SimpleAnimationLayerId) {}
}

#[derive(Debug, Clone)]
pub struct SimpleAnimator<K> {
    active_layers: [Option<SimpleAnimationRequest<K>>; SIMPLE_ANIMATION_LAYER_COUNT],
    hidden_until_animated: bool,
}

impl<K> Default for SimpleAnimator<K> {
    fn default() -> Self {
        Self {
            active_layers: array::from_fn(|_| None),
            hidden_until_animated: false,
        }
    }
}

impl<K> SimpleAnimator<K> {
    #[must_use]
    pub const fn active_layer(
        &self,
        layer: SimpleAnimationLayerId,
    ) -> Option<&SimpleAnimationRequest<K>> {
        self.active_layers[layer.index()].as_ref()
    }

    #[must_use]
    pub const fn is_layer_active(&self, layer: SimpleAnimationLayerId) -> bool {
        self.active_layer(layer).is_some()
    }

    #[must_use]
    pub fn active_layer_count(&self) -> usize {
        self.active_layers.iter().flatten().count()
    }

    #[must_use]
    pub const fn is_hidden_until_animated(&self) -> bool {
        self.hidden_until_animated
    }

    /// Clears every active layer and reports each stop to `backend`.
    ///
    /// # Panics
    ///
    /// Panics if an index into the fixed-size layer array is not a valid
    /// [`SimpleAnimationLayerId`], which cannot happen because the array is
    /// exactly `SIMPLE_ANIMATION_LAYER_COUNT` long.
    pub fn detach(&mut self, backend: &mut impl SimpleAnimationBackend<K>) {
        for (index, active) in self.active_layers.iter_mut().enumerate() {
            if active.take().is_some() {
                let layer = SimpleAnimationLayerId::try_from(index).expect("fixed layer index");
                backend.animation_stopped(layer);
            }
        }
        self.hidden_until_animated = false;
    }

    pub fn tick(&mut self, backend: &mut impl SimpleAnimationBackend<K>) {
        if self.hidden_until_animated && backend.has_character() && backend.is_character_visible() {
            self.hidden_until_animated = false;
            backend.set_entity_visible(true);
        }

        if !backend.has_character() {
            return;
        }

        for index in 0..SIMPLE_ANIMATION_LAYER_COUNT {
            let Some(active) = self.active_layers[index].as_ref() else {
                continue;
            };
            if active.looping {
                continue;
            }

            let layer = active.layer;
            if (backend.layer_normalized_time(layer) - 1.0).abs() < f32::EPSILON {
                // The native tick path intentionally does not reset root motion here.
                let _ = backend.stop_animation(layer, 0.0);
                self.active_layers[index] = None;
                backend.animation_stopped(layer);
            }
        }
    }
}

impl<K: Clone + PartialEq> SimpleAnimator<K> {
    pub fn start_animation(
        &mut self,
        request: &SimpleAnimationRequest<K>,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() || backend.is_under_cinematic_control() {
            return SimpleAnimationResult::Failure;
        }

        if let Some(active) = self.active_layer(request.layer)
            && !request.interrupt_if_already_playing
            && active.shipping_already_playing_match(request)
        {
            return SimpleAnimationResult::AnimationAlreadyPlaying;
        }

        if !backend.has_animation(&request.animation) {
            return SimpleAnimationResult::AnimationNotFound;
        }

        if !backend.start_animation(&request.animation, request.playback_parameters()) {
            return SimpleAnimationResult::Failure;
        }

        // `AZStd::unordered_map::insert` does not replace an occupied layer. Preserve that
        // shipped behavior while still notifying with the newly requested animation.
        if self.active_layers[request.layer.index()].is_none() {
            self.active_layers[request.layer.index()] = Some(request.clone());
        }
        backend.animation_started(request);

        if request.layer == SimpleAnimationLayerId::BASE {
            backend.set_animation_driven_motion(request.animation_driven_root_motion);
        }

        SimpleAnimationResult::Success
    }

    pub fn start_animations<'a>(
        &mut self,
        requests: impl IntoIterator<Item = &'a SimpleAnimationRequest<K>>,
        hide_until_animated: bool,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult
    where
        K: 'a,
    {
        if hide_until_animated && backend.has_character() && !backend.is_character_visible() {
            backend.set_entity_visible(false);
            self.hidden_until_animated = true;
        }

        let mut request_count = 0_usize;
        let mut failure_count = 0_usize;
        for request in requests {
            request_count += 1;
            failure_count += usize::from(
                self.start_animation(request, backend) != SimpleAnimationResult::Success,
            );
        }
        aggregate_results(request_count, failure_count)
    }

    pub fn stop_animation(
        &mut self,
        layer: SimpleAnimationLayerId,
        blend_out_time: f32,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() || backend.is_under_cinematic_control() {
            return SimpleAnimationResult::Failure;
        }

        let Some(active) = self.active_layer(layer) else {
            return SimpleAnimationResult::NoAnimationPlayingOnLayer;
        };

        if layer == SimpleAnimationLayerId::BASE && active.animation_driven_root_motion {
            backend.set_animation_driven_motion(false);
        }

        if !backend.stop_animation(layer, blend_out_time) {
            return SimpleAnimationResult::Failure;
        }

        self.active_layers[layer.index()] = None;
        backend.animation_stopped(layer);
        SimpleAnimationResult::Success
    }

    /// Stops every layer selected by `layers`, aggregating the per-layer
    /// results the way the shipping implementation does.
    ///
    /// # Panics
    ///
    /// Panics if an index into the fixed-size layer array is not a valid
    /// [`SimpleAnimationLayerId`], which cannot happen because the loop is
    /// bounded by `SIMPLE_ANIMATION_LAYER_COUNT`.
    pub fn stop_animations(
        &mut self,
        layers: SimpleAnimationLayerMask,
        blend_out_time: f32,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() {
            return SimpleAnimationResult::Failure;
        }

        let mut failure_count = 0_usize;
        for index in 0..SIMPLE_ANIMATION_LAYER_COUNT {
            let layer = SimpleAnimationLayerId::try_from(index).expect("fixed layer index");
            if layers.contains(layer.into()) {
                failure_count += usize::from(
                    self.stop_animation(layer, blend_out_time, backend)
                        != SimpleAnimationResult::Success,
                );
            }
        }
        aggregate_results(layers.bits().count_ones() as usize, failure_count)
    }

    pub fn stop_all_animations(
        &mut self,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() || backend.is_under_cinematic_control() {
            return SimpleAnimationResult::Failure;
        }

        backend.stop_all_animations();
        self.detach(backend);
        SimpleAnimationResult::Success
    }

    pub fn set_playback_speed(
        &self,
        layer: SimpleAnimationLayerId,
        speed: f32,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() || !self.is_layer_active(layer) {
            return SimpleAnimationResult::Failure;
        }
        backend.set_layer_playback_speed(layer, speed);
        SimpleAnimationResult::Success
    }

    pub fn set_playback_weight(
        &self,
        layer: SimpleAnimationLayerId,
        weight: f32,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if !backend.has_character() || !self.is_layer_active(layer) {
            return SimpleAnimationResult::Failure;
        }
        backend.set_layer_weight(layer, weight);
        SimpleAnimationResult::Success
    }
}

#[derive(Debug, Clone)]
pub struct SimpleAnimationComponentRuntime<K> {
    default_layers: [Option<SimpleAnimationRequest<K>>; SIMPLE_ANIMATION_LAYER_COUNT],
    queued_before_asset_ready: Vec<SimpleAnimationRequest<K>>,
    animator: SimpleAnimator<K>,
    mesh_asset_ready: bool,
    hide_until_animated: bool,
}

impl<K> SimpleAnimationComponentRuntime<K> {
    #[must_use]
    pub fn new(hide_until_animated: bool) -> Self {
        Self {
            default_layers: array::from_fn(|_| None),
            queued_before_asset_ready: Vec::new(),
            animator: SimpleAnimator::default(),
            mesh_asset_ready: false,
            hide_until_animated,
        }
    }

    #[must_use]
    pub const fn animator(&self) -> &SimpleAnimator<K> {
        &self.animator
    }

    #[must_use]
    pub const fn animator_mut(&mut self) -> &mut SimpleAnimator<K> {
        &mut self.animator
    }

    #[must_use]
    pub const fn is_mesh_asset_ready(&self) -> bool {
        self.mesh_asset_ready
    }

    #[must_use]
    pub const fn queued_request_count(&self) -> usize {
        self.queued_before_asset_ready.len()
    }

    pub fn set_default(&mut self, request: SimpleAnimationRequest<K>) {
        let layer = &mut self.default_layers[request.layer.index()];
        if layer.is_none() {
            *layer = Some(request);
        }
    }

    pub const fn on_mesh_destroyed(&mut self) {
        self.mesh_asset_ready = false;
    }
}

impl<K: Clone + PartialEq> SimpleAnimationComponentRuntime<K> {
    pub fn start_default_animations(
        &mut self,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        let requests = self.default_layers.iter().flatten();
        self.animator
            .start_animations(requests, self.hide_until_animated, backend)
    }

    pub fn start_animation(
        &mut self,
        request: &SimpleAnimationRequest<K>,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult {
        if self.mesh_asset_ready {
            self.animator.start_animation(request, backend)
        } else {
            self.queued_before_asset_ready.push(request.clone());
            SimpleAnimationResult::SuccessWithErrors
        }
    }

    pub fn start_animation_set<'a>(
        &mut self,
        requests: impl IntoIterator<Item = &'a SimpleAnimationRequest<K>>,
        backend: &mut impl SimpleAnimationBackend<K>,
    ) -> SimpleAnimationResult
    where
        K: 'a,
    {
        self.animator
            .start_animations(requests, self.hide_until_animated, backend)
    }

    pub fn on_mesh_created(&mut self, backend: &mut impl SimpleAnimationBackend<K>) {
        self.mesh_asset_ready = true;
        let _ = self.start_default_animations(backend);

        let queued = std::mem::take(&mut self.queued_before_asset_ready);
        for request in &queued {
            let _ = self.start_animation(request, backend);
        }
    }
}

const fn aggregate_results(request_count: usize, failure_count: usize) -> SimpleAnimationResult {
    if failure_count == 0 {
        SimpleAnimationResult::Success
    } else if failure_count < request_count {
        SimpleAnimationResult::SuccessWithErrors
    } else {
        SimpleAnimationResult::Failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Backend {
        character: bool,
        cinematic: bool,
        character_visible: bool,
        known: Vec<u32>,
        normalized: [f32; SIMPLE_ANIMATION_LAYER_COUNT],
        starts: Vec<(u32, SimpleAnimationPlaybackParameters)>,
        stops: Vec<(SimpleAnimationLayerId, f32)>,
        notifications: Vec<(bool, SimpleAnimationLayerId)>,
        entity_visibility: Vec<bool>,
        root_motion: Vec<bool>,
    }

    impl SimpleAnimationBackend<u32> for Backend {
        fn has_character(&self) -> bool {
            self.character
        }

        fn is_under_cinematic_control(&self) -> bool {
            self.cinematic
        }

        fn is_character_visible(&self) -> bool {
            self.character_visible
        }

        fn set_entity_visible(&mut self, visible: bool) {
            self.entity_visibility.push(visible);
        }

        fn has_animation(&self, animation: &u32) -> bool {
            self.known.contains(animation)
        }

        fn start_animation(
            &mut self,
            animation: &u32,
            parameters: SimpleAnimationPlaybackParameters,
        ) -> bool {
            self.starts.push((*animation, parameters));
            true
        }

        fn stop_animation(&mut self, layer: SimpleAnimationLayerId, blend_out_time: f32) -> bool {
            self.stops.push((layer, blend_out_time));
            true
        }

        fn stop_all_animations(&mut self) {}

        fn set_layer_playback_speed(&mut self, _layer: SimpleAnimationLayerId, _speed: f32) {}

        fn set_layer_weight(&mut self, _layer: SimpleAnimationLayerId, _weight: f32) {}

        fn layer_normalized_time(&self, layer: SimpleAnimationLayerId) -> f32 {
            self.normalized[layer.index()]
        }

        fn set_animation_driven_motion(&mut self, enabled: bool) {
            self.root_motion.push(enabled);
        }

        fn animation_started(&mut self, request: &SimpleAnimationRequest<u32>) {
            self.notifications.push((true, request.layer));
        }

        fn animation_stopped(&mut self, layer: SimpleAnimationLayerId) {
            self.notifications.push((false, layer));
        }
    }

    fn request(animation: u32, layer: i32) -> SimpleAnimationRequest<u32> {
        SimpleAnimationRequest {
            animation,
            layer: SimpleAnimationLayerId::try_from(layer).unwrap(),
            looping: false,
            playback_speed: 1.0,
            transition_time: DEFAULT_TRANSITION_TIME,
            interrupt_if_already_playing: false,
            layer_weight: 1.0,
            animation_driven_root_motion: layer == 0,
        }
    }

    #[test]
    fn playback_flags_match_shipping_values() {
        let looping = SimpleAnimationRequest {
            looping: true,
            ..request(7, 0)
        };
        assert_eq!(looping.playback_parameters().flags.bits(), 0x0000_8102);
        assert_eq!(
            request(7, 0).playback_parameters().flags.bits(),
            0x4000_8104
        );
    }

    #[test]
    fn shipping_equality_quirk_is_explicit() {
        let active = request(7, 0);
        assert!(!active.shipping_already_playing_match(&active));

        let different_floats = SimpleAnimationRequest {
            playback_speed: 2.0,
            transition_time: 0.25,
            layer_weight: 0.5,
            ..active
        };
        assert!(active.shipping_already_playing_match(&different_floats));
    }

    #[test]
    fn start_validates_backend_and_retains_first_layer_descriptor() {
        let mut backend = Backend {
            character: true,
            known: vec![7, 8],
            ..Default::default()
        };
        let mut animator = SimpleAnimator::default();
        let first = request(7, 0);
        let replacement = SimpleAnimationRequest {
            animation: 8,
            interrupt_if_already_playing: true,
            ..request(8, 0)
        };

        assert_eq!(
            animator.start_animation(&first, &mut backend),
            SimpleAnimationResult::Success
        );
        assert_eq!(
            animator.start_animation(&replacement, &mut backend),
            SimpleAnimationResult::Success
        );
        assert_eq!(
            animator.active_layer(SimpleAnimationLayerId::BASE),
            Some(&first)
        );
        assert_eq!(backend.root_motion, vec![true, true]);
    }

    #[test]
    fn component_queues_until_mesh_created_then_replays() {
        let mut backend = Backend {
            character: true,
            known: vec![7],
            ..Default::default()
        };
        let mut runtime = SimpleAnimationComponentRuntime::new(false);
        let request = request(7, 2);

        assert_eq!(
            runtime.start_animation(&request, &mut backend),
            SimpleAnimationResult::SuccessWithErrors
        );
        assert_eq!(runtime.queued_request_count(), 1);

        runtime.on_mesh_created(&mut backend);
        assert_eq!(runtime.queued_request_count(), 0);
        assert!(runtime.animator().is_layer_active(request.layer));
    }

    #[test]
    fn hide_until_first_character_visible_and_one_shot_completion_match_shipping() {
        let mut backend = Backend {
            character: true,
            character_visible: false,
            known: vec![7],
            ..Default::default()
        };
        let mut animator = SimpleAnimator::default();
        let request = request(7, 1);

        assert_eq!(
            animator.start_animations([&request], true, &mut backend),
            SimpleAnimationResult::Success
        );
        assert_eq!(backend.entity_visibility, vec![false]);

        backend.character_visible = true;
        backend.normalized[request.layer.index()] = 1.0;
        animator.tick(&mut backend);

        assert_eq!(backend.entity_visibility, vec![false, true]);
        assert!(!animator.is_layer_active(request.layer));
        assert_eq!(backend.stops, vec![(request.layer, 0.0)]);
    }
}
