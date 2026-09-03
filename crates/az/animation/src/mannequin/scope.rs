//! Mannequin scope sequencing, independent of the animation renderer.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::playback::AnimationFlags;

use super::{
    ActionEndMethod, ActionHandle, BlendQuery, BlendQueryFlags, FragmentId, FragmentSelection,
    FragmentTagState, ResumeFlags, ScopeId, TagState,
};

pub const FRAGMENT_PART_COUNT: usize = 3;
const MAX_QUEUED_TIME_INCREMENTS: usize = 5;

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendCurve {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClipBlend {
    pub exit_time: f32,
    pub start_time: f32,
    pub duration: f32,
    pub flags: AnimationFlags,
    pub curve: BlendCurve,
    pub terminal: bool,
}

impl Default for ClipBlend {
    fn default() -> Self {
        Self {
            exit_time: 0.0,
            start_time: 0.0,
            duration: 0.2,
            flags: AnimationFlags::empty(),
            curve: BlendCurve::Linear,
            terminal: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationEntry<K> {
    /// `None` is Cry's explicit empty animation entry and stops the layer.
    pub animation: Option<K>,
    pub flags: AnimationFlags,
    pub playback_speed: f32,
    pub playback_weight: f32,
    pub blend_channels: [f32; 4],
    /// Cry's authored joint-mask list index (`0` means no mask).
    pub weight_list: u8,
}

impl<K> Default for AnimationEntry<K> {
    fn default() -> Self {
        Self {
            animation: None,
            flags: AnimationFlags::empty(),
            playback_speed: 1.0,
            playback_weight: 1.0,
            blend_channels: [0.0; 4],
            weight_list: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip<K> {
    pub blend: ClipBlend,
    pub animation: AnimationEntry<K>,
    pub reference_length: f32,
    pub blend_part: u8,
    pub part: u8,
    pub variable_length: bool,
}

impl<K> Default for AnimationClip<K> {
    fn default() -> Self {
        Self {
            blend: ClipBlend::default(),
            animation: AnimationEntry::default(),
            reference_length: 0.0,
            blend_part: 0,
            part: 0,
            variable_length: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralEntry<P> {
    /// `None` is the explicit empty procedural clip used to blend a layer out.
    pub parameters: Option<P>,
    pub blend: ClipBlend,
    pub blend_part: u8,
    pub part: u8,
}

impl<P> Default for ProceduralEntry<P> {
    fn default() -> Self {
        Self {
            parameters: None,
            blend: ClipBlend::default(),
            blend_part: 0,
            part: 0,
        }
    }
}

/// One authored Mannequin fragment before transition assembly.
///
/// This is Cry's `CFragment`: database selection returns one of these, while
/// [`FragmentData`] is the transient three-part sequence produced by a query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fragment<K, P> {
    pub blend_out_duration: f32,
    pub animation_layers: Vec<Vec<AnimationClip<K>>>,
    pub procedural_layers: Vec<Vec<ProceduralEntry<P>>>,
}

impl<K, P> Default for Fragment<K, P> {
    fn default() -> Self {
        Self {
            blend_out_duration: 0.2,
            animation_layers: Vec::new(),
            procedural_layers: Vec::new(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipType {
    #[default]
    Normal,
    Transition,
    TransitionOutro,
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FragmentSequenceFlags: u32 {
        const FRAGMENT = 1 << 0;
        const TRANSITION_OUTRO = 1 << 1;
        const TRANSITION = 1 << 2;
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    struct SequencerFlags: u8 {
        const QUEUED = 1 << 0;
        const BLENDING_OUT = 1 << 1;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentData<K, P> {
    pub blend_out_duration: f32,
    pub animation_layers: Vec<Vec<AnimationClip<K>>>,
    pub procedural_layers: Vec<Vec<ProceduralEntry<P>>>,
    pub durations: [f32; FRAGMENT_PART_COUNT],
    pub part_types: [ClipType; FRAGMENT_PART_COUNT],
    pub is_one_shot: bool,
    pub sequence_flags: FragmentSequenceFlags,
}

impl<K, P> Default for FragmentData<K, P> {
    fn default() -> Self {
        Self {
            blend_out_duration: 0.0,
            animation_layers: Vec::new(),
            procedural_layers: Vec::new(),
            durations: [0.0; FRAGMENT_PART_COUNT],
            part_types: [ClipType::Normal; FRAGMENT_PART_COUNT],
            is_one_shot: false,
            sequence_flags: FragmentSequenceFlags::empty(),
        }
    }
}

impl<K, P> FragmentData<K, P> {
    /// Apply optional timing mutations to every concrete clip in a selected
    /// fragment product.
    pub fn apply_timing_overrides(
        &mut self,
        start_time_offset: Option<f32>,
        blend_time: Option<f32>,
    ) {
        let start_time_offset = start_time_offset.filter(|value| *value >= 0.0);
        let blend_time = blend_time.filter(|value| *value >= 0.0);
        for blend in self
            .animation_layers
            .iter_mut()
            .flatten()
            .map(|clip| &mut clip.blend)
            .chain(
                self.procedural_layers
                    .iter_mut()
                    .flatten()
                    .map(|clip| &mut clip.blend),
            )
        {
            if let Some(offset) = start_time_offset {
                blend.start_time += offset;
            }
            if let Some(duration) = blend_time {
                blend.duration = duration;
            }
        }
    }

    /// Move a selected fragment across an animation-key boundary while
    /// retaining its already-cooked procedural values and sequencing data.
    /// This is used by renderer adapters to turn canonical asset references
    /// into loaded motion handles without cloning the fragment graph.
    ///
    /// # Errors
    ///
    /// Returns the first `E` produced by `map`, which is called once per
    /// animation key present in the sequence.
    pub fn try_map_animations<T, E>(
        self,
        mut map: impl FnMut(K) -> Result<T, E>,
    ) -> Result<FragmentData<T, P>, E> {
        let animation_layers = self
            .animation_layers
            .into_iter()
            .map(|layer| {
                layer
                    .into_iter()
                    .map(|clip| {
                        Ok(AnimationClip {
                            blend: clip.blend,
                            animation: AnimationEntry {
                                animation: clip.animation.animation.map(&mut map).transpose()?,
                                flags: clip.animation.flags,
                                playback_speed: clip.animation.playback_speed,
                                playback_weight: clip.animation.playback_weight,
                                blend_channels: clip.animation.blend_channels,
                                weight_list: clip.animation.weight_list,
                            },
                            reference_length: clip.reference_length,
                            blend_part: clip.blend_part,
                            part: clip.part,
                            variable_length: clip.variable_length,
                        })
                    })
                    .collect::<Result<Vec<_>, E>>()
            })
            .collect::<Result<Vec<_>, E>>()?;

        Ok(FragmentData {
            blend_out_duration: self.blend_out_duration,
            animation_layers,
            procedural_layers: self.procedural_layers,
            durations: self.durations,
            part_types: self.part_types,
            is_one_shot: self.is_one_shot,
            sequence_flags: self.sequence_flags,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationStartParameters {
    pub lane: AnimationLane,
    pub transition_time: f32,
    pub key_time: f32,
    pub playback_speed: f32,
    pub playback_weight: f32,
    pub blend_channels: [f32; 4],
    pub weight_list: u8,
    pub user_token: u32,
    pub flags: AnimationFlags,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveAnimationState {
    pub normalized_time: f32,
    pub expected_duration: f32,
}

/// Animation-player capabilities required by Mannequin. A Bevy adapter maps
/// these operations to animation graph nodes and active transitions.
pub trait AnimationPlayback<K> {
    fn animation_duration(&self, animation: &K) -> Option<f32>;

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState>;

    fn start_animation(&mut self, animation: &K, parameters: AnimationStartParameters) -> bool;

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32);

    fn clear_layer(&mut self, lane: AnimationLane);

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32);

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32);

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32);

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32);

    /// Apply Cry's explicit-time update to every active animation on a layer.
    /// The adapter owns representation-specific FIFO/graph traversal.
    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    );
}

/// Stable identity of an animation layer within a Mannequin scope.
///
/// Cry layer numbers are local to each scope context/character instance. The
/// scope ID is therefore part of the runtime identity even when multiple
/// scopes happen to use the same numeric layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnimationLane {
    pub scope: ScopeId,
    pub layer: u32,
}

impl AnimationLane {
    #[must_use]
    pub const fn new(scope: ScopeId, layer: u32) -> Self {
        Self { scope, layer }
    }
}

impl Default for AnimationLane {
    fn default() -> Self {
        Self::new(ScopeId::new(0).expect("zero is a valid scope id"), 0)
    }
}

impl From<ProceduralLane> for AnimationLane {
    #[inline]
    fn from(lane: ProceduralLane) -> Self {
        Self::new(lane.scope, lane.layer)
    }
}

/// Procedural-clip capabilities required by Mannequin. `P` is a cooked,
/// concrete parameter type (commonly a project enum or an `Arc<dyn ...>`).
pub trait ProceduralPlayback<P> {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the argument set CActionScope hands a procedural clip on enter"
    )]
    fn enter_procedural(
        &mut self,
        lane: ProceduralLane,
        scope_base_animation_layer: u32,
        action: ActionHandle,
        parameters: &P,
        blend_time: f32,
        duration: f32,
        user_token: u32,
        install_mode: ProceduralInstallMode,
        action_speed_bias: f32,
        remaining_blend_duration: f32,
    ) -> Option<ActionMutation>;

    fn exit_procedural(&mut self, lane: ProceduralLane, blend_time: f32);

    fn fail_procedural(&mut self, lane: ProceduralLane);

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32);

    fn debug_draw_procedural(&mut self, lane: ProceduralLane);
}

/// A mutation performed by a procedural clip on its controlling action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionMutation {
    SetSpeedBias(f32),
}

impl ActionMutation {
    #[inline]
    pub const fn apply_to(self, action: &mut super::Action) {
        match self {
            Self::SetSpeedBias(speed_bias) => action.speed_bias = speed_bias,
        }
    }
}

/// Installation mode forwarded by a scope to a concrete procedural clip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProceduralInstallMode {
    #[default]
    Normal,
    TimeWarpReinstall,
}

/// Whether the caller requested the procedural debug pass for this update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProceduralDebug {
    #[default]
    Disabled,
    Enabled,
}

impl ProceduralDebug {
    #[inline]
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl From<bool> for ProceduralDebug {
    #[inline]
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

/// Stable identity of one procedural sequencer lane.
///
/// Procedural layer numbers are local to a scope. Carrying the scope ID avoids
/// collisions when multiple active scopes each own layer zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProceduralLane {
    pub scope: ScopeId,
    pub layer: u32,
}

impl ProceduralLane {
    #[must_use]
    pub const fn new(scope: ScopeId, layer: u32) -> Self {
        Self { scope, layer }
    }
}

impl Default for ProceduralLane {
    fn default() -> Self {
        Self {
            scope: ScopeId::try_from(0).expect("0 is a valid scope id"),
            layer: 0,
        }
    }
}

/// Timing and installation data passed to one concrete procedural behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProceduralEnterContext {
    /// Lane in the controller's procedural sequencers. This identifies clip
    /// lifetime; it is not a `CryAnimation` layer.
    pub lane: ProceduralLane,
    /// First absolute `CryAnimation` layer owned by the scope.
    pub scope_base_animation_layer: u32,
    /// Stable identity of the `IAction` controlling this fragment.
    pub action: ActionHandle,
    /// Speed bias copied from the controlling action when the scope installed.
    pub action_speed_bias: f32,
    pub blend_time: f32,
    pub duration: f32,
    /// Sum of the authored blend durations after this entry on the same lane.
    pub remaining_blend_duration: f32,
    pub user_token: u32,
}

impl Default for ProceduralEnterContext {
    fn default() -> Self {
        Self {
            lane: ProceduralLane::default(),
            scope_base_animation_layer: 0,
            action: ActionHandle::from_bits(0),
            action_speed_bias: 1.0,
            blend_time: 0.0,
            duration: 0.0,
            remaining_blend_duration: 0.0,
            user_token: 0,
        }
    }
}

/// Timing data passed when one concrete procedural behavior leaves a layer.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProceduralExitContext {
    pub lane: ProceduralLane,
    pub blend_time: f32,
}

/// A typed procedural clip's lifecycle against the capabilities it requires.
///
/// Project procedural parameter types implement this trait for narrow runtime
/// capability traits. The engine sequencer therefore remains generic over the
/// project clip enum, while each installed clip owns concrete, typed state.
pub trait ProceduralClipBehavior<R> {
    type State;

    /// Whether this clip accepts Mannequin's time-warp auto-reinstall path.
    #[inline]
    fn supports_timewarp_reinstall(&self) -> bool {
        true
    }

    fn enter(&self, runtime: &mut R, context: ProceduralEnterContext) -> Self::State;

    /// Returns a controlling-action mutation produced by `enter`.
    #[inline]
    fn action_mutation_after_enter(
        &self,
        _state: &Self::State,
        _context: ProceduralEnterContext,
    ) -> Option<ActionMutation> {
        None
    }

    fn update(&self, _runtime: &mut R, _state: &mut Self::State, _time_passed: f32) {}

    fn debug_draw(&self, _runtime: &mut R, _state: &mut Self::State) {}

    fn exit(&self, _runtime: &mut R, _state: Self::State, _context: ProceduralExitContext) {}

    /// Cry's default `IProceduralClip::OnFail` is intentionally empty: failure
    /// destroys the installed clip without running its normal exit behavior.
    fn fail(&self, _runtime: &mut R, _state: Self::State) {}
}

struct ActiveProcedural<P, S> {
    parameters: P,
    state: S,
}

const INLINE_PROCEDURAL_LANES: usize = 8;

struct ActiveProceduralEntry<P, S> {
    lane: ProceduralLane,
    procedural: ActiveProcedural<P, S>,
}

/// Sorted, contiguous active-lane storage. Mannequin scopes normally use only
/// a handful of procedural layers, so the common path stays inline while
/// retaining deterministic lookup and allowing unusually large authored rigs
/// to spill safely.
struct ActiveProceduralSet<P, S> {
    entries: SmallVec<[ActiveProceduralEntry<P, S>; INLINE_PROCEDURAL_LANES]>,
}

impl<P, S> Default for ActiveProceduralSet<P, S> {
    fn default() -> Self {
        Self {
            entries: SmallVec::new(),
        }
    }
}

impl<P, S> ActiveProceduralSet<P, S> {
    #[inline]
    fn index(&self, lane: ProceduralLane) -> Result<usize, usize> {
        self.entries.binary_search_by_key(&lane, |entry| entry.lane)
    }

    #[inline]
    fn contains(&self, lane: ProceduralLane) -> bool {
        self.index(lane).is_ok()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn insert(&mut self, lane: ProceduralLane, procedural: ActiveProcedural<P, S>) {
        match self.index(lane) {
            Ok(index) => self.entries[index].procedural = procedural,
            Err(index) => self
                .entries
                .insert(index, ActiveProceduralEntry { lane, procedural }),
        }
    }

    fn remove(&mut self, lane: ProceduralLane) -> Option<ActiveProcedural<P, S>> {
        self.index(lane)
            .ok()
            .map(|index| self.entries.remove(index).procedural)
    }

    fn get_mut(&mut self, lane: ProceduralLane) -> Option<&mut ActiveProcedural<P, S>> {
        let index = self.index(lane).ok()?;
        Some(&mut self.entries[index].procedural)
    }

    #[expect(
        dead_code,
        reason = "shared-reference twin of the used `get_mut`; kept so this private \
                  lane-map exposes the usual insert/remove/get/get_mut set"
    )]
    fn get(&self, lane: ProceduralLane) -> Option<&ActiveProcedural<P, S>> {
        let index = self.index(lane).ok()?;
        Some(&self.entries[index].procedural)
    }
}

/// Owns one renderer/gameplay backend plus the concrete state installed on
/// each procedural layer.
///
/// This is the typed counterpart of Cry's `SProcSequencer::proceduralClip`.
/// It lets [`ScopeRuntime`] execute [`ProceduralClipBehavior`] directly with
/// no factory names, reflected parameter trees, or dynamic downcasts.
pub struct TypedProceduralPlayback<P, R>
where
    P: ProceduralClipBehavior<R>,
{
    runtime: R,
    active: ActiveProceduralSet<P, P::State>,
}

impl<P, R> TypedProceduralPlayback<P, R>
where
    P: ProceduralClipBehavior<R>,
{
    #[must_use]
    pub fn new(runtime: R) -> Self {
        Self {
            runtime,
            active: ActiveProceduralSet::default(),
        }
    }

    #[must_use]
    pub const fn runtime(&self) -> &R {
        &self.runtime
    }

    pub const fn runtime_mut(&mut self) -> &mut R {
        &mut self.runtime
    }

    #[must_use]
    pub fn into_runtime(self) -> R {
        self.runtime
    }

    #[must_use]
    pub fn has_active_procedural(&self, lane: ProceduralLane) -> bool {
        self.active.contains(lane)
    }
}

impl<P, R> From<R> for TypedProceduralPlayback<P, R>
where
    P: ProceduralClipBehavior<R>,
{
    fn from(runtime: R) -> Self {
        Self::new(runtime)
    }
}

impl<P, R> ProceduralPlayback<P> for TypedProceduralPlayback<P, R>
where
    P: Clone + ProceduralClipBehavior<R>,
{
    fn enter_procedural(
        &mut self,
        lane: ProceduralLane,
        scope_base_animation_layer: u32,
        action: ActionHandle,
        parameters: &P,
        blend_time: f32,
        duration: f32,
        user_token: u32,
        install_mode: ProceduralInstallMode,
        action_speed_bias: f32,
        remaining_blend_duration: f32,
    ) -> Option<ActionMutation> {
        debug_assert!(!self.active.contains(lane));
        if install_mode == ProceduralInstallMode::TimeWarpReinstall
            && !parameters.supports_timewarp_reinstall()
        {
            return None;
        }
        let context = ProceduralEnterContext {
            lane,
            scope_base_animation_layer,
            action,
            action_speed_bias,
            blend_time,
            duration,
            remaining_blend_duration,
            user_token,
        };
        let state = parameters.enter(&mut self.runtime, context);
        let mutation = parameters.action_mutation_after_enter(&state, context);
        self.active.insert(
            lane,
            ActiveProcedural {
                parameters: parameters.clone(),
                state,
            },
        );
        mutation
    }

    fn exit_procedural(&mut self, lane: ProceduralLane, blend_time: f32) {
        let Some(active) = self.active.remove(lane) else {
            return;
        };
        active.parameters.exit(
            &mut self.runtime,
            active.state,
            ProceduralExitContext { lane, blend_time },
        );
    }

    fn fail_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.active.remove(lane) else {
            return;
        };
        active.parameters.fail(&mut self.runtime, active.state);
    }

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32) {
        let Some(active) = self.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .update(&mut self.runtime, &mut active.state, time_passed);
    }

    fn debug_draw_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .debug_draw(&mut self.runtime, &mut active.state);
    }
}

impl<K, P, R> AnimationPlayback<K> for TypedProceduralPlayback<P, R>
where
    P: ProceduralClipBehavior<R>,
    R: AnimationPlayback<K>,
{
    fn animation_duration(&self, animation: &K) -> Option<f32> {
        self.runtime.animation_duration(animation)
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        self.runtime.top_animation(lane)
    }

    fn start_animation(&mut self, animation: &K, parameters: AnimationStartParameters) -> bool {
        self.runtime.start_animation(animation, parameters)
    }

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32) {
        self.runtime.stop_animation(lane, blend_time);
    }

    fn clear_layer(&mut self, lane: AnimationLane) {
        self.runtime.clear_layer(lane);
    }

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32) {
        self.runtime.set_layer_playback_scale(lane, scale);
    }

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_layer_blend_weight(lane, weight);
    }

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_top_animation_weight(lane, weight);
    }

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32) {
        self.runtime
            .set_top_animation_normalized_time(lane, normalized_time);
    }

    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    ) {
        self.runtime
            .advance_layer_animations(lane, time_passed, queued_increments);
    }
}

/// Stored procedural-layer ownership separated from the runtime capabilities
/// used to execute it.
///
/// ECS integrations normally keep this registry in a component and assemble a
/// short-lived borrowing runtime facade for each update. This preserves
/// concrete per-clip state without forcing renderer, entity, or message-writer
/// borrows into a long-lived wrapper object.
pub struct TypedProceduralRegistry<P, S> {
    active: ActiveProceduralSet<P, S>,
}

impl<P, S> Default for TypedProceduralRegistry<P, S> {
    fn default() -> Self {
        Self {
            active: ActiveProceduralSet::default(),
        }
    }
}

impl<P, S> TypedProceduralRegistry<P, S> {
    #[must_use]
    pub fn has_active_procedural(&self, lane: ProceduralLane) -> bool {
        self.active.contains(lane)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    #[must_use]
    pub const fn backend<'a, R>(
        &'a mut self,
        runtime: &'a mut R,
    ) -> BorrowedTypedProceduralPlayback<'a, P, S, R> {
        BorrowedTypedProceduralPlayback {
            registry: self,
            runtime,
        }
    }

    /// Borrows the registry and a runtime that supplies both animation and
    /// procedural capabilities for a complete scope/controller update.
    #[must_use]
    pub const fn mannequin_backend<'a, R>(
        &'a mut self,
        runtime: &'a mut R,
    ) -> BorrowedTypedMannequinPlayback<'a, P, S, R> {
        BorrowedTypedMannequinPlayback {
            registry: self,
            runtime,
        }
    }
}

/// Borrows a stored procedural registry together with the concrete runtime
/// capabilities available for the current update.
pub struct BorrowedTypedProceduralPlayback<'a, P, S, R> {
    registry: &'a mut TypedProceduralRegistry<P, S>,
    runtime: &'a mut R,
}

pub struct BorrowedTypedMannequinPlayback<'a, P, S, R> {
    registry: &'a mut TypedProceduralRegistry<P, S>,
    runtime: &'a mut R,
}

impl<K, P, S, R> AnimationPlayback<K> for BorrowedTypedMannequinPlayback<'_, P, S, R>
where
    R: AnimationPlayback<K>,
{
    fn animation_duration(&self, animation: &K) -> Option<f32> {
        self.runtime.animation_duration(animation)
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        self.runtime.top_animation(lane)
    }

    fn start_animation(&mut self, animation: &K, parameters: AnimationStartParameters) -> bool {
        self.runtime.start_animation(animation, parameters)
    }

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32) {
        self.runtime.stop_animation(lane, blend_time);
    }

    fn clear_layer(&mut self, lane: AnimationLane) {
        self.runtime.clear_layer(lane);
    }

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32) {
        self.runtime.set_layer_playback_scale(lane, scale);
    }

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_layer_blend_weight(lane, weight);
    }

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_top_animation_weight(lane, weight);
    }

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32) {
        self.runtime
            .set_top_animation_normalized_time(lane, normalized_time);
    }

    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    ) {
        self.runtime
            .advance_layer_animations(lane, time_passed, queued_increments);
    }
}

impl<P, S, R> ProceduralPlayback<P> for BorrowedTypedMannequinPlayback<'_, P, S, R>
where
    P: Clone + ProceduralClipBehavior<R, State = S>,
{
    fn enter_procedural(
        &mut self,
        lane: ProceduralLane,
        scope_base_animation_layer: u32,
        action: ActionHandle,
        parameters: &P,
        blend_time: f32,
        duration: f32,
        user_token: u32,
        install_mode: ProceduralInstallMode,
        action_speed_bias: f32,
        remaining_blend_duration: f32,
    ) -> Option<ActionMutation> {
        debug_assert!(!self.registry.active.contains(lane));
        if install_mode == ProceduralInstallMode::TimeWarpReinstall
            && !parameters.supports_timewarp_reinstall()
        {
            return None;
        }
        let context = ProceduralEnterContext {
            lane,
            scope_base_animation_layer,
            action,
            action_speed_bias,
            blend_time,
            duration,
            remaining_blend_duration,
            user_token,
        };
        let state = parameters.enter(self.runtime, context);
        let mutation = parameters.action_mutation_after_enter(&state, context);
        self.registry.active.insert(
            lane,
            ActiveProcedural {
                parameters: parameters.clone(),
                state,
            },
        );
        mutation
    }

    fn exit_procedural(&mut self, lane: ProceduralLane, blend_time: f32) {
        let Some(active) = self.registry.active.remove(lane) else {
            return;
        };
        active.parameters.exit(
            self.runtime,
            active.state,
            ProceduralExitContext { lane, blend_time },
        );
    }

    fn fail_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.registry.active.remove(lane) else {
            return;
        };
        active.parameters.fail(self.runtime, active.state);
    }

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32) {
        let Some(active) = self.registry.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .update(self.runtime, &mut active.state, time_passed);
    }

    fn debug_draw_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.registry.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .debug_draw(self.runtime, &mut active.state);
    }
}

/// Composes independently borrowed animation and procedural capabilities into
/// the backend required by a scope/controller update.
///
/// This keeps each host capability concrete and independently reusable without
/// trait objects.
pub struct ComposedMannequinPlayback<'a, A, P> {
    animation: &'a mut A,
    procedural: &'a mut P,
}

impl<'a, A, P> ComposedMannequinPlayback<'a, A, P> {
    #[must_use]
    pub const fn new(animation: &'a mut A, procedural: &'a mut P) -> Self {
        Self {
            animation,
            procedural,
        }
    }
}

impl<K, A, P> AnimationPlayback<K> for ComposedMannequinPlayback<'_, A, P>
where
    A: AnimationPlayback<K>,
{
    fn animation_duration(&self, animation: &K) -> Option<f32> {
        self.animation.animation_duration(animation)
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        self.animation.top_animation(lane)
    }

    fn start_animation(&mut self, animation: &K, parameters: AnimationStartParameters) -> bool {
        self.animation.start_animation(animation, parameters)
    }

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32) {
        self.animation.stop_animation(lane, blend_time);
    }

    fn clear_layer(&mut self, lane: AnimationLane) {
        self.animation.clear_layer(lane);
    }

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32) {
        self.animation.set_layer_playback_scale(lane, scale);
    }

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.animation.set_layer_blend_weight(lane, weight);
    }

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.animation.set_top_animation_weight(lane, weight);
    }

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32) {
        self.animation
            .set_top_animation_normalized_time(lane, normalized_time);
    }

    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    ) {
        self.animation
            .advance_layer_animations(lane, time_passed, queued_increments);
    }
}

impl<C, A, P> ProceduralPlayback<C> for ComposedMannequinPlayback<'_, A, P>
where
    P: ProceduralPlayback<C>,
{
    fn enter_procedural(
        &mut self,
        lane: ProceduralLane,
        scope_base_animation_layer: u32,
        action: ActionHandle,
        parameters: &C,
        blend_time: f32,
        duration: f32,
        user_token: u32,
        install_mode: ProceduralInstallMode,
        action_speed_bias: f32,
        remaining_blend_duration: f32,
    ) -> Option<ActionMutation> {
        self.procedural.enter_procedural(
            lane,
            scope_base_animation_layer,
            action,
            parameters,
            blend_time,
            duration,
            user_token,
            install_mode,
            action_speed_bias,
            remaining_blend_duration,
        )
    }

    fn exit_procedural(&mut self, lane: ProceduralLane, blend_time: f32) {
        self.procedural.exit_procedural(lane, blend_time);
    }

    fn fail_procedural(&mut self, lane: ProceduralLane) {
        self.procedural.fail_procedural(lane);
    }

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32) {
        self.procedural.update_procedural(lane, time_passed);
    }

    fn debug_draw_procedural(&mut self, lane: ProceduralLane) {
        self.procedural.debug_draw_procedural(lane);
    }
}

impl<P, S, R> ProceduralPlayback<P> for BorrowedTypedProceduralPlayback<'_, P, S, R>
where
    P: Clone + ProceduralClipBehavior<R, State = S>,
{
    fn enter_procedural(
        &mut self,
        lane: ProceduralLane,
        scope_base_animation_layer: u32,
        action: ActionHandle,
        parameters: &P,
        blend_time: f32,
        duration: f32,
        user_token: u32,
        install_mode: ProceduralInstallMode,
        action_speed_bias: f32,
        remaining_blend_duration: f32,
    ) -> Option<ActionMutation> {
        debug_assert!(!self.registry.active.contains(lane));
        if install_mode == ProceduralInstallMode::TimeWarpReinstall
            && !parameters.supports_timewarp_reinstall()
        {
            return None;
        }
        let context = ProceduralEnterContext {
            lane,
            scope_base_animation_layer,
            action,
            action_speed_bias,
            blend_time,
            duration,
            remaining_blend_duration,
            user_token,
        };
        let state = parameters.enter(self.runtime, context);
        let mutation = parameters.action_mutation_after_enter(&state, context);
        self.registry.active.insert(
            lane,
            ActiveProcedural {
                parameters: parameters.clone(),
                state,
            },
        );
        mutation
    }

    fn exit_procedural(&mut self, lane: ProceduralLane, blend_time: f32) {
        let Some(active) = self.registry.active.remove(lane) else {
            return;
        };
        active.parameters.exit(
            self.runtime,
            active.state,
            ProceduralExitContext { lane, blend_time },
        );
    }

    fn fail_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.registry.active.remove(lane) else {
            return;
        };
        active.parameters.fail(self.runtime, active.state);
    }

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32) {
        let Some(active) = self.registry.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .update(self.runtime, &mut active.state, time_passed);
    }

    fn debug_draw_procedural(&mut self, lane: ProceduralLane) {
        let Some(active) = self.registry.active.get_mut(lane) else {
            return;
        };
        active
            .parameters
            .debug_draw(self.runtime, &mut active.state);
    }
}

impl<K, P, S, R> AnimationPlayback<K> for BorrowedTypedProceduralPlayback<'_, P, S, R>
where
    R: AnimationPlayback<K>,
{
    fn animation_duration(&self, animation: &K) -> Option<f32> {
        self.runtime.animation_duration(animation)
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        self.runtime.top_animation(lane)
    }

    fn start_animation(&mut self, animation: &K, parameters: AnimationStartParameters) -> bool {
        self.runtime.start_animation(animation, parameters)
    }

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32) {
        self.runtime.stop_animation(lane, blend_time);
    }

    fn clear_layer(&mut self, lane: AnimationLane) {
        self.runtime.clear_layer(lane);
    }

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32) {
        self.runtime.set_layer_playback_scale(lane, scale);
    }

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_layer_blend_weight(lane, weight);
    }

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_top_animation_weight(lane, weight);
    }

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32) {
        self.runtime
            .set_top_animation_normalized_time(lane, normalized_time);
    }

    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    ) {
        self.runtime
            .advance_layer_animations(lane, time_passed, queued_increments);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeEvent {
    ActionMutation {
        action: ActionHandle,
        mutation: ActionMutation,
    },
    ClipInstalled {
        scope: ScopeId,
        part: u8,
        clip_type: ClipType,
    },
    SequenceFinished {
        scope: ScopeId,
        layer: u32,
    },
}

#[derive(Debug, Clone)]
struct AnimationSequencer<K> {
    sequence: Vec<AnimationClip<K>>,
    blend: ClipBlend,
    install_time: f32,
    reference_time: f32,
    saved_normalized_time: f32,
    position: usize,
    flags: SequencerFlags,
}

impl<K> Default for AnimationSequencer<K> {
    fn default() -> Self {
        Self {
            sequence: Vec::new(),
            blend: ClipBlend::default(),
            install_time: -1.0,
            reference_time: -1.0,
            saved_normalized_time: -1.0,
            position: 0,
            flags: SequencerFlags::empty(),
        }
    }
}

#[derive(Debug, Clone)]
struct ProceduralSequencer<P> {
    sequence: Vec<ProceduralEntry<P>>,
    blend: ClipBlend,
    install_time: f32,
    position: usize,
    flags: SequencerFlags,
}

impl<P> Default for ProceduralSequencer<P> {
    fn default() -> Self {
        Self {
            sequence: Vec::new(),
            blend: ClipBlend::default(),
            install_time: -1.0,
            position: 0,
            flags: SequencerFlags::empty(),
        }
    }
}

/// Shipping scope state. Clip keys and procedural parameters are cooked before
/// this type is constructed; no names are resolved in the update path.
#[derive(Debug, Clone)]
pub struct ScopeRuntime<K, P> {
    id: ScopeId,
    base_layer: u32,
    animation_sequencers: Vec<AnimationSequencer<K>>,
    procedural_sequencers: Vec<ProceduralSequencer<P>>,
    speed_bias: f32,
    animation_weight: f32,
    time_increment: f32,
    additional_tags: TagState,
    last_fragment: Option<FragmentId>,
    last_queue_tags: FragmentTagState,
    last_selection: Option<FragmentSelection>,
    sequence_flags: FragmentSequenceFlags,
    fragment_time: f32,
    fragment_duration: f32,
    transition_outro_duration: f32,
    transition_duration: f32,
    blend_out_duration: f32,
    part_types: [ClipType; FRAGMENT_PART_COUNT],
    last_normalized_time: f32,
    normalized_time: f32,
    user_token: u32,
    controlling_action: Option<ActionHandle>,
    muted_animation_layers: u32,
    muted_procedural_layers: u32,
    one_shot: bool,
    fragment_installed: bool,
}

impl<K, P> ScopeRuntime<K, P>
where
    K: Clone,
    P: Clone,
{
    #[must_use]
    pub fn new(
        id: ScopeId,
        base_layer: u32,
        layer_count: usize,
        additional_tags: TagState,
    ) -> Self {
        Self {
            id,
            base_layer,
            animation_sequencers: vec![AnimationSequencer::default(); layer_count],
            procedural_sequencers: Vec::new(),
            speed_bias: 1.0,
            animation_weight: 1.0,
            time_increment: 0.0,
            additional_tags,
            last_fragment: None,
            last_queue_tags: FragmentTagState::default(),
            last_selection: None,
            sequence_flags: FragmentSequenceFlags::empty(),
            fragment_time: 0.0,
            fragment_duration: 0.0,
            transition_outro_duration: 0.0,
            transition_duration: 0.0,
            blend_out_duration: 0.0,
            part_types: [ClipType::Normal; FRAGMENT_PART_COUNT],
            last_normalized_time: 0.0,
            normalized_time: 0.0,
            user_token: 0,
            controlling_action: None,
            muted_animation_layers: 0,
            muted_procedural_layers: 0,
            one_shot: false,
            fragment_installed: false,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ScopeId {
        self.id
    }

    #[must_use]
    pub const fn base_layer(&self) -> u32 {
        self.base_layer
    }

    #[must_use]
    pub const fn additional_tags(&self) -> TagState {
        self.additional_tags
    }

    #[must_use]
    pub const fn sequence_flags(&self) -> FragmentSequenceFlags {
        self.sequence_flags
    }

    #[must_use]
    pub const fn fragment_time(&self) -> f32 {
        self.fragment_time
    }

    #[must_use]
    pub const fn fragment_duration(&self) -> f32 {
        self.fragment_duration
    }

    #[must_use]
    pub const fn transition_duration(&self) -> f32 {
        self.transition_duration
    }

    #[must_use]
    pub const fn transition_outro_duration(&self) -> f32 {
        self.transition_outro_duration
    }

    #[must_use]
    pub const fn blend_out_duration(&self) -> f32 {
        self.blend_out_duration
    }

    #[must_use]
    pub const fn is_one_shot(&self) -> bool {
        self.one_shot
    }

    #[must_use]
    pub const fn last_fragment(&self) -> Option<FragmentId> {
        self.last_fragment
    }

    #[must_use]
    pub const fn last_selection(&self) -> Option<FragmentSelection> {
        self.last_selection
    }

    #[must_use]
    pub const fn previous_normalized_time(&self) -> f32 {
        self.last_normalized_time
    }

    #[must_use]
    pub const fn normalized_time(&self) -> f32 {
        self.normalized_time
    }

    /// Capture the current root-animation time before transition selection.
    #[inline]
    pub const fn snapshot_normalized_time(&mut self) {
        self.last_normalized_time = self.normalized_time;
    }

    #[must_use]
    pub const fn controlling_action(&self) -> Option<ActionHandle> {
        self.controlling_action
    }

    #[must_use]
    pub fn is_same_selection(
        &self,
        fragment: FragmentId,
        selection: Option<FragmentSelection>,
    ) -> bool {
        self.last_fragment == Some(fragment)
            && self.last_selection.map(|value| value.tag_set_index)
                == selection.map(|value| value.tag_set_index)
    }

    #[must_use]
    pub const fn has_fragment(&self) -> bool {
        self.sequence_flags
            .contains(FragmentSequenceFlags::FRAGMENT)
    }

    #[must_use]
    pub const fn has_outro_transition(&self) -> bool {
        self.sequence_flags
            .contains(FragmentSequenceFlags::TRANSITION_OUTRO)
    }

    #[must_use]
    pub fn fragment_start_time(&self) -> f32 {
        self.transition_duration - self.fragment_time
    }

    #[must_use]
    pub fn blend_query(
        &self,
        fragment_to: Option<FragmentId>,
        tag_state_to: FragmentTagState,
        higher_priority: bool,
        to_installed: bool,
        no_transitions: bool,
    ) -> BlendQuery {
        let mut flags = BlendQueryFlags::empty();
        flags.set(BlendQueryFlags::FROM_INSTALLED, self.fragment_installed);
        flags.set(BlendQueryFlags::TO_INSTALLED, to_installed);
        flags.set(BlendQueryFlags::HIGHER_PRIORITY, higher_priority);
        flags.set(BlendQueryFlags::NO_TRANSITIONS, no_transitions);
        BlendQuery {
            fragment_from: self.last_fragment,
            fragment_to,
            tag_state_from: self.last_queue_tags,
            tag_state_to,
            additional_tags: self.additional_tags,
            fragment_time: self.fragment_time,
            previous_normalized_time: self.last_normalized_time,
            normalized_time: self.normalized_time,
            flags,
            forced_blend: None,
        }
    }

    pub const fn increment_time(&mut self, delta_time: f32) {
        self.time_increment = delta_time;
    }

    pub const fn mute_layers(&mut self, animation_layers: u32, procedural_layers: u32) {
        self.muted_animation_layers = animation_layers;
        self.muted_procedural_layers = procedural_layers;
    }

    /// Install already-selected fragment data. Database/tag matching stays in
    /// the controller driver and hands this hot path concrete clip keys.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the already-selected fragment state CActionScope::QueueFragment installs"
    )]
    pub fn queue_fragment(
        &mut self,
        fragment: Option<FragmentId>,
        tags: FragmentTagState,
        selection: Option<FragmentSelection>,
        mut data: FragmentData<K, P>,
        mut start_time: f32,
        controlling_action: ActionHandle,
        action_speed_bias: f32,
        animation_weight: f32,
        user_token: u32,
        is_root_scope: bool,
        persistent_fragment: bool,
        principle_context: bool,
    ) -> bool {
        self.sequence_flags = data.sequence_flags;
        self.last_queue_tags = tags;
        self.last_selection = selection;
        self.last_normalized_time = 0.0;
        self.normalized_time = 0.0;
        self.one_shot = data.is_one_shot && !persistent_fragment;
        self.blend_out_duration = data.blend_out_duration;
        self.fragment_installed = principle_context;
        self.speed_bias = action_speed_bias;
        self.animation_weight = animation_weight;

        self.fragment_duration = 0.0;
        self.transition_duration = 0.0;
        self.transition_outro_duration = 0.0;
        self.part_types = data.part_types;
        for (part_type, duration) in data.part_types.into_iter().zip(data.durations) {
            match part_type {
                ClipType::Normal => self.fragment_duration += duration,
                ClipType::Transition => self.transition_duration += duration,
                ClipType::TransitionOutro => self.transition_outro_duration += duration,
            }
        }

        if !is_root_scope {
            if self.sequence_flags.intersects(
                FragmentSequenceFlags::TRANSITION | FragmentSequenceFlags::TRANSITION_OUTRO,
            ) {
                start_time = 0.0;
            } else {
                start_time = (start_time
                    - (self.transition_outro_duration + self.transition_duration))
                    .max(0.0);
            }
        }

        self.last_fragment = fragment;
        self.fragment_time = -start_time;
        self.user_token = user_token;
        self.controlling_action = Some(controlling_action);

        let scope_layers = self.animation_sequencers.len();
        let mut animation_layers = data.animation_layers.drain(..);
        for layer in 0..scope_layers {
            let sequencer = &mut self.animation_sequencers[layer];
            sequencer.position = 0;
            sequencer.reference_time = -1.0;
            match animation_layers.next() {
                Some(sequence) if !sequence.is_empty() => {
                    sequencer.blend = sequence[0].blend;
                    sequencer.install_time = start_time + sequencer.blend.exit_time;
                    sequencer.sequence = sequence;
                    sequencer.flags = SequencerFlags::QUEUED;
                }
                _ => {
                    sequencer.sequence.clear();
                    sequencer.blend = ClipBlend::default();
                    sequencer.install_time = start_time;
                    sequencer.flags = SequencerFlags::QUEUED | SequencerFlags::BLENDING_OUT;
                }
            }
        }

        let procedural_count = data
            .procedural_layers
            .len()
            .max(self.procedural_sequencers.len());
        self.procedural_sequencers
            .resize_with(procedural_count, ProceduralSequencer::default);
        let mut procedural_layers = data.procedural_layers.drain(..);
        for layer in 0..procedural_count {
            let sequencer = &mut self.procedural_sequencers[layer];
            sequencer.position = 0;
            match procedural_layers.next() {
                Some(sequence) if !sequence.is_empty() => {
                    let layer_blend_time = sequence[0].blend.exit_time;
                    sequencer.install_time = start_time;
                    sequencer.blend = sequence[0].blend;
                    sequencer.sequence = sequence;
                    sequencer.flags = SequencerFlags::QUEUED;
                    if layer_blend_time > 0.0 {
                        sequencer.blend = ClipBlend::default();
                        sequencer.flags.insert(SequencerFlags::BLENDING_OUT);
                    }
                }
                _ => {
                    sequencer.sequence.clear();
                    sequencer.blend = ClipBlend::default();
                    sequencer.install_time = start_time;
                    sequencer.flags = SequencerFlags::QUEUED | SequencerFlags::BLENDING_OUT;
                }
            }
        }

        self.sequence_flags
            .contains(FragmentSequenceFlags::FRAGMENT)
    }

    /// Time left before the installed fragment finishes on this scope.
    ///
    /// # Panics
    ///
    /// Never in practice: the trailing clip is only read after the sequence has
    /// been confirmed non-empty.
    #[must_use]
    pub fn calculate_fragment_time_remaining(&self) -> f32 {
        if let Some(sequencer) = self.animation_sequencers.first()
            && !sequencer.sequence.is_empty()
        {
            let mut remaining = sequencer.install_time.max(0.0);
            for clip in sequencer.sequence.iter().skip(sequencer.position + 1) {
                remaining += clip.blend.exit_time;
            }
            if sequencer.position < sequencer.sequence.len() {
                remaining += sequencer.sequence.last().unwrap().reference_length;
            }
            return remaining;
        }

        self.fragment_duration + self.transition_duration + self.transition_outro_duration
            - self.fragment_time
    }

    /// Calculates duration using the first clip selected for this scope.
    #[must_use]
    pub fn calculate_fragment_duration(
        &self,
        fragment: &FragmentData<K, P>,
        backend: &impl AnimationPlayback<K>,
    ) -> f32 {
        let Some(first_clip) = fragment
            .animation_layers
            .first()
            .and_then(|layer| layer.first())
        else {
            return 0.0;
        };

        let clip_count = fragment.animation_layers[0].len();
        let mut duration = 0.0;
        let mut last_duration = 0.0;
        for index in 0..clip_count {
            if index > 0 {
                duration += if first_clip.blend.exit_time >= 0.0 {
                    first_clip.blend.exit_time
                } else {
                    last_duration
                };
            }
            last_duration = first_clip
                .animation
                .animation
                .as_ref()
                .filter(|_| {
                    !first_clip
                        .animation
                        .flags
                        .contains(AnimationFlags::LOOP_ANIMATION)
                        && first_clip.animation.playback_speed > 0.0
                })
                .and_then(|animation| backend.animation_duration(animation))
                .map_or(0.0, |value| value / first_clip.animation.playback_speed);
        }
        duration + last_duration
    }

    /// Absolute [`AnimationLane`] for a scope-relative animation layer index.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "animation layer indices are bounded by the scope's authored layer count, \
                  which Mannequin itself stores as a u32"
    )]
    const fn animation_lane(scope: ScopeId, base_layer: u32, layer: usize) -> AnimationLane {
        AnimationLane::new(scope, base_layer + layer as u32)
    }

    /// [`ProceduralLane`] for a scope-relative procedural layer index.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "procedural layer indices are bounded by the scope's authored layer count, \
                  which Mannequin itself stores as a u32"
    )]
    const fn procedural_lane(scope: ScopeId, layer: usize) -> ProceduralLane {
        ProceduralLane::new(scope, layer as u32)
    }

    pub fn apply_animation_weight(
        &self,
        layer: usize,
        weight: f32,
        backend: &mut impl AnimationPlayback<K>,
    ) {
        let Some(sequencer) = self.animation_sequencers.get(layer) else {
            return;
        };
        let Some(clip) = sequencer
            .position
            .checked_sub(1)
            .and_then(|index| sequencer.sequence.get(index))
        else {
            return;
        };
        backend.set_top_animation_weight(
            Self::animation_lane(self.id, self.base_layer, layer),
            (weight * clip.animation.playback_weight).max(0.0),
        );
    }

    #[expect(
        clippy::float_cmp,
        reason = "bit-exact port of CActionScope::Update: the shipping code re-pushes the \
                  bias and weight to every layer only when the incoming value differs \
                  exactly, so an epsilon comparison would change how often it re-pushes"
    )]
    pub fn update(
        &mut self,
        delta_time: f32,
        playing_action: Option<(f32, f32)>,
        procedural_debug: ProceduralDebug,
        backend: &mut (impl AnimationPlayback<K> + ProceduralPlayback<P>),
        mut emit: impl FnMut(ScopeEvent),
    ) {
        let mut scaled_time = delta_time;
        if let Some((new_speed_bias, new_animation_weight)) = playing_action {
            scaled_time *= self.speed_bias;
            if self.speed_bias != new_speed_bias {
                self.speed_bias = new_speed_bias;
                if self.time_increment == 0.0 {
                    for layer in 0..self.animation_sequencers.len() {
                        backend.set_layer_playback_scale(
                            Self::animation_lane(self.id, self.base_layer, layer),
                            new_speed_bias,
                        );
                    }
                }
            }
            if self.animation_weight != new_animation_weight {
                self.animation_weight = new_animation_weight;
                for layer in 0..self.animation_sequencers.len() {
                    backend.set_layer_blend_weight(
                        Self::animation_lane(self.id, self.base_layer, layer),
                        new_animation_weight,
                    );
                }
            }
        }

        let advanced_time =
            self.update_sequencers(scaled_time, procedural_debug, backend, &mut emit);
        self.fragment_time += advanced_time;
        if self.one_shot && self.fragment_installed {
            let total =
                self.fragment_duration + self.transition_duration + self.transition_outro_duration;
            if total > 0.0 {
                self.fragment_time = self.fragment_time.min(total);
            }
        }
    }

    pub fn pause(&mut self, backend: &impl AnimationPlayback<K>) {
        for (layer, sequencer) in self.animation_sequencers.iter_mut().enumerate() {
            sequencer.saved_normalized_time = backend
                .top_animation(Self::animation_lane(self.id, self.base_layer, layer))
                .map_or(-1.0, |animation| animation.normalized_time);
        }
    }

    pub fn resume(
        &mut self,
        forced_blend_time: Option<f32>,
        flags: ResumeFlags,
        backend: &mut impl AnimationPlayback<K>,
    ) {
        for layer in 0..self.animation_sequencers.len() {
            let sequencer = &self.animation_sequencers[layer];
            let animation_lane = Self::animation_lane(self.id, self.base_layer, layer);
            let blend_time = forced_blend_time.unwrap_or(sequencer.blend.duration);
            if sequencer.saved_normalized_time < 0.0 {
                backend.stop_animation(animation_lane, blend_time);
                continue;
            }
            let Some(clip) = sequencer
                .position
                .checked_sub(1)
                .and_then(|position| sequencer.sequence.get(position))
            else {
                continue;
            };
            let Some(animation) = clip.animation.animation.as_ref() else {
                continue;
            };
            let parameters = self.animation_parameters(layer, &clip.animation, clip.blend, backend);
            if backend.start_animation(animation, parameters) {
                let restore = if clip
                    .animation
                    .flags
                    .contains(AnimationFlags::LOOP_ANIMATION)
                {
                    flags.contains(ResumeFlags::RESTORE_LOOPING_ANIMATION_TIME)
                } else {
                    flags.contains(ResumeFlags::RESTORE_NON_LOOPING_ANIMATION_TIME)
                };
                if restore {
                    backend.set_top_animation_normalized_time(
                        animation_lane,
                        sequencer.saved_normalized_time,
                    );
                }
            }
        }
    }

    pub fn clear_sequencers(&mut self) {
        for sequencer in &mut self.animation_sequencers {
            sequencer.sequence.clear();
            sequencer.install_time = -1.0;
            sequencer.position = 0;
            sequencer.flags = SequencerFlags::empty();
        }
        for sequencer in &mut self.procedural_sequencers {
            sequencer.sequence.clear();
            sequencer.install_time = -1.0;
            sequencer.position = 0;
            sequencer.flags = SequencerFlags::empty();
        }
        self.controlling_action = None;
    }

    pub fn flush(
        &mut self,
        method: ActionEndMethod,
        backend: &mut (impl AnimationPlayback<K> + ProceduralPlayback<P>),
    ) {
        for layer in 0..self.animation_sequencers.len() {
            let sequencer = &mut self.animation_sequencers[layer];
            sequencer.sequence.clear();
            sequencer.install_time = -1.0;
            sequencer.position = 0;
            sequencer.flags = SequencerFlags::empty();
            if method != ActionEndMethod::NormalLeaveAnimations {
                backend.clear_layer(Self::animation_lane(self.id, self.base_layer, layer));
            }
        }
        for layer in 0..self.procedural_sequencers.len() {
            let lane = Self::procedural_lane(self.id, layer);
            match method {
                ActionEndMethod::Normal | ActionEndMethod::NormalLeaveAnimations => {
                    backend.exit_procedural(lane, 0.0);
                }
                ActionEndMethod::Failure => backend.fail_procedural(lane),
            }
        }
        self.procedural_sequencers.clear();
        self.last_fragment = None;
        self.last_selection = None;
        self.controlling_action = None;
        self.fragment_time = 0.0;
        self.last_queue_tags = FragmentTagState::default();
        self.sequence_flags = FragmentSequenceFlags::empty();
    }

    fn update_sequencers(
        &mut self,
        delta_time: f32,
        procedural_debug: ProceduralDebug,
        backend: &mut (impl AnimationPlayback<K> + ProceduralPlayback<P>),
        emit: &mut impl FnMut(ScopeEvent),
    ) -> f32 {
        let had_increment = self.time_increment != 0.0;
        let time_passed = delta_time + self.time_increment;
        self.time_increment = 0.0;
        let mut queued_increments = [0.0; MAX_QUEUED_TIME_INCREMENTS];
        let mut queued_count = 0;

        for layer in 0..self.animation_sequencers.len() {
            let mut time_left = time_passed;
            while self.animation_sequencers[layer]
                .flags
                .contains(SequencerFlags::QUEUED)
            {
                let install_time = self.animation_sequencers[layer].install_time;
                if time_left < install_time {
                    self.animation_sequencers[layer].install_time -= time_left;
                    break;
                }
                let remainder = time_left - install_time;
                if self.play_pending_animation(layer, backend, emit) {
                    time_left = remainder;
                    if queued_count < MAX_QUEUED_TIME_INCREMENTS {
                        queued_increments[queued_count] = time_left;
                        queued_count += 1;
                    }
                    let sequencer = &self.animation_sequencers[layer];
                    if sequencer.position >= sequencer.sequence.len() {
                        break;
                    }
                }
            }

            if had_increment {
                backend.advance_layer_animations(
                    Self::animation_lane(self.id, self.base_layer, layer),
                    time_passed,
                    &queued_increments[..queued_count],
                );
            }
        }

        if let Some(root) = backend.top_animation(AnimationLane::new(self.id, self.base_layer)) {
            self.last_normalized_time = self.normalized_time;
            self.normalized_time = root.normalized_time;
        }

        self.advance_procedural_sequencers(
            time_passed,
            ProceduralInstallMode::Normal,
            procedural_debug,
            backend,
            emit,
        );
        time_passed
    }

    /// Enter procedural clips whose install time has elapsed without advancing
    /// the animation sequencers. Mannequin uses this immediately after queuing
    /// a fragment when procedural-on-install is enabled.
    pub fn enter_queued_procedurals(
        &mut self,
        install_mode: ProceduralInstallMode,
        backend: &mut impl ProceduralPlayback<P>,
        mut emit: impl FnMut(ScopeEvent),
    ) {
        self.advance_procedural_sequencers(
            0.0,
            install_mode,
            ProceduralDebug::Enabled,
            backend,
            &mut emit,
        );
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "animation layer indices are bounded by the scope's authored layer count, \
                  which the emitted event carries as a u32"
    )]
    fn play_pending_animation(
        &mut self,
        layer: usize,
        backend: &mut impl AnimationPlayback<K>,
        emit: &mut impl FnMut(ScopeEvent),
    ) -> bool {
        let (blending_out, blend, entry, part) = {
            let sequencer = &mut self.animation_sequencers[layer];
            let blending_out = sequencer.flags.contains(SequencerFlags::BLENDING_OUT);
            sequencer.flags.remove(SequencerFlags::QUEUED);
            sequencer.install_time = 0.0;
            sequencer.reference_time = -1.0;
            if sequencer.position < sequencer.sequence.len() || blending_out {
                if blending_out {
                    sequencer.flags.remove(SequencerFlags::BLENDING_OUT);
                    (true, sequencer.blend, AnimationEntry::default(), 0)
                } else {
                    let clip = sequencer.sequence[sequencer.position].clone();
                    sequencer.position += 1;
                    (false, sequencer.blend, clip.animation, clip.part)
                }
            } else {
                emit(ScopeEvent::SequenceFinished {
                    scope: self.id,
                    layer: layer as u32,
                });
                return false;
            }
        };

        self.install_animation(layer, &entry, blend, backend);
        if !blending_out {
            emit(ScopeEvent::ClipInstalled {
                scope: self.id,
                part,
                clip_type: self.part_types[usize::from(part).min(FRAGMENT_PART_COUNT - 1)],
            });
        }
        let persistent = entry.flags.contains(AnimationFlags::LOOP_ANIMATION);
        Self::queue_animation_from_sequence(&mut self.animation_sequencers[layer], persistent);
        true
    }

    fn queue_animation_from_sequence(sequencer: &mut AnimationSequencer<K>, persistent: bool) {
        if let Some(clip) = sequencer.sequence.get(sequencer.position) {
            sequencer.blend = clip.blend;
            sequencer.install_time = clip.blend.exit_time;
            if sequencer.position > 0 {
                sequencer.reference_time =
                    sequencer.sequence[sequencer.position - 1].reference_length;
            }
            sequencer.flags.insert(SequencerFlags::QUEUED);
            debug_assert!(sequencer.install_time >= 0.0);
        } else if !persistent {
            if sequencer.position > 0 {
                sequencer.reference_time =
                    sequencer.sequence[sequencer.position - 1].reference_length;
            }
            sequencer.install_time = sequencer.reference_time;
            sequencer.flags.insert(SequencerFlags::QUEUED);
        }
    }

    fn install_animation(
        &self,
        layer: usize,
        entry: &AnimationEntry<K>,
        blend: ClipBlend,
        backend: &mut impl AnimationPlayback<K>,
    ) -> bool {
        if layer < u32::BITS as usize && self.muted_animation_layers & (1 << layer) != 0 {
            return false;
        }
        let animation_lane = Self::animation_lane(self.id, self.base_layer, layer);
        let Some(animation) = entry.animation.as_ref() else {
            backend.stop_animation(animation_lane, blend.duration);
            return true;
        };
        let parameters = self.animation_parameters(layer, entry, blend, backend);
        backend.start_animation(animation, parameters)
    }

    fn animation_parameters(
        &self,
        layer: usize,
        entry: &AnimationEntry<K>,
        blend: ClipBlend,
        backend: &impl AnimationPlayback<K>,
    ) -> AnimationStartParameters {
        let key_time = entry
            .animation
            .as_ref()
            .and_then(|animation| backend.animation_duration(animation))
            .filter(|duration| *duration > 0.0)
            .map_or(0.0, |duration| blend.start_time / duration);
        let mut flags = entry.flags | blend.flags | AnimationFlags::ALLOW_ANIMATION_RESTART;
        if !entry.flags.contains(AnimationFlags::LOOP_ANIMATION) {
            flags.insert(AnimationFlags::REPEAT_LAST_KEY);
        }
        AnimationStartParameters {
            lane: Self::animation_lane(self.id, self.base_layer, layer),
            transition_time: blend.duration,
            key_time,
            playback_speed: entry.playback_speed,
            playback_weight: entry.playback_weight,
            blend_channels: entry.blend_channels,
            weight_list: entry.weight_list,
            user_token: self.user_token,
            flags,
        }
    }

    fn advance_procedural_sequencers(
        &mut self,
        delta_time: f32,
        install_mode: ProceduralInstallMode,
        debug: ProceduralDebug,
        backend: &mut impl ProceduralPlayback<P>,
        emit: &mut impl FnMut(ScopeEvent),
    ) {
        for layer in 0..self.procedural_sequencers.len() {
            let mut time_left = delta_time;
            while self.procedural_sequencers[layer]
                .flags
                .contains(SequencerFlags::QUEUED)
            {
                let install_time = self.procedural_sequencers[layer].install_time;
                if time_left < install_time {
                    self.procedural_sequencers[layer].install_time -= time_left;
                    break;
                }
                time_left -= install_time;
                self.procedural_sequencers[layer].install_time = -1.0;
                self.play_pending_procedural(layer, install_mode, backend, emit);
            }
            let lane = Self::procedural_lane(self.id, layer);
            backend.update_procedural(lane, time_left);
            if debug.is_enabled() {
                backend.debug_draw_procedural(lane);
            }
        }
    }

    fn play_pending_procedural(
        &mut self,
        layer: usize,
        install_mode: ProceduralInstallMode,
        backend: &mut impl ProceduralPlayback<P>,
        emit: &mut impl FnMut(ScopeEvent),
    ) -> bool {
        let (entry, blend, duration, remaining_blend_duration, blending_out) = {
            let sequencer = &mut self.procedural_sequencers[layer];
            sequencer.flags.remove(SequencerFlags::QUEUED);
            let blending_out = sequencer.flags.contains(SequencerFlags::BLENDING_OUT);
            if sequencer.position >= sequencer.sequence.len() && !blending_out {
                return false;
            }
            if blending_out {
                sequencer.flags.remove(SequencerFlags::BLENDING_OUT);
                (ProceduralEntry::default(), sequencer.blend, -1.0, 0.0, true)
            } else {
                let entry = sequencer.sequence[sequencer.position].clone();
                sequencer.position += 1;
                let duration = sequencer
                    .sequence
                    .get(sequencer.position)
                    .map_or(-1.0, |next| {
                        next.blend.exit_time - sequencer.blend.exit_time
                    });
                let remaining_blend_duration = sequencer.sequence[sequencer.position..]
                    .iter()
                    .map(|entry| entry.blend.duration)
                    .sum();
                (
                    entry,
                    sequencer.blend,
                    duration,
                    remaining_blend_duration,
                    false,
                )
            }
        };

        if layer < u32::BITS as usize && self.muted_procedural_layers & (1 << layer) == 0 {
            let lane = Self::procedural_lane(self.id, layer);
            backend.exit_procedural(lane, blend.duration);
            if let Some(parameters) = entry.parameters.as_ref() {
                let controlling_action = self
                    .controlling_action
                    .expect("a queued procedural fragment must have a controlling action");
                if let Some(mutation) = backend.enter_procedural(
                    lane,
                    self.base_layer,
                    controlling_action,
                    parameters,
                    blend.duration,
                    duration,
                    self.user_token,
                    install_mode,
                    self.speed_bias,
                    remaining_blend_duration,
                ) {
                    emit(ScopeEvent::ActionMutation {
                        action: controlling_action,
                        mutation,
                    });
                }
            }
        }
        if !blending_out {
            emit(ScopeEvent::ClipInstalled {
                scope: self.id,
                part: entry.part,
                clip_type: self.part_types[usize::from(entry.part).min(FRAGMENT_PART_COUNT - 1)],
            });
        }
        Self::queue_procedural_from_sequence(&mut self.procedural_sequencers[layer]);
        !blending_out
    }

    fn queue_procedural_from_sequence(sequencer: &mut ProceduralSequencer<P>) {
        if let Some(entry) = sequencer.sequence.get(sequencer.position) {
            sequencer.blend = entry.blend;
            sequencer.install_time = entry.blend.exit_time;
            sequencer.flags.insert(SequencerFlags::QUEUED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestProcedural {
        accepts_timewarp: bool,
    }

    impl ProceduralClipBehavior<Vec<&'static str>> for TestProcedural {
        type State = ();

        fn supports_timewarp_reinstall(&self) -> bool {
            self.accepts_timewarp
        }

        fn enter(
            &self,
            runtime: &mut Vec<&'static str>,
            _context: ProceduralEnterContext,
        ) -> Self::State {
            runtime.push("enter");
        }

        fn update(
            &self,
            runtime: &mut Vec<&'static str>,
            _state: &mut Self::State,
            _time_passed: f32,
        ) {
            runtime.push("update");
        }

        fn debug_draw(&self, runtime: &mut Vec<&'static str>, _state: &mut Self::State) {
            runtime.push("debug");
        }
    }

    #[test]
    fn timewarp_reinstall_rejects_an_unsupported_clip_before_enter() {
        let lane = ProceduralLane::default();
        let mut playback = TypedProceduralPlayback::new(Vec::new());

        playback.enter_procedural(
            lane,
            0,
            ActionHandle::from_bits(0),
            &TestProcedural {
                accepts_timewarp: false,
            },
            0.0,
            0.0,
            0,
            ProceduralInstallMode::TimeWarpReinstall,
            1.0,
            0.0,
        );

        assert!(!playback.has_active_procedural(lane));
        assert!(playback.runtime().is_empty());
    }

    #[test]
    fn procedural_debug_runs_after_update() {
        let lane = ProceduralLane::default();
        let mut playback = TypedProceduralPlayback::new(Vec::new());
        playback.enter_procedural(
            lane,
            0,
            ActionHandle::from_bits(0),
            &TestProcedural {
                accepts_timewarp: true,
            },
            0.0,
            0.0,
            0,
            ProceduralInstallMode::Normal,
            1.0,
            0.0,
        );

        playback.update_procedural(lane, 1.0 / 60.0);
        playback.debug_draw_procedural(lane);

        assert_eq!(playback.runtime(), &["enter", "update", "debug"]);
    }
}
