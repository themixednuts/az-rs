//! Reflected version-1 simple animation component data.

use az_animation::simple::{
    DEFAULT_TRANSITION_TIME, InvalidSimpleAnimationLayerId, SimpleAnimationComponentRuntime,
    SimpleAnimationLayerId, SimpleAnimationRequest,
};
use az_core::component::Component as AzComponent;
use az_derive::{AzComponent, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const ANIMATED_LAYER_TYPE_ID: Uuid = uuid!("147EAB48-2D6E-41CF-8414-CEABF3F1E59B");
pub const SIMPLE_ANIMATION_COMPONENT_TYPE_ID: Uuid = uuid!("FBB470EF-2288-4B62-B41F-D830DD4C5B98");

#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "AnimatedLayer", ANIMATED_LAYER_TYPE_ID)]
pub struct AnimatedLayer {
    #[serde(rename = "Animation Name", default)]
    pub animation_name: String,
    #[serde(rename = "Layer Id", default)]
    pub layer_id: i32,
    #[serde(rename = "Looping", default)]
    pub looping: bool,
    #[serde(rename = "Playback Speed", default)]
    pub playback_speed: f32,
    #[serde(rename = "Layer Weight", default)]
    pub layer_weight: f32,
    #[serde(rename = "AnimDrivenMotion", default)]
    pub animation_driven_motion: bool,
}

impl Default for AnimatedLayer {
    fn default() -> Self {
        Self {
            animation_name: String::new(),
            layer_id: 0,
            looping: true,
            playback_speed: 1.0,
            layer_weight: 1.0,
            animation_driven_motion: false,
        }
    }
}

impl AnimatedLayer {
    #[must_use]
    pub fn animation_name(&self) -> Option<&str> {
        let name = self.animation_name.trim();
        (!name.is_empty()).then_some(name)
    }

    #[must_use]
    pub fn layer_index(&self) -> Option<usize> {
        usize::try_from(self.layer_id).ok()
    }

    /// Turns this authored layer into a playback request.
    ///
    /// # Errors
    ///
    /// [`AnimatedLayerResolveError::InvalidLayer`] when `layer_id` is outside
    /// the simple-animation layer range, or
    /// [`AnimatedLayerResolveError::Animation`] wrapping whatever
    /// `resolve_animation` returned for `animation_name`.
    pub fn resolve<K, E>(
        &self,
        resolve_animation: impl FnOnce(&str) -> Result<K, E>,
    ) -> Result<SimpleAnimationRequest<K>, AnimatedLayerResolveError<E>> {
        let layer = SimpleAnimationLayerId::try_from(self.layer_id)
            .map_err(AnimatedLayerResolveError::InvalidLayer)?;
        let animation = resolve_animation(&self.animation_name)
            .map_err(AnimatedLayerResolveError::Animation)?;

        Ok(SimpleAnimationRequest {
            animation,
            layer,
            looping: self.looping,
            playback_speed: self.playback_speed,
            transition_time: DEFAULT_TRANSITION_TIME,
            interrupt_if_already_playing: false,
            layer_weight: self.layer_weight,
            animation_driven_root_motion: self.animation_driven_motion,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnimatedLayerResolveError<E> {
    #[error(transparent)]
    InvalidLayer(InvalidSimpleAnimationLayerId),
    #[error("failed to resolve simple-animation clip")]
    Animation(#[source] E),
}

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
#[az_component(name = "SimpleAnimationComponent", SIMPLE_ANIMATION_COMPONENT_TYPE_ID)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.SimpleAnimationComponent", version = 1)]
pub struct SimpleAnimationComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Hide Until Animated", default)]
    pub hide_until_animated: bool,
    #[serde(rename = "Playback Entries", default)]
    pub playback_entries: Vec<AnimatedLayer>,
}

impl SimpleAnimationComponent {
    /// Builds the runtime defaults for every authored playback entry.
    ///
    /// # Errors
    ///
    /// Returns the first error [`AnimatedLayer::resolve`] produces for an entry
    /// of `playback_entries`, so
    /// [`AnimatedLayerResolveError::InvalidLayer`] for an out-of-range layer id
    /// and [`AnimatedLayerResolveError::Animation`] for a clip
    /// `resolve_animation` rejects.
    pub fn resolve_runtime<K, E>(
        &self,
        mut resolve_animation: impl FnMut(&str) -> Result<K, E>,
    ) -> Result<SimpleAnimationComponentRuntime<K>, AnimatedLayerResolveError<E>> {
        let mut runtime = SimpleAnimationComponentRuntime::new(self.hide_until_animated);
        for layer in &self.playback_entries {
            runtime.set_default(layer.resolve(&mut resolve_animation)?);
        }
        Ok(runtime)
    }
}
