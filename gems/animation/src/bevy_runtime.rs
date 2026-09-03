//! Bevy 0.19 evaluator adapter for Cry character animation and Mannequin.

use std::sync::Arc;

use bevy::{
    animation::{
        AnimatedBy, AnimationClip, AnimationPlayer, AnimationTargetId, animated_field,
        animation_curves::AnimatedField,
        graph::{
            AnimationGraph, AnimationGraphHandle, AnimationMask, AnimationNodeIndex,
            AnimationNodeType,
        },
    },
    asset::{AssetPath, AssetServer, Assets, Handle},
    ecs::hierarchy::ChildOf,
    gltf::{Gltf, GltfLoaderSettings, GltfNode},
    math::{Quat, Vec3, Vec4},
    prelude::{
        Added, AssetApp, Bundle, Commands, Component, Entity, FixedUpdate, IntoScheduleConfigs,
        Name, Plugin, PostUpdate, Query, Res, ResMut, Resource, SystemSet, Time, Update, With,
    },
    transform::components::Transform,
};

use az_animation::{
    animation_set::{AnimationProductRef, AnimationRef},
    blend_space::{
        BlendWeights, CombinedBlendSpace, DirectDeltaMotion, MAX_BLEND_SPACE_MOTIONS,
        MotionParameters, MotionTiming, ParametricBlendSpace, parameterized_normalized_delta,
    },
    blend_space_asset::{BlendSpaceAsset, BlendSpaceMotion, CombinedBlendSpaceAsset},
    character::{
        AnimationInstanceId, AnimationTimeStep, AnimationTransitionPolicy,
        CharacterAnimationParameters, CharacterAnimationRuntime, TransitionAnimation,
    },
    controller_target::{CONTROLLER_TARGET_ROOT_NAME, controller_target_path},
    mannequin::{
        ActionHandle, ActionMutation, ActiveAnimationState, AnimationLane, AnimationPlayback,
        AnimationStartParameters, ProceduralInstallMode, ProceduralLane, ProceduralPlayback,
    },
    motion::{
        AnimationDrivenMotionRequest, MotionParameterId, MotionParameterSink, RootMotionCommand,
        RootMotionDelta, RootMotionState,
    },
    playback::AnimationFlags,
};
use az_physics::{
    CharacterControllerCommands, PhysicsAction, PhysicsBodyHandle, PhysicsPose, PhysicsSet,
    PhysicsWorld,
};

/// A clip handle before the asset has loaded and its duration is validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BevyMotionSource {
    gltf: Handle<Gltf>,
    animation_index: usize,
    mask: AnimationMask,
}

fn load_motion_gltf(
    asset_server: &AssetServer,
    path: impl Into<AssetPath<'static>>,
) -> Handle<Gltf> {
    asset_server
        .load_builder()
        .with_settings(|settings: &mut GltfLoaderSettings| settings.include_source = true)
        .load(path)
}

#[must_use]
pub fn controller_animation_target_root_id() -> AnimationTargetId {
    AnimationTargetId::from_name(&Name::new(CONTROLLER_TARGET_ROOT_NAME))
}

#[must_use]
pub fn controller_animation_target_id(controller_id: u32) -> AnimationTargetId {
    let path = controller_target_path(controller_id).map(Name::new);
    AnimationTargetId::from_names(path.iter())
}

impl BevyMotionSource {
    #[must_use]
    pub fn new(gltf: impl Into<Handle<Gltf>>, animation_index: usize, mask: AnimationMask) -> Self {
        Self {
            gltf: gltf.into(),
            animation_index,
            mask,
        }
    }

    /// Requests one cooked motion product with the source document retained.
    ///
    /// Motion resolution reads controller bindings and root-motion metadata
    /// from the glTF document, so the default loader settings are insufficient.
    #[must_use]
    pub fn load(
        asset_server: &AssetServer,
        path: impl Into<AssetPath<'static>>,
        animation_index: usize,
        mask: AnimationMask,
    ) -> Self {
        Self::new(load_motion_gltf(asset_server, path), animation_index, mask)
    }

    /// # Errors
    ///
    /// [`MotionResolveError::Pending`] while the glTF and its clip are still
    /// loading, or a concrete variant when the product is malformed.
    pub fn resolve(
        &self,
        gltfs: &Assets<Gltf>,
        nodes: &Assets<GltfNode>,
        clips: &Assets<AnimationClip>,
    ) -> Result<BevyMotion, MotionResolveError> {
        self.resolve_clip(gltfs, nodes, clips)
            .map(BevyMotion::direct)
    }

    /// The clip half of [`Self::resolve`], shared with the parametric path so a
    /// blend-space example resolves exactly the way a direct motion does.
    ///
    /// # Errors
    ///
    /// [`MotionResolveError::Pending`] while the glTF and its clip are still
    /// loading, or a concrete variant when the product is malformed.
    pub fn resolve_clip(
        &self,
        gltfs: &Assets<Gltf>,
        nodes: &Assets<GltfNode>,
        clips: &Assets<AnimationClip>,
    ) -> Result<BevyClipMotion, MotionResolveError> {
        let gltf = gltfs.get(&self.gltf).ok_or(MotionResolveError::Pending)?;
        let clip_handle = gltf
            .animations
            .get(self.animation_index)
            .ok_or(MotionResolveError::MissingAnimation {
                index: self.animation_index,
            })?
            .clone();
        let clip = clips.get(&clip_handle).ok_or(MotionResolveError::Pending)?;
        let source = gltf
            .source
            .as_ref()
            .ok_or(MotionResolveError::MissingGltfDocument)?;
        let mut animation_roots = gltf
            .nodes
            .iter()
            .map(|handle| nodes.get(handle).ok_or(MotionResolveError::Pending))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|node| node.is_animation_root);
        let root = animation_roots
            .next()
            .ok_or(MotionResolveError::AnimationRootCount { found: 0 })?;
        if animation_roots.next().is_some() {
            return Err(MotionResolveError::AnimationRootCount {
                found: 2 + animation_roots.count(),
            });
        }
        let duration = az_animation::builder::animation_duration(source, self.animation_index)
            .unwrap_or_else(|| clip.duration());
        let controller_binding =
            az_animation::builder::animation_controller_binding(source, self.animation_index);
        let (root_target, root_node_index) = match controller_binding {
            Some(binding) if binding.uses_controller_targets() => {
                match binding.azoth_root_controller_id {
                    Some(controller_id) => (
                        controller_animation_target_id(controller_id),
                        az_animation::builder::animation_controller_node_index(
                            source,
                            controller_id,
                        )
                        .ok_or(MotionResolveError::MissingControllerNode { controller_id })?,
                    ),
                    None => (controller_animation_target_root_id(), root.index),
                }
            }
            _ => (
                AnimationTargetId::from_name(&Name::new(root.name.clone())),
                root.index,
            ),
        };
        let timing = az_animation::builder::root_motion_timing(
            source,
            self.animation_index,
            root_node_index,
            duration,
        )
        .ok_or(MotionResolveError::MissingRootMotionTiming)?;
        Ok(BevyClipMotion {
            clip: clip_handle,
            duration,
            mask: self.mask,
            root_target,
            timing,
            direct_delta_motion: DirectDeltaMotion::default(),
        })
    }
}

/// Why a motion source did not become a runtime motion on this attempt.
///
/// [`Self::Pending`] is the only recoverable case: it means Bevy has not
/// finished loading the products yet and the caller should ask again. Every
/// other variant is terminal for that source, because it describes a motion
/// product the animation builder should never have emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MotionResolveError {
    #[error("motion products are still loading")]
    Pending,
    #[error("motion product has no animation at index {index}")]
    MissingAnimation { index: usize },
    #[error("motion product carries no glTF document")]
    MissingGltfDocument,
    #[error("motion product must have exactly one animation root, found {found}")]
    AnimationRootCount { found: usize },
    #[error("motion product has no node for controller {controller_id}")]
    MissingControllerNode { controller_id: u32 },
    #[error("motion product has no extractable root-motion timing")]
    MissingRootMotionTiming,
    #[error("parametric motion could not be assembled: {0}")]
    InvalidParametricMotion(InvalidBevyMotion),
    #[error("parametric example {index} has an out-of-range playback scale")]
    InvalidPlaybackScale { index: usize },
}

impl MotionResolveError {
    /// Whether the caller should retry on a later frame rather than give up.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }
}

impl AsRef<Handle<Gltf>> for BevyMotionSource {
    fn as_ref(&self) -> &Handle<Gltf> {
        &self.gltf
    }
}

/// The runtime product path a compiled motion reference resolves to.
///
/// The blend-space compiler stores the authoring source path in the reference's
/// hint (`SourceReferenceResolver::motion`), and the animation builder renames
/// `*.anim.glb` to `*.motion.glb` when it emits the product
/// (`az_animation::builder::animation_product_path`).
#[must_use]
pub fn motion_product_path(motion: &AnimationRef) -> Option<String> {
    let AnimationProductRef::Motion(motion) = motion.product.as_ref()? else {
        return None;
    };
    Some(az_animation::builder::animation_product_path(
        motion.hint()?,
    ))
}

/// One blend-space example: the clip source plus the per-example metadata
/// `CryEngine` keeps on `m_arrParameter[i]`.
#[derive(Debug, Clone, PartialEq)]
struct BevyParametricExample {
    source: BevyMotionSource,
    /// `<Example PlaybackScale="...">` (`GlobalAnimationHeaderLMG.cpp:583`).
    playback_scale: f32,
    /// `<Example UseDirectlyForDeltaMotion0..3="...">`
    /// (`GlobalAnimationHeaderLMG.cpp:612-615`).
    direct_delta_motion: DirectDeltaMotion,
}

/// The sampler half of a parametric motion, held while its example clips load.
#[derive(Debug, Clone, PartialEq)]
enum BevyParametricSampler {
    BlendSpace(ParametricBlendSpace),
    Combined(CombinedBlendSpace),
}

/// A compiled blend space whose example clips have not loaded yet.
///
/// This is the parametric counterpart of [`BevyMotionSource`]: it holds the
/// compiled sampler and one clip source per example, and turns into a
/// [`BevyMotion`] once every example's glTF, nodes and clip are in `Assets`.
#[derive(Debug, Clone, PartialEq)]
pub struct BevyParametricSource {
    asset_id: az_core::AssetId,
    examples: Vec<BevyParametricExample>,
    sampler: BevyParametricSampler,
    timewarp_group: Option<Arc<str>>,
}

impl BevyParametricSource {
    /// Builds the pending motion for a compiled [`BlendSpaceAsset`].
    ///
    /// Each example's motion product is requested through `AssetServer::load`,
    /// the same way `LyShine` binds its image and atlas references
    /// (`gems/lyshine/src/render.rs`); Bevy deduplicates repeated loads of the
    /// same path, which matters because blend spaces reuse clips heavily.
    ///
    /// Returns `None` when any example carries no resolvable motion reference,
    /// because the sampler indexes clips positionally and a hole would shift
    /// every later example onto the wrong clip.
    #[must_use]
    pub fn from_blend_space(
        asset_id: az_core::AssetId,
        asset: &BlendSpaceAsset,
        asset_server: &AssetServer,
        mask: AnimationMask,
    ) -> Option<Self> {
        Self::from_blend_space_with(asset_id, asset, mask, |path| {
            load_motion_gltf(asset_server, path)
        })
    }

    /// [`Self::from_blend_space`] against an explicit loader, so the positional
    /// example wiring can be exercised without an `AssetServer`.
    #[must_use]
    pub fn from_blend_space_with(
        asset_id: az_core::AssetId,
        asset: &BlendSpaceAsset,
        mask: AnimationMask,
        load: impl FnMut(String) -> Handle<Gltf>,
    ) -> Option<Self> {
        Some(Self {
            asset_id,
            examples: parametric_examples(&asset.motions, mask, load)?,
            sampler: BevyParametricSampler::BlendSpace(asset.sampler.clone()),
            timewarp_group: asset.timewarp_group.as_deref().map(Arc::from),
        })
    }

    /// Builds the pending motion for a compiled [`CombinedBlendSpaceAsset`].
    ///
    /// `CombinedBlendSpaceCompiler` deduplicates the child blend spaces' motions
    /// into one list and rewrites each sub-space's `example_indices` against it,
    /// so the positional clip order below is the order
    /// `CombinedBlendSpace::evaluate` writes weights in.
    #[must_use]
    pub fn from_combined_blend_space(
        asset_id: az_core::AssetId,
        asset: &CombinedBlendSpaceAsset,
        asset_server: &AssetServer,
        mask: AnimationMask,
    ) -> Option<Self> {
        Self::from_combined_blend_space_with(asset_id, asset, mask, |path| {
            load_motion_gltf(asset_server, path)
        })
    }

    /// [`Self::from_combined_blend_space`] against an explicit loader.
    #[must_use]
    pub fn from_combined_blend_space_with(
        asset_id: az_core::AssetId,
        asset: &CombinedBlendSpaceAsset,
        mask: AnimationMask,
        load: impl FnMut(String) -> Handle<Gltf>,
    ) -> Option<Self> {
        Some(Self {
            asset_id,
            examples: parametric_examples(&asset.motions, mask, load)?,
            sampler: BevyParametricSampler::Combined(asset.sampler.clone()),
            timewarp_group: None,
        })
    }

    /// The glTF handles this motion is waiting on, for load-readiness checks.
    pub fn sources(&self) -> impl Iterator<Item = &Handle<Gltf>> + '_ {
        self.examples.iter().map(|example| example.source.as_ref())
    }

    #[must_use]
    pub const fn asset_id(&self) -> az_core::AssetId {
        self.asset_id
    }

    /// Resolves every example into its clip and assembles the parametric
    /// motion. Reports [`MotionResolveError::Pending`] until all of them have
    /// loaded, which keeps the positional clip order intact.
    ///
    /// # Errors
    ///
    /// [`MotionResolveError::Pending`] while any example is still loading, or
    /// the first concrete failure any example reports.
    pub fn resolve(
        &self,
        gltfs: &Assets<Gltf>,
        nodes: &Assets<GltfNode>,
        clips: &Assets<AnimationClip>,
    ) -> Result<BevyMotion, MotionResolveError> {
        let mut resolved = Vec::with_capacity(self.examples.len());
        for example in &self.examples {
            resolved.push(example.source.resolve_clip(gltfs, nodes, clips)?);
        }
        self.assemble(resolved)
    }

    /// Applies each example's authored metadata to its resolved clip and builds
    /// the parametric motion. Split out of [`Self::resolve`] so the metadata and
    /// ordering rules can be tested without loading real glTF assets.
    fn assemble(&self, clips: Vec<BevyClipMotion>) -> Result<BevyMotion, MotionResolveError> {
        if clips.len() != self.examples.len() {
            return Err(MotionResolveError::InvalidParametricMotion(
                InvalidBevyMotion::ExampleCount {
                    actual: clips.len(),
                    expected: self.examples.len(),
                },
            ));
        }
        let mut resolved = Vec::with_capacity(clips.len());
        for (index, (example, clip)) in self.examples.iter().zip(clips).enumerate() {
            // CryEngine scales an example's playback speed and routes its
            // authored delta-motion flags into the sampler, so both belong on
            // the clip rather than on the shared motion.
            resolved.push(
                clip.with_parametric_metadata(example.playback_scale, example.direct_delta_motion)
                    .ok_or(MotionResolveError::InvalidPlaybackScale { index })?,
            );
        }
        let motion = match &self.sampler {
            BevyParametricSampler::BlendSpace(sampler) => {
                BevyMotion::blend_space(sampler.clone(), resolved)
            }
            BevyParametricSampler::Combined(sampler) => {
                BevyMotion::combined_blend_space(sampler.clone(), resolved)
            }
        }
        .map_err(MotionResolveError::InvalidParametricMotion)?
        .with_asset_id(self.asset_id);
        Ok(match &self.timewarp_group {
            Some(group) => motion.with_timewarp_group(Arc::clone(group)),
            None => motion,
        })
    }
}

fn parametric_examples(
    motions: &[BlendSpaceMotion],
    mask: AnimationMask,
    mut load: impl FnMut(String) -> Handle<Gltf>,
) -> Option<Vec<BevyParametricExample>> {
    motions
        .iter()
        .map(|motion| {
            let path = motion_product_path(&motion.animation)?;
            Some(BevyParametricExample {
                // Every motion product carries exactly one animation, so the
                // example always binds animation zero.
                source: BevyMotionSource::new(load(path), 0, mask),
                playback_scale: motion.playback_scale,
                direct_delta_motion: motion.direct_delta_motion,
            })
        })
        .collect()
}

/// One fully resolved clip in a direct or parametric motion.
#[derive(Debug, Clone, PartialEq)]
pub struct BevyClipMotion {
    clip: Handle<AnimationClip>,
    duration: f32,
    mask: AnimationMask,
    root_target: AnimationTargetId,
    timing: MotionTiming,
    direct_delta_motion: DirectDeltaMotion,
}

impl BevyClipMotion {
    #[must_use]
    pub const fn clip(&self) -> &Handle<AnimationClip> {
        &self.clip
    }

    #[must_use]
    pub const fn duration(&self) -> f32 {
        self.duration
    }

    #[must_use]
    pub const fn mask(&self) -> AnimationMask {
        self.mask
    }

    #[must_use]
    pub const fn root_target(&self) -> AnimationTargetId {
        self.root_target
    }

    #[must_use]
    pub const fn timing(&self) -> MotionTiming {
        self.timing
    }

    #[must_use]
    pub const fn direct_delta_motion(&self) -> DirectDeltaMotion {
        self.direct_delta_motion
    }

    #[must_use]
    pub fn with_parametric_metadata(
        mut self,
        playback_scale: f32,
        direct_delta_motion: DirectDeltaMotion,
    ) -> Option<Self> {
        self.timing = self.timing.with_playback_scale(playback_scale)?;
        self.direct_delta_motion = direct_delta_motion;
        Some(self)
    }
}

impl AsRef<Handle<AnimationClip>> for BevyClipMotion {
    fn as_ref(&self) -> &Handle<AnimationClip> {
        &self.clip
    }
}

#[derive(Debug, Clone, PartialEq)]
enum BevyMotionSampler {
    Direct,
    BlendSpace(ParametricBlendSpace),
    CombinedBlendSpace(CombinedBlendSpace),
}

/// Fully resolved motion key consumed by the hot playback path.
#[derive(Debug, Clone, PartialEq)]
pub struct BevyMotion {
    asset_id: Option<az_core::AssetId>,
    clips: Arc<[BevyClipMotion]>,
    timings: Arc<[MotionTiming]>,
    sampler: BevyMotionSampler,
    nominal_duration: f32,
    timewarp_group: Option<Arc<str>>,
}

/// Per-instance state owned by one queued direct or parametric motion.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BevyMotionState {
    parameters: MotionParameters,
    weights: BlendWeights,
}

impl BevyMotionState {
    #[must_use]
    pub const fn parameters(&self) -> &MotionParameters {
        &self.parameters
    }

    #[must_use]
    pub const fn weights(&self) -> &BlendWeights {
        &self.weights
    }
}

pub type BevyTransitionAnimation = TransitionAnimation<BevyMotion, BevyMotionState>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionEventWindow {
    pub previous: f32,
    pub current: f32,
    pub cycles: u32,
    pub include_start: bool,
}

impl BevyMotion {
    #[must_use]
    pub fn direct(clip: BevyClipMotion) -> Self {
        let timing = clip.timing;
        Self {
            asset_id: None,
            nominal_duration: clip.duration,
            clips: Arc::from([clip]),
            timings: Arc::from([timing]),
            sampler: BevyMotionSampler::Direct,
            timewarp_group: None,
        }
    }

    /// Builds a parametric motion from a compiled blend-space sampler and the
    /// clip of every one of its examples, in example order.
    ///
    /// # Errors
    ///
    /// [`InvalidBevyMotion::ExampleCount`] when the number of clips does not
    /// match the sampler's example count, because the sampler addresses its
    /// clips positionally.
    pub fn blend_space(
        sampler: ParametricBlendSpace,
        clips: impl IntoIterator<Item = BevyClipMotion>,
    ) -> Result<Self, InvalidBevyMotion> {
        Self::parametric(BevyMotionSampler::BlendSpace(sampler), clips)
    }

    /// [`Self::blend_space`] for a sampler assembled from sub-spaces.
    ///
    /// # Errors
    ///
    /// [`InvalidBevyMotion::ExampleCount`] when the number of clips does not
    /// match the sampler's example count, because the sampler addresses its
    /// clips positionally.
    pub fn combined_blend_space(
        sampler: CombinedBlendSpace,
        clips: impl IntoIterator<Item = BevyClipMotion>,
    ) -> Result<Self, InvalidBevyMotion> {
        Self::parametric(BevyMotionSampler::CombinedBlendSpace(sampler), clips)
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "the clip count equals the sampler's validated example count, which is bounded \
                  by MAX_BLEND_SPACE_MOTIONS, so it is exactly representable in f32"
    )]
    fn parametric(
        sampler: BevyMotionSampler,
        clips: impl IntoIterator<Item = BevyClipMotion>,
    ) -> Result<Self, InvalidBevyMotion> {
        let clips = clips.into_iter().collect::<Vec<_>>();
        let expected = match &sampler {
            BevyMotionSampler::Direct => 1,
            BevyMotionSampler::BlendSpace(sampler) => sampler.example_count(),
            BevyMotionSampler::CombinedBlendSpace(sampler) => sampler.example_count(),
        };
        if clips.len() != expected {
            return Err(InvalidBevyMotion::ExampleCount {
                actual: clips.len(),
                expected,
            });
        }
        let nominal_duration =
            clips.iter().map(|clip| clip.duration).sum::<f32>() / clips.len() as f32;
        let timings = clips.iter().map(|clip| clip.timing).collect::<Vec<_>>();
        Ok(Self {
            asset_id: None,
            clips: clips.into(),
            timings: timings.into(),
            sampler,
            nominal_duration,
            timewarp_group: None,
        })
    }

    #[must_use]
    pub fn direct_clip(&self) -> Option<&BevyClipMotion> {
        matches!(self.sampler, BevyMotionSampler::Direct).then(|| &self.clips[0])
    }

    #[must_use]
    pub fn clips(&self) -> &[BevyClipMotion] {
        &self.clips
    }

    #[must_use]
    pub const fn asset_id(&self) -> Option<az_core::AssetId> {
        self.asset_id
    }

    #[must_use]
    pub const fn with_asset_id(mut self, asset_id: az_core::AssetId) -> Self {
        self.asset_id = Some(asset_id);
        self
    }

    #[must_use]
    pub fn with_timewarp_group(mut self, group: impl Into<Arc<str>>) -> Self {
        self.timewarp_group = Some(group.into());
        self
    }

    #[must_use]
    pub fn timewarp_group(&self) -> Option<&str> {
        self.timewarp_group.as_deref()
    }

    #[must_use]
    pub fn event_window(&self, animation: &BevyTransitionAnimation) -> Option<MotionEventWindow> {
        let (previous, current, cycles) = match &self.sampler {
            BevyMotionSampler::Direct => {
                let timing = self.clips.first()?.timing;
                let previous = timing.normalized_time(
                    sample_segment_index_at(animation, timing, animation.previous_segment_index()),
                    animation.previous_normalized_time(),
                )?;
                let current = timing.normalized_time(
                    sample_segment_index(animation, timing),
                    animation.normalized_time(),
                )?;
                (previous, current, animation.loops_this_update())
            }
            BevyMotionSampler::BlendSpace(_) | BevyMotionSampler::CombinedBlendSpace(_) => (
                animation.previous_normalized_time(),
                animation.normalized_time(),
                animation.segment_advances_this_update(),
            ),
        };
        Some(MotionEventWindow {
            previous,
            current,
            cycles,
            include_start: animation.is_first_evaluation(),
        })
    }

    #[must_use]
    pub const fn duration(&self) -> f32 {
        self.nominal_duration
    }

    /// The per-instance state a fresh queue entry starts with.
    ///
    /// A direct motion pins its single clip's weight immediately, the way
    /// `CryEngine` leaves `GetParametricSampler() == NULL` and samples the one
    /// CAF; a parametric motion starts with no weights and gets them from its
    /// first `Parameterizer` pass (`ParametricSampler.cpp:27`).
    #[must_use]
    pub fn initial_state(&self) -> BevyMotionState {
        let mut state = BevyMotionState::default();
        if matches!(self.sampler, BevyMotionSampler::Direct) {
            state.weights.set_direct();
        }
        state
    }

    fn evaluate(&self, parameters: &MotionParameters, weights: &mut BlendWeights) {
        match &self.sampler {
            BevyMotionSampler::Direct => weights.set_direct(),
            BevyMotionSampler::BlendSpace(sampler) => sampler.evaluate(parameters, weights),
            BevyMotionSampler::CombinedBlendSpace(sampler) => {
                sampler.evaluate(parameters, weights);
            }
        }
    }

    /// One frame of `CSkeletonAnim::UpdateParameters`
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_BlendMan.cpp`).
    ///
    /// The two sampler kinds do not share a clock. A direct CAF divides the
    /// frame delta by its own current segment's duration
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_BlendMan.cpp:328`:
    /// `m_fCurrentDeltaTime = (fFrameDeltaTime * m_fPlaybackScale) / fSegTime`)
    /// and keeps the total duration it was given when it was queued
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/SkeletonAnim_Queue.cpp:487`).
    /// A parametric group instead runs
    /// `SParametricSamplerInternal::Parameterizer`'s speed-over-distance
    /// time warp (`ParametricSampler.cpp:181-282`) and recomputes both expected
    /// durations every frame (`SkeletonAnim_BlendMan.cpp:306-313`).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a segment index addresses one of the motion's segments, and this runtime \
                  carries segment counts and indices as u8 (`AnimationTimeStep::segment_count`, \
                  `segment_indices`), so the resolved index is bounded by u8::MAX"
    )]
    fn time_step(
        &self,
        state: &mut BevyMotionState,
        segment_index: usize,
        looping: bool,
        playback_scale: f32,
        delta_time: f32,
    ) -> Option<AnimationTimeStep> {
        self.evaluate(&state.parameters, &mut state.weights);
        let segment_count = u8::try_from(
            state
                .weights
                .active()
                .map(|(index, _)| self.timings[index].segment_count())
                .max()?,
        )
        .unwrap_or(u8::MAX);

        if matches!(self.sampler, BevyMotionSampler::Direct) {
            let timing = *self.timings.first()?;
            let expected_segment_duration = timing
                .clock_segment_duration(resolved_sample_segment(segment_index, looping, timing))?;
            return Some(AnimationTimeStep {
                normalized_delta: playback_scale * delta_time / expected_segment_duration,
                expected_segment_duration,
                expected_total_duration: timing.duration(),
                segment_count,
            });
        }

        let mut segment_indices = [0u8; MAX_BLEND_SPACE_MOTIONS];
        for (index, _) in state.weights.active() {
            segment_indices[index] =
                resolved_sample_segment(segment_index, looping, self.timings[index]) as u8;
        }
        let normalized_delta = parameterized_normalized_delta(
            delta_time,
            &state.weights,
            &self.timings,
            &segment_indices[..self.timings.len()],
        )?;
        let expected_segment_duration = state
            .weights
            .active()
            .map(|(index, weight)| {
                let duration = self.timings[index]
                    .clock_segment_duration(resolved_sample_segment(
                        segment_index,
                        looping,
                        self.timings[index],
                    ))
                    .expect("active motion segment was validated");
                weight * duration
            })
            .sum::<f32>()
            .max(0.0001);
        let weighted_total_duration = state
            .weights
            .active()
            .map(|(index, weight)| weight * self.timings[index].duration())
            .sum::<f32>();
        let scaled_delta = normalized_delta * playback_scale;
        let expected_total_duration = if scaled_delta > 0.0 {
            weighted_total_duration
                * (delta_time / (scaled_delta * expected_segment_duration)).max(0.0)
        } else {
            weighted_total_duration
        };
        Some(AnimationTimeStep {
            normalized_delta: scaled_delta,
            expected_segment_duration,
            expected_total_duration,
            segment_count,
        })
    }

    const fn is_idle_to_move(&self) -> bool {
        match &self.sampler {
            BevyMotionSampler::Direct => false,
            BevyMotionSampler::BlendSpace(sampler) => sampler.is_idle_to_move(),
            BevyMotionSampler::CombinedBlendSpace(sampler) => sampler.is_idle_to_move(),
        }
    }

    fn has_parameter(&self, parameter: MotionParameterId) -> bool {
        match &self.sampler {
            BevyMotionSampler::Direct => false,
            BevyMotionSampler::BlendSpace(sampler) => sampler
                .dimensions()
                .iter()
                .any(|dimension| dimension.parameter == parameter),
            BevyMotionSampler::CombinedBlendSpace(sampler) => sampler
                .dimensions()
                .iter()
                .any(|dimension| dimension.parameter == parameter),
        }
    }

    fn parameter_is_locked(&self, parameter: MotionParameterId) -> Option<bool> {
        match &self.sampler {
            BevyMotionSampler::Direct => None,
            BevyMotionSampler::BlendSpace(sampler) => sampler
                .dimensions()
                .iter()
                .find(|dimension| dimension.parameter == parameter)
                .map(|dimension| dimension.locked),
            BevyMotionSampler::CombinedBlendSpace(sampler) => sampler
                .dimensions()
                .iter()
                .find(|dimension| dimension.parameter == parameter)
                .map(|dimension| dimension.locked),
        }
    }

    fn reference_timing(&self, parameters: &MotionParameters) -> Option<MotionTiming> {
        let mut weights = BlendWeights::default();
        self.evaluate(parameters, &mut weights);
        weights
            .active()
            .max_by_key(|(index, _)| self.timings[*index].segment_count())
            .map(|(index, _)| self.timings[index])
    }

    fn entire_normalized_time(
        &self,
        parameters: &MotionParameters,
        segment: usize,
        phase: f32,
    ) -> Option<f32> {
        let timing = self.reference_timing(parameters)?;
        timing.normalized_time(segment.min(timing.segment_count() - 1), phase)
    }

    fn segment_time_from_entire(
        &self,
        parameters: &MotionParameters,
        normalized_time: f32,
    ) -> Option<(usize, f32)> {
        let timing = self.reference_timing(parameters)?;
        let normalized_time = normalized_time.clamp(0.0, 1.0);
        for segment_index in 0..timing.segment_count() {
            let segment = timing.segment(segment_index)?;
            if normalized_time <= segment.normalized_end {
                let width = segment.normalized_end - segment.normalized_start;
                return Some((
                    segment_index,
                    ((normalized_time - segment.normalized_start) / width).clamp(0.0, 1.0),
                ));
            }
        }
        None
    }
}

fn sample_segment_index(animation: &BevyTransitionAnimation, timing: MotionTiming) -> usize {
    sample_segment_index_at(animation, timing, animation.segment_index())
}

fn sample_segment_index_at(
    animation: &BevyTransitionAnimation,
    timing: MotionTiming,
    segment_index: usize,
) -> usize {
    resolved_sample_segment(
        segment_index,
        animation.flags().contains(AnimationFlags::LOOP_ANIMATION),
        timing,
    )
}

fn resolved_sample_segment(segment_index: usize, looping: bool, timing: MotionTiming) -> usize {
    let segment_count = timing.segment_count();
    if looping {
        segment_index % segment_count
    } else {
        segment_index.min(segment_count - 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidBevyMotion {
    #[error("parametric motion has {actual} clips; expected {expected}")]
    ExampleCount { actual: usize, expected: usize },
}

#[derive(Debug, Clone, Copy)]
struct GraphBinding {
    node: AnimationNodeIndex,
    layer: u32,
    instance: Option<AnimationInstanceId>,
    sample: u8,
    used: bool,
}

/// Cry's authoritative animation clock and FIFO for one Bevy animation root.
///
/// Bevy nodes are evaluated while paused; this component advances normalized
/// time itself, so client rendering and headless/server simulation share the
/// same queue and timing rules.
#[derive(Component, Debug, Clone, Default)]
pub struct CryAnimationPlayer {
    runtime: CharacterAnimationRuntime<BevyMotion, BevyMotionState>,
    bindings: Vec<GraphBinding>,
}

struct BevyTransitionPolicy;

impl AnimationTransitionPolicy<BevyMotion, BevyMotionState> for BevyTransitionPolicy {
    fn is_ready(&self, _animation: &BevyMotion) -> bool {
        true
    }

    fn animation_time_step(
        &mut self,
        animation: &mut BevyTransitionAnimation,
        delta_time: f32,
    ) -> Option<AnimationTimeStep> {
        let motion = animation.animation().clone();
        let segment_index = animation.segment_index();
        let looping = animation.flags().contains(AnimationFlags::LOOP_ANIMATION);
        let playback_scale = animation.playback_scale();
        motion.time_step(
            animation.state_mut(),
            segment_index,
            looping,
            playback_scale,
            delta_time,
        )
    }

    fn idle_to_move_ready(
        &self,
        previous: &BevyTransitionAnimation,
        _next: &BevyTransitionAnimation,
    ) -> bool {
        if !previous.animation().is_idle_to_move() {
            return true;
        }
        assert!(
            previous
                .animation()
                .has_parameter(MotionParameterId::TurnAngle),
            "idle-to-move blend space requires a TurnAngle dimension"
        );
        if previous.segment_index() == 0 {
            return false;
        }
        previous
            .state()
            .parameters
            .get_or_default(MotionParameterId::TurnAngle)
            > 0.0
            || previous.normalized_time() > 0.5
    }

    fn entire_normalized_time(&self, animation: &BevyTransitionAnimation) -> Option<f32> {
        animation.animation().entire_normalized_time(
            &animation.state().parameters,
            animation.segment_index(),
            animation.normalized_time(),
        )
    }

    fn synchronize_animation_state(
        &mut self,
        previous: &BevyTransitionAnimation,
        next: &mut BevyTransitionAnimation,
    ) {
        *next.state_mut() = *previous.state();
    }

    fn shares_timewarp_group(
        &self,
        previous: &BevyTransitionAnimation,
        next: &BevyTransitionAnimation,
    ) -> bool {
        previous
            .animation()
            .timewarp_group()
            .zip(next.animation().timewarp_group())
            .is_some_and(|(previous, next)| previous.eq_ignore_ascii_case(next))
    }
}

impl CryAnimationPlayer {
    #[must_use]
    pub const fn runtime(&self) -> &CharacterAnimationRuntime<BevyMotion, BevyMotionState> {
        &self.runtime
    }

    pub const fn runtime_mut(
        &mut self,
    ) -> &mut CharacterAnimationRuntime<BevyMotion, BevyMotionState> {
        &mut self.runtime
    }

    pub fn advance(&mut self, delta_time: f32) {
        self.runtime.update(delta_time, &mut BevyTransitionPolicy);
    }

    /// Returns the newest active parametric motion's desired value.
    #[must_use]
    pub fn desired_motion_parameter(&self, parameter: MotionParameterId) -> Option<f32> {
        for layer in self.runtime.layers() {
            let active_count = layer
                .executed_animations()
                .iter()
                .filter(|animation| animation.is_activated())
                .count();
            for animation in layer.executed_animations()[..active_count].iter().rev() {
                if animation.animation().has_parameter(parameter) {
                    return Some(animation.state().parameters.get_or_default(parameter));
                }
            }
        }
        None
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "layer indices are bounded by az_animation::character::ANIMATION_LAYER_COUNT \
                  and blend-space sample indices by MAX_BLEND_SPACE_MOTIONS"
    )]
    fn apply(&mut self, graph: &mut AnimationGraph, player: &mut AnimationPlayer) {
        let runtime = &self.runtime;
        let bindings = &mut self.bindings;
        for binding in bindings.iter_mut() {
            binding.used = false;
        }
        for layer_index in 0..az_animation::character::ANIMATION_LAYER_COUNT {
            let layer = &runtime.layers()[layer_index];
            for animation in layer.active_animations() {
                let instance = animation.id();
                let effective_weight = runtime.effective_weight(layer_index as u32, animation);
                let motion = animation.animation();
                for (sample_index, sample_weight) in animation.state().weights.active() {
                    let sample_index = sample_index as u8;
                    let clip = &motion.clips[usize::from(sample_index)];
                    let node = match bindings.iter_mut().find(|binding| {
                        binding.instance == Some(instance) && binding.sample == sample_index
                    }) {
                        Some(binding) => {
                            binding.used = true;
                            binding.node
                        }
                        None => Self::bind_animation(
                            bindings,
                            layer_index as u32,
                            instance,
                            sample_index,
                            clip,
                            graph,
                            player,
                        ),
                    };
                    let active = player
                        .animation_mut(node)
                        .expect("bound Cry animation must be active in Bevy");
                    active
                        .set_weight(effective_weight * sample_weight)
                        .set_speed(0.0)
                        .set_seek_time(
                            clip.timing
                                .normalized_time(
                                    sample_segment_index(animation, clip.timing),
                                    animation.normalized_time(),
                                )
                                .expect("active motion segment was validated")
                                * clip.duration(),
                        )
                        .pause();
                }
            }
        }

        for binding in bindings {
            if binding.instance.is_none() || binding.used {
                continue;
            }
            player.stop(binding.node);
            binding.instance = None;
        }
    }

    #[must_use]
    pub fn calculate_relative_movement(
        &self,
        clips: &Assets<AnimationClip>,
        frame_delta_time: f32,
    ) -> RootMotionDelta {
        let Some(layer) = self.runtime.layer(0) else {
            return RootMotionDelta::default();
        };
        let active = layer.active_animations();
        if active.is_empty() {
            return RootMotionDelta::default();
        }

        let priority_start = active
            .iter()
            .enumerate()
            .filter(|(_, animation)| {
                animation
                    .flags()
                    .contains(AnimationFlags::FULL_ROOT_PRIORITY)
                    && animation.transition_weight() != 0.0
            })
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or_default();
        let contributors = &active[priority_start..];
        let mut weight_sum = 0.0f32;
        for animation in contributors {
            for (_, sample_weight) in animation.state().weights.active() {
                let weight = animation.transition_weight() * sample_weight;
                if weight.abs() >= 0.001 {
                    weight_sum += weight;
                }
            }
        }
        if weight_sum < 0.01 {
            return RootMotionDelta::default();
        }

        let mut extracted = ExtractedRootMotion::default();
        let frame_playback_scale = frame_delta_time * layer.layer_playback_scale();
        for animation in contributors {
            let motion = animation.animation();
            for (sample_index, sample_weight) in animation.state().weights.active() {
                let weight = animation.transition_weight() * sample_weight;
                if weight.abs() < 0.001 {
                    continue;
                }
                let sample = &motion.clips[sample_index];
                let Some(clip) = clips.get(sample.clip()) else {
                    continue;
                };
                let Some(delta) = animation_root_delta(animation, sample, clip) else {
                    continue;
                };
                let weight = weight / weight_sum;
                match &motion.sampler {
                    BevyMotionSampler::Direct => extracted.accumulate_direct(delta, weight),
                    BevyMotionSampler::BlendSpace(sampler) => {
                        extracted.accumulate_parametric(
                            delta,
                            weight,
                            animation_delta_seconds(animation, sample),
                            frame_playback_scale,
                            animation.playback_scale() * sample.timing.playback_scale(),
                            sampler
                                .dimensions()
                                .iter()
                                .enumerate()
                                .map(|(index, dimension)| {
                                    (dimension.parameter, sample.direct_delta_motion.get(index))
                                })
                                .chain(
                                    sampler
                                        .additional_extraction()
                                        .iter()
                                        .copied()
                                        .map(|parameter| (parameter, None)),
                                ),
                        );
                    }
                    BevyMotionSampler::CombinedBlendSpace(sampler) => {
                        extracted.accumulate_parametric(
                            delta,
                            weight,
                            animation_delta_seconds(animation, sample),
                            frame_playback_scale,
                            animation.playback_scale() * sample.timing.playback_scale(),
                            sampler
                                .dimensions()
                                .iter()
                                .map(|dimension| (dimension.parameter, None))
                                .chain(
                                    sampler
                                        .additional_extraction()
                                        .iter()
                                        .copied()
                                        .map(|parameter| (parameter, None)),
                                ),
                        );
                    }
                }
            }
        }
        extracted.finish()
    }

    pub fn set_top_animation_flags(&mut self, layer: u32, flags: AnimationFlags) -> bool {
        self.runtime.set_top_animation_flags(layer, flags)
    }

    fn bind_animation(
        bindings: &mut Vec<GraphBinding>,
        layer: u32,
        instance: AnimationInstanceId,
        sample: u8,
        motion: &BevyClipMotion,
        graph: &mut AnimationGraph,
        player: &mut AnimationPlayer,
    ) -> AnimationNodeIndex {
        let node = if let Some(binding) = bindings
            .iter_mut()
            .find(|binding| binding.layer == layer && binding.instance.is_none())
        {
            let node = graph
                .graph
                .node_weight_mut(binding.node)
                .expect("Cry animation graph binding must remain stable");
            node.node_type = AnimationNodeType::Clip(motion.clip.clone());
            node.mask = motion.mask;
            node.weight = 1.0;
            binding.instance = Some(instance);
            binding.sample = sample;
            binding.used = true;
            binding.node
        } else {
            let node = graph.add_clip_with_mask(motion.clip.clone(), motion.mask, 1.0, graph.root);
            bindings.push(GraphBinding {
                node,
                layer,
                instance: Some(instance),
                sample,
                used: true,
            });
            node
        };
        player.start(node).set_speed(0.0).pause();
        node
    }
}

#[derive(Debug, Clone, Copy)]
struct RootPose {
    rotation: Quat,
    translation: Vec3,
}

impl RootPose {
    fn inverse(self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            rotation,
            translation: rotation * -self.translation,
        }
    }

    fn compose(self, other: Self) -> Self {
        Self {
            rotation: normalize_quat_or_identity(self.rotation * other.rotation),
            translation: self.rotation * other.translation + self.translation,
        }
    }
}

#[inline]
fn normalize_quat_or_identity(rotation: Quat) -> Quat {
    Quat::from_vec4(Vec4::from(rotation).try_normalize().unwrap_or(Vec4::W))
}

#[derive(Debug, Clone, Copy, Default)]
struct ExtractedRootMotion {
    move_speed: f32,
    move_distance: f32,
    turn_speed: f32,
    turn_distance: f32,
    travel_direction: f32,
    slope: f32,
    translation: Vec3,
}

impl ExtractedRootMotion {
    #[expect(
        clippy::suboptimal_flops,
        reason = "bit-exact CryEngine animation math; fusing the multiply and add changes the \
                  rounding and therefore the compiled/played animation"
    )]
    fn accumulate_direct(&mut self, delta: RootPose, weight: f32) {
        self.move_speed += weight * delta.translation.length();
        self.turn_speed += weight * yaw(delta.rotation);
        self.travel_direction += weight * travel_direction(delta.translation);
        self.slope += weight * travel_slope(delta.translation);
        self.translation += weight * delta.translation;
    }

    #[expect(
        clippy::suboptimal_flops,
        reason = "bit-exact CryEngine animation math; fusing the multiply and add changes the \
                  rounding and therefore the compiled/played animation"
    )]
    #[expect(
        clippy::if_not_else,
        reason = "`animation_delta != 0.0` is the ported divide-by-zero guard: the non-zero \
                  case is the real path and inverting it would hide that"
    )]
    fn accumulate_parametric(
        &mut self,
        delta: RootPose,
        weight: f32,
        animation_delta: f32,
        frame_playback_scale: f32,
        sample_playback_scale: f32,
        parameters: impl IntoIterator<Item = (MotionParameterId, Option<f32>)>,
    ) {
        for (parameter, direct) in parameters {
            match parameter {
                MotionParameterId::TravelSpeed => {
                    self.move_speed += weight
                        * direct.unwrap_or_else(|| {
                            if animation_delta != 0.0 {
                                delta.translation.length() / animation_delta * sample_playback_scale
                            } else {
                                0.0
                            }
                        })
                        * frame_playback_scale;
                }
                MotionParameterId::TravelDistance => {
                    self.move_distance += weight
                        * direct.map_or_else(
                            || delta.translation.length(),
                            |value| value * frame_playback_scale,
                        );
                }
                MotionParameterId::TurnSpeed => {
                    self.turn_speed += weight
                        * direct.unwrap_or_else(|| {
                            if animation_delta != 0.0 {
                                yaw(delta.rotation) / animation_delta * sample_playback_scale
                            } else {
                                0.0
                            }
                        })
                        * frame_playback_scale;
                }
                MotionParameterId::TurnAngle => {
                    self.turn_distance += weight
                        * direct.map_or_else(
                            || yaw(delta.rotation),
                            |value| value * frame_playback_scale,
                        );
                }
                MotionParameterId::TravelAngle => {
                    self.travel_direction +=
                        weight * direct.unwrap_or_else(|| travel_direction(delta.translation));
                }
                MotionParameterId::TravelSlope => {
                    self.slope +=
                        weight * direct.unwrap_or_else(|| travel_slope(delta.translation));
                }
                MotionParameterId::StopLeg
                | MotionParameterId::BlendWeight
                | MotionParameterId::BlendWeight2
                | MotionParameterId::BlendWeight3
                | MotionParameterId::BlendWeight4
                | MotionParameterId::AimHorizontalNavigationSpeed
                | MotionParameterId::AimHorizontalNavigationAngle
                | MotionParameterId::DesiredFacing
                | MotionParameterId::VelocityX
                | MotionParameterId::VelocityY
                | MotionParameterId::SlopeYaw
                | MotionParameterId::SlopePitch => {}
            }
        }
        self.translation += weight * delta.translation;
    }

    fn finish(self) -> RootMotionDelta {
        let move_distance = self.move_speed + self.move_distance;
        let turn_angle = self.turn_speed + self.turn_distance;
        let mut travel_direction = self.travel_direction;
        if self.translation.y < 0.0 {
            travel_direction = if self.translation.x < 0.0 {
                std::f32::consts::PI - travel_direction
            } else {
                -std::f32::consts::PI - travel_direction
            };
        }
        RootMotionDelta {
            rotation: Quat::from_rotation_z(turn_angle),
            translation: Quat::from_rotation_z(travel_direction)
                * (Quat::from_rotation_x(self.slope) * Vec3::new(0.0, move_distance, 0.0)),
        }
    }
}

fn animation_root_delta(
    animation: &BevyTransitionAnimation,
    motion: &BevyClipMotion,
    clip: &AnimationClip,
) -> Option<RootPose> {
    let previous_normalized_time = motion.timing.normalized_time(
        sample_segment_index_at(animation, motion.timing, animation.previous_segment_index()),
        animation.previous_normalized_time(),
    )?;
    let normalized_time = motion.timing.normalized_time(
        sample_segment_index(animation, motion.timing),
        animation.normalized_time(),
    )?;
    let previous = sample_root_pose(
        clip,
        motion.root_target,
        previous_normalized_time * motion.duration,
    )?;
    let current = sample_root_pose(clip, motion.root_target, normalized_time * motion.duration)?;
    let loops = sample_loops_this_update(animation, motion.timing);
    if loops == 0 {
        return Some(previous.inverse().compose(current));
    }

    let start = sample_root_pose(clip, motion.root_target, 0.0)?;
    let end = sample_root_pose(clip, motion.root_target, motion.duration)?;
    let mut delta = previous.inverse().compose(end);
    let cycle = start.inverse().compose(end);
    for _ in 1..loops {
        delta = delta.compose(cycle);
    }
    Some(delta.compose(start.inverse().compose(current)))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "`loops` counts the segment wraps inside a single frame, a handful at most, so it \
              is far inside f32's exactly representable integer range"
)]
fn animation_delta_seconds(animation: &BevyTransitionAnimation, motion: &BevyClipMotion) -> f32 {
    let previous_normalized_time = motion
        .timing
        .normalized_time(
            sample_segment_index_at(animation, motion.timing, animation.previous_segment_index()),
            animation.previous_normalized_time(),
        )
        .unwrap_or_default();
    let normalized_time = motion
        .timing
        .normalized_time(
            sample_segment_index(animation, motion.timing),
            animation.normalized_time(),
        )
        .unwrap_or(previous_normalized_time);
    let loops = sample_loops_this_update(animation, motion.timing);
    let normalized_delta = if loops == 0 {
        normalized_time - previous_normalized_time
    } else {
        loops as f32 + normalized_time - previous_normalized_time
    };
    normalized_delta * motion.duration
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a motion has at most u8::MAX segments and the per-instance segment index counts \
              the segments that instance has played, so both stay far below u32::MAX"
)]
const fn sample_loops_this_update(
    animation: &BevyTransitionAnimation,
    timing: MotionTiming,
) -> u32 {
    if !animation.flags().contains(AnimationFlags::LOOP_ANIMATION) {
        return 0;
    }
    let segment_count = timing.segment_count() as u32;
    let previous_segment = animation.previous_segment_index() as u32 % segment_count;
    (previous_segment + animation.segment_advances_this_update()) / segment_count
}

fn sample_root_pose(
    clip: &AnimationClip,
    target: AnimationTargetId,
    time: f32,
) -> Option<RootPose> {
    let translation = clip.sample_clamped(animated_field!(Transform::translation), target, time);
    let rotation = clip.sample_clamped(animated_field!(Transform::rotation), target, time);
    if translation.is_none() && rotation.is_none() {
        return None;
    }
    let rotation = rotation.unwrap_or(Quat::IDENTITY);
    Some(RootPose {
        rotation: normalize_quat_or_identity(Quat::from_xyzw(0.0, 0.0, rotation.z, rotation.w)),
        translation: translation.unwrap_or(Vec3::ZERO),
    })
}

fn yaw(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::Y;
    (-forward.x).atan2(forward.y)
}

fn travel_direction(translation: Vec3) -> f32 {
    let direction = (-translation.x).atan2(translation.y);
    if translation.y < 0.0 {
        if translation.x < 0.0 {
            std::f32::consts::PI - direction
        } else {
            -std::f32::consts::PI - direction
        }
    } else {
        direction
    }
}

fn travel_slope(translation: Vec3) -> f32 {
    let direction = (-translation.x).atan2(translation.y);
    let local = Quat::from_rotation_z(-direction) * translation;
    local.z.atan2(local.y)
}

impl AnimationPlayback<BevyMotion> for CryAnimationPlayer {
    fn animation_duration(&self, animation: &BevyMotion) -> Option<f32> {
        Some(animation.duration())
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        let animation = self.runtime.layer(lane.layer)?.top()?;
        Some(ActiveAnimationState {
            normalized_time: animation.animation().entire_normalized_time(
                &animation.state().parameters,
                animation.segment_index(),
                animation.normalized_time(),
            )?,
            expected_duration: animation.expected_duration(),
        })
    }

    fn start_animation(
        &mut self,
        animation: &BevyMotion,
        parameters: AnimationStartParameters,
    ) -> bool {
        self.runtime
            .start_animation_with_state(
                animation.clone(),
                CharacterAnimationParameters {
                    layer: parameters.lane.layer,
                    transition_time: parameters.transition_time,
                    key_time: parameters.key_time,
                    playback_speed: parameters.playback_speed,
                    playback_weight: parameters.playback_weight,
                    user_data: parameters.blend_channels.into(),
                    expected_duration: animation.duration(),
                    allow_multi_layer_animation: 1.0,
                    user_token: parameters.user_token,
                    flags: parameters.flags,
                },
                animation.initial_state(),
            )
            .is_ok()
    }

    fn stop_animation(&mut self, lane: AnimationLane, blend_time: f32) {
        self.runtime.stop_animation(lane.layer, blend_time);
    }

    fn clear_layer(&mut self, lane: AnimationLane) {
        self.runtime.clear_layer(lane.layer);
    }

    fn set_layer_playback_scale(&mut self, lane: AnimationLane, scale: f32) {
        self.runtime.set_layer_playback_scale(lane.layer, scale);
    }

    fn set_layer_blend_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_layer_blend_weight(lane.layer, weight);
    }

    fn set_top_animation_weight(&mut self, lane: AnimationLane, weight: f32) {
        self.runtime.set_top_animation_weight(lane.layer, weight);
    }

    fn set_top_animation_normalized_time(&mut self, lane: AnimationLane, normalized_time: f32) {
        let segment_time = self
            .runtime
            .layer(lane.layer)
            .and_then(|layer| layer.top())
            .and_then(|animation| {
                animation
                    .animation()
                    .segment_time_from_entire(&animation.state().parameters, normalized_time)
            });
        if let Some((segment, phase)) = segment_time {
            self.runtime
                .set_top_animation_segment_time(lane.layer, segment, phase);
        }
    }

    fn advance_layer_animations(
        &mut self,
        lane: AnimationLane,
        time_passed: f32,
        queued_increments: &[f32],
    ) {
        let already_applied = queued_increments.last().copied().unwrap_or_default();
        self.runtime
            .advance_layer(lane.layer, (time_passed - already_applied).max(0.0));
    }
}

impl MotionParameterSink for CryAnimationPlayer {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "layer indices are bounded by az_animation::character::ANIMATION_LAYER_COUNT"
    )]
    fn set_desired_motion_parameter(
        &mut self,
        parameter: MotionParameterId,
        value: f32,
        _delta_time: f32,
    ) {
        for layer_index in 0..az_animation::character::ANIMATION_LAYER_COUNT {
            let Some(layer) = self.runtime.layer_mut(layer_index as u32) else {
                continue;
            };
            let animation_count = layer.animations().len();
            for (index, animation) in layer.animations_mut().iter_mut().enumerate() {
                let Some(locked) = animation.animation().parameter_is_locked(parameter) else {
                    continue;
                };
                let initialized = animation.state().parameters.is_initialized(parameter);
                let blending_out = index + 1 < animation_count
                    && !animation
                        .flags()
                        .contains(AnimationFlags::UPDATE_MOTION_PARAMETERS_WHILE_BLENDING_OUT);
                let accept = !initialized || (!locked && !blending_out);
                animation
                    .state_mut()
                    .parameters
                    .record_desired(parameter, value, accept);
            }
        }
    }
}

/// Capability implemented by a project's concrete procedural-clip registry.
pub trait ProceduralClipExecutor<P> {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the argument set CActionScope hands a procedural clip on enter"
    )]
    fn enter(
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

    fn exit(&mut self, lane: ProceduralLane, blend_time: f32);

    fn fail(&mut self, lane: ProceduralLane);

    fn update(&mut self, lane: ProceduralLane, time_passed: f32);

    fn debug_draw(&mut self, lane: ProceduralLane);
}

/// Borrows the animation player and a typed procedural registry as one
/// Mannequin backend without a trait object or renderer-specific core API.
pub struct BevyMannequinBackend<'a, E> {
    animation: &'a mut CryAnimationPlayer,
    procedural: &'a mut E,
}

impl<'a, E> BevyMannequinBackend<'a, E> {
    #[must_use]
    pub const fn new(animation: &'a mut CryAnimationPlayer, procedural: &'a mut E) -> Self {
        Self {
            animation,
            procedural,
        }
    }
}

impl<E> AnimationPlayback<BevyMotion> for BevyMannequinBackend<'_, E> {
    fn animation_duration(&self, animation: &BevyMotion) -> Option<f32> {
        self.animation.animation_duration(animation)
    }

    fn top_animation(&self, lane: AnimationLane) -> Option<ActiveAnimationState> {
        self.animation.top_animation(lane)
    }

    fn start_animation(
        &mut self,
        animation: &BevyMotion,
        parameters: AnimationStartParameters,
    ) -> bool {
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

impl<P, E> ProceduralPlayback<P> for BevyMannequinBackend<'_, E>
where
    E: ProceduralClipExecutor<P>,
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
        self.procedural.enter(
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
        self.procedural.exit(lane, blend_time);
    }

    fn fail_procedural(&mut self, lane: ProceduralLane) {
        self.procedural.fail(lane);
    }

    fn update_procedural(&mut self, lane: ProceduralLane, time_passed: f32) {
        self.procedural.update(lane, time_passed);
    }

    fn debug_draw_procedural(&mut self, lane: ProceduralLane) {
        self.procedural.debug_draw(lane);
    }
}

#[derive(Bundle)]
pub struct CryAnimationPlayerBundle {
    pub cry_animation: CryAnimationPlayer,
    pub animation_driven_motion: CryAnimationDrivenMotion,
    pub root_motion: CryRootMotionState,
    pub animation_player: AnimationPlayer,
    pub animation_graph: AnimationGraphHandle,
}

impl CryAnimationPlayerBundle {
    #[must_use]
    pub fn new(graphs: &mut Assets<AnimationGraph>) -> Self {
        let graph = graphs.add(AnimationGraph::new());
        Self {
            cry_animation: CryAnimationPlayer::default(),
            animation_driven_motion: CryAnimationDrivenMotion::default(),
            root_motion: CryRootMotionState::default(),
            animation_player: AnimationPlayer::default(),
            animation_graph: AnimationGraphHandle(graph),
        }
    }
}

/// Frame-local request that enables and scales locator motion for one
/// character animation root.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct CryAnimationDrivenMotion(Option<AnimationDrivenMotionRequest>);

impl CryAnimationDrivenMotion {
    #[must_use]
    pub const fn request(&self) -> Option<AnimationDrivenMotionRequest> {
        self.0
    }

    pub fn set(&mut self, request: impl Into<AnimationDrivenMotionRequest>) {
        self.0 = Some(request.into());
    }

    pub const fn clear(&mut self) {
        self.0 = None;
    }
}

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CryRootMotionState(RootMotionState);

impl AsRef<RootMotionState> for CryRootMotionState {
    fn as_ref(&self) -> &RootMotionState {
        &self.0
    }
}

impl AsMut<RootMotionState> for CryRootMotionState {
    fn as_mut(&mut self) -> &mut RootMotionState {
        &mut self.0
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CryAnimationSet {
    Reset,
    BindTargets,
    Advance,
    RootMotion,
}

fn reset_cry_animation_driven_motion(mut requests: Query<&mut CryAnimationDrivenMotion>) {
    for mut request in &mut requests {
        request.clear();
    }
}

fn bind_cry_animation_targets(
    mut commands: Commands,
    added_targets: Query<(Entity, &AnimationTargetId), Added<AnimationTargetId>>,
    targets: Query<(Entity, &AnimationTargetId)>,
    added_players: Query<Entity, Added<CryAnimationPlayer>>,
    parents: Query<&ChildOf>,
    players: Query<(), With<CryAnimationPlayer>>,
) {
    for (target, _) in &added_targets {
        if let Some(player) = animation_player_ancestor(target, &parents, &players) {
            commands.entity(target).insert(AnimatedBy(player));
        }
    }

    for player in &added_players {
        for (target, _) in &targets {
            if animation_player_ancestor(target, &parents, &players) == Some(player) {
                commands.entity(target).insert(AnimatedBy(player));
            }
        }
    }
}

fn animation_player_ancestor(
    mut entity: Entity,
    parents: &Query<&ChildOf>,
    players: &Query<(), With<CryAnimationPlayer>>,
) -> Option<Entity> {
    loop {
        if players.contains(entity) {
            return Some(entity);
        }
        entity = parents.get(entity).ok()?.parent();
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]
fn advance_cry_animation_players(time: Res<Time>, mut players: Query<&mut CryAnimationPlayer>) {
    for mut player in &mut players {
        player.advance(time.delta_secs());
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]
fn apply_cry_animation_root_motion(
    time: Res<Time>,
    clips: Res<Assets<AnimationClip>>,
    mut physics_world: Option<ResMut<PhysicsWorld>>,
    mut characters: Query<(
        &CryAnimationPlayer,
        &CryAnimationDrivenMotion,
        &mut CryRootMotionState,
        &PhysicsBodyHandle,
    )>,
) {
    let Some(world) = physics_world.as_deref_mut() else {
        return;
    };

    for (player, requested, mut state, &body) in &mut characters {
        let enabled = requested.request().is_some();
        let Ok(status) = world.body_status(body) else {
            continue;
        };

        let mut relative_motion = if enabled {
            player.calculate_relative_movement(&clips, time.delta_secs())
        } else {
            RootMotionDelta::default()
        };
        if let Some(request) = requested.request() {
            relative_motion.translation *= request.translation_multiplier;
            relative_motion.rotation =
                Quat::from_rotation_z(yaw(relative_motion.rotation) * request.rotation_multiplier);
        }

        match AsMut::<RootMotionState>::as_mut(&mut *state).update(
            enabled,
            status.pose.rotation,
            relative_motion,
            time.delta_secs(),
        ) {
            RootMotionCommand::Apply {
                linear_velocity,
                rotation_delta,
            } => {
                let _ = world.request_velocity(body, linear_velocity);
                let _ = world.apply_action(
                    body,
                    PhysicsAction::SetPose(PhysicsPose {
                        translation: status.pose.translation,
                        rotation: normalize_quat_or_identity(status.pose.rotation * rotation_delta),
                    }),
                );
            }
            RootMotionCommand::Stop => {
                let _ = world.request_velocity(body, Vec3::ZERO);
            }
            RootMotionCommand::None => {}
        }
    }
}

fn apply_cry_animation_players(
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut players: Query<(
        &mut CryAnimationPlayer,
        &mut AnimationPlayer,
        &AnimationGraphHandle,
    )>,
) {
    for (mut cry_animation, mut player, graph_handle) in &mut players {
        let Some(mut graph) = graphs.get_mut(graph_handle) else {
            continue;
        };
        cry_animation.apply(&mut graph, &mut player);
    }
}

/// Schedule used by authoritative Cry animation advancement and root motion.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CryAnimationSchedule {
    #[default]
    Update,
    FixedUpdate,
}

/// Installs the deterministic Cry clock immediately before Bevy evaluates its
/// animation graphs.
pub struct CryAnimationPlugin;

impl Plugin for CryAnimationPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        crate::register_character_runtime(app);
        crate::motion_runtime::register_motion_runtime(app);
        app.init_asset::<AnimationClip>()
            .init_asset::<AnimationGraph>()
            .init_asset::<az_animation::events::AnimationEventDatabaseAsset>()
            .init_asset_loader::<crate::AnimationEventDatabaseAssetLoader>()
            .init_asset::<az_animation::blend_space_asset::BlendSpaceAsset>()
            .init_asset::<az_animation::blend_space_asset::CombinedBlendSpaceAsset>()
            .init_asset_loader::<crate::BlendSpaceAssetLoader>()
            .init_asset_loader::<crate::CombinedBlendSpaceAssetLoader>()
            .init_asset::<az_animation::character::definition::CharacterDefinitionAsset>()
            .init_asset_loader::<crate::CharacterDefinitionAssetLoader>()
            .register_type::<DirectDeltaMotion>()
            .register_type::<az_animation::animation_set::AnimationProductRef>()
            .register_type::<az_animation::animation_set::AnimationRef>()
            .register_type::<az_animation::animation_set::BlendSpaceRef>()
            .register_type::<az_animation::animation_set::CombinedBlendSpaceRef>()
            .register_type::<az_animation::blend_space_asset::BlendSpaceAsset>()
            .register_type::<az_animation::blend_space_asset::BlendSpaceMotion>()
            .register_type::<az_animation::blend_space_asset::CombinedBlendSpaceAsset>();
        let schedule = app
            .world()
            .get_resource::<CryAnimationSchedule>()
            .copied()
            .unwrap_or_default();
        macro_rules! install_cry_animation_runtime {
            ($schedule:expr) => {
                app.configure_sets(
                    $schedule,
                    (
                        CryAnimationSet::Reset,
                        CryAnimationSet::BindTargets,
                        CryAnimationSet::Advance,
                    )
                        .chain(),
                )
                .add_systems(
                    $schedule,
                    (
                        reset_cry_animation_driven_motion.in_set(CryAnimationSet::Reset),
                        bind_cry_animation_targets.in_set(CryAnimationSet::BindTargets),
                        advance_cry_animation_players.in_set(CryAnimationSet::Advance),
                    ),
                )
                .add_systems(
                    $schedule,
                    apply_cry_animation_root_motion
                        .after(CryAnimationSet::Advance)
                        .in_set(CryAnimationSet::RootMotion)
                        .in_set(PhysicsSet::Forces),
                );
            };
        }
        match schedule {
            CryAnimationSchedule::Update => {
                install_cry_animation_runtime!(Update);
            }
            CryAnimationSchedule::FixedUpdate => {
                install_cry_animation_runtime!(FixedUpdate);
            }
        }
        app.add_systems(
            PostUpdate,
            apply_cry_animation_players.before(bevy::app::AnimationSystems),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_animation::animation_set::AnimationMotionRef;
    use az_animation::blend_space::{
        BlendSpaceDimension, ParametricBlendSpaceDescription, VirtualExample,
    };
    use bevy::animation::prelude::{AnimatableCurve, AnimatableKeyframeCurve};

    /// `CryEngine`'s default CAF key rate; only reachable through
    /// [`MotionTiming::clock_segment_duration`], which floors a segment at one
    /// sample period. Every segment below is far longer than 1/30 s, so the
    /// floor never binds and the fixtures stay readable.
    const SAMPLE_RATE: f32 = 30.0;

    fn motion(clips: &mut Assets<AnimationClip>, name: &str, translation: Vec3) -> BevyMotion {
        let root_target = AnimationTargetId::from_name(&Name::new(name.to_owned()));
        let mut clip = AnimationClip::default();
        clip.add_curve_to_target(
            root_target,
            AnimatableCurve::new(
                animated_field!(Transform::translation),
                AnimatableKeyframeCurve::new([(0.0, Vec3::ZERO), (1.0, translation)]).unwrap(),
            ),
        );
        BevyMotion::direct(BevyClipMotion {
            clip: clips.add(clip),
            duration: 1.0,
            mask: 0,
            root_target,
            timing: MotionTiming::single(1.0, 1.0, SAMPLE_RATE, translation.length()).unwrap(),
            direct_delta_motion: DirectDeltaMotion::default(),
        })
    }

    /// A two-example 1D blend space whose examples carry distinct playback
    /// scales and delta-motion flags, so the positional wiring is observable.
    fn blend_space_asset() -> BlendSpaceAsset {
        let sampler = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: vec![BlendSpaceDimension {
                parameter: MotionParameterId::TravelSpeed,
                min: 0.0,
                max: 1.0,
                cells: 2,
                locked: false,
            }],
            additional_extraction: Vec::new(),
            example_count: 2,
            example_parameters: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            pseudo_examples: Vec::new(),
            faces: Vec::new(),
            virtual_examples: vec![
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [1.0, 0.0].into_iter().collect(),
                },
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [0.0, 1.0].into_iter().collect(),
                },
            ],
            threshold: None,
            idle_to_move: false,
        })
        .unwrap();
        BlendSpaceAsset {
            motions: vec![
                BlendSpaceMotion {
                    animation: AnimationRef::new(
                        "idle",
                        AnimationMotionRef::new(
                            az_core::AssetId::nil(),
                            Some("animations/player/idle.anim.glb"),
                        ),
                    ),
                    playback_scale: 2.0,
                    direct_delta_motion: DirectDeltaMotion::from_dimensions([(true, 3.0)]),
                },
                BlendSpaceMotion {
                    animation: AnimationRef::new(
                        "run",
                        AnimationMotionRef::new(
                            az_core::AssetId::nil(),
                            Some("animations/player/run.anim.glb"),
                        ),
                    ),
                    playback_scale: 0.5,
                    direct_delta_motion: DirectDeltaMotion::default(),
                },
            ],
            timewarp_group: Some("Locomotion".to_owned()),
            sampler,
        }
    }

    #[test]
    fn motion_product_path_renames_the_authoring_extension() {
        let reference = AnimationRef::new(
            "idle",
            AnimationMotionRef::new(
                az_core::AssetId::nil(),
                Some("animations/player/idle.anim.glb"),
            ),
        );

        assert_eq!(
            motion_product_path(&reference).as_deref(),
            Some("animations/player/idle.motion.glb")
        );
        assert_eq!(motion_product_path(&AnimationRef::alias("idle")), None);
    }

    /// The sampler addresses clips positionally, so example `i` of the compiled
    /// blend space must become clip `i` of the motion, carrying that example's
    /// own playback scale and delta-motion flags.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the per-example playback scales are copied verbatim from the compiled blend \
                  space, so the assertion is that they arrive bit-for-bit unchanged"
    )]
    fn blend_space_examples_keep_their_order_and_per_example_metadata() {
        let mut clips = Assets::<AnimationClip>::default();
        let mut requested = Vec::new();
        let asset = blend_space_asset();
        let source = BevyParametricSource::from_blend_space_with(
            az_core::AssetId::nil(),
            &asset,
            0,
            |path| {
                requested.push(path);
                Handle::default()
            },
        )
        .expect("every example resolves to a product path");

        assert_eq!(
            requested,
            vec![
                "animations/player/idle.motion.glb".to_owned(),
                "animations/player/run.motion.glb".to_owned(),
            ]
        );

        let first = motion(&mut clips, "first-root", Vec3::X).clips[0].clone();
        let second = motion(&mut clips, "second-root", Vec3::Y).clips[0].clone();
        let first_clip = first.clip().clone();
        let second_clip = second.clip().clone();
        let resolved = source
            .assemble(vec![first, second])
            .expect("both examples assemble");

        assert_eq!(resolved.clips().len(), 2);
        assert_eq!(resolved.clips()[0].clip(), &first_clip);
        assert_eq!(resolved.clips()[1].clip(), &second_clip);
        assert_eq!(resolved.clips()[0].timing().playback_scale(), 2.0);
        assert_eq!(resolved.clips()[1].timing().playback_scale(), 0.5);
        assert_eq!(
            resolved.clips()[0].direct_delta_motion(),
            DirectDeltaMotion::from_dimensions([(true, 3.0)])
        );
        assert_eq!(
            resolved.clips()[1].direct_delta_motion(),
            DirectDeltaMotion::default()
        );
        assert_eq!(resolved.timewarp_group(), Some("Locomotion"));
        assert_eq!(resolved.asset_id(), Some(az_core::AssetId::nil()));
    }

    /// A blend space is all-or-nothing: one example without a resolvable motion
    /// reference would shift every later example onto the wrong clip, so the
    /// whole source is refused.
    #[test]
    fn blend_space_with_an_unresolvable_example_yields_no_source() {
        let mut asset = blend_space_asset();
        asset.motions[0].animation = AnimationRef::alias("idle");

        assert!(
            BevyParametricSource::from_blend_space_with(az_core::AssetId::nil(), &asset, 0, |_| {
                Handle::default()
            })
            .is_none()
        );
    }

    /// The compiled sampler decides the weights; the bridge only has to keep
    /// the clips in the order the sampler indexes them.
    #[test]
    fn assembled_blend_space_evaluates_through_the_compiled_sampler() {
        let mut clips = Assets::<AnimationClip>::default();
        let asset = blend_space_asset();
        let source =
            BevyParametricSource::from_blend_space_with(az_core::AssetId::nil(), &asset, 0, |_| {
                Handle::default()
            })
            .unwrap();
        let first = motion(&mut clips, "first-root", Vec3::X).clips[0].clone();
        let second = motion(&mut clips, "second-root", Vec3::Y).clips[0].clone();
        let resolved = source.assemble(vec![first, second]).unwrap();

        let mut parameters = MotionParameters::default();
        parameters.set(MotionParameterId::TravelSpeed, 1.0);
        let mut weights = BlendWeights::default();
        resolved.evaluate(&parameters, &mut weights);

        // CryEngine clamps the grid coordinate to `cells - 1 - 0.001` before
        // interpolating (`ParametricSampler.cpp:509`), so the top of the range
        // keeps a thousandth of the first example.
        assert!((weights.as_slice()[0] - 0.001).abs() < 0.0001);
        assert!((weights.as_slice()[1] - 0.999).abs() < 0.0001);
    }

    fn parameters(flags: AnimationFlags) -> CharacterAnimationParameters {
        CharacterAnimationParameters {
            expected_duration: 1.0,
            flags,
            ..Default::default()
        }
    }

    fn parametric_motion(
        clips: &mut Assets<AnimationClip>,
        locked: bool,
        segmented: bool,
    ) -> BevyMotion {
        let sampler = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: vec![BlendSpaceDimension {
                parameter: MotionParameterId::TravelSpeed,
                min: 0.0,
                max: 1.0,
                cells: 2,
                locked,
            }],
            additional_extraction: Vec::new(),
            example_count: 2,
            // The authored grid is already the right size, so it is used
            // as CryEngine's cache and never rebuilt from these.
            example_parameters: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            pseudo_examples: Vec::new(),
            faces: Vec::new(),
            virtual_examples: vec![
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [1.0, 0.0].into_iter().collect(),
                },
                VirtualExample {
                    indices: [0, 1].into_iter().collect(),
                    weights: [0.0, 1.0].into_iter().collect(),
                },
            ],
            threshold: None,
            idle_to_move: false,
        })
        .unwrap();
        let mut first = motion(clips, "parametric-root", Vec3::Y).clips[0].clone();
        let mut second = motion(clips, "parametric-root", Vec3::Y).clips[0].clone();
        if segmented {
            // Two half-second segments, each covering one unit of root travel:
            // the mean travel speed CryEngine measures off the root keys is
            // therefore 2 units/s, not 1 (`ParametricSampler.cpp:240-252`).
            let timing = MotionTiming::from_segments(
                1.0,
                1.0,
                SAMPLE_RATE,
                &[0.0, 0.5, 1.0],
                &[1.0, 1.0],
                &[2.0, 2.0],
            )
            .unwrap();
            first.timing = timing;
            second.timing = timing;
        }
        BevyMotion::blend_space(sampler, [first, second]).unwrap()
    }

    #[test]
    fn extracts_direct_root_translation_from_the_authoritative_clock() {
        let mut clips = Assets::default();
        let motion = motion(&mut clips, "root", Vec3::new(0.0, 2.0, 0.0));
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation(motion, parameters(AnimationFlags::empty()))
            .unwrap();

        player.advance(0.25);
        let delta = player.calculate_relative_movement(&clips, 0.25);

        assert!(
            delta
                .translation
                .abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 0.0001)
        );
        assert!(delta.rotation.abs_diff_eq(Quat::IDENTITY, 0.0001));
    }

    #[test]
    fn extracts_every_loop_crossed_in_one_update() {
        let mut clips = Assets::default();
        let motion = motion(&mut clips, "root", Vec3::Y);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation(motion, parameters(AnimationFlags::LOOP_ANIMATION))
            .unwrap();

        player.advance(2.25);
        let delta = player.calculate_relative_movement(&clips, 2.25);

        assert!(
            delta
                .translation
                .abs_diff_eq(Vec3::new(0.0, 2.25, 0.0), 0.0001)
        );
    }

    #[test]
    fn segmented_motion_uses_shared_segment_phase_and_entire_clip_sampling() {
        let mut clips = Assets::default();
        let mut motion = motion(&mut clips, "root", Vec3::new(0.0, 2.0, 0.0));
        let clip = Arc::make_mut(&mut motion.clips)[0].clone();
        Arc::make_mut(&mut motion.clips)[0] = BevyClipMotion {
            timing: MotionTiming::from_segments(
                1.0,
                1.0,
                SAMPLE_RATE,
                &[0.0, 0.5, 1.0],
                &[1.0, 1.0],
                &[2.0, 2.0],
            )
            .unwrap(),
            ..clip
        };
        motion.timings = motion.clips.iter().map(|clip| clip.timing).collect();
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation(motion, parameters(AnimationFlags::empty()))
            .unwrap();

        player.advance(0.75);
        let animation = player.runtime().layer(0).unwrap().top().unwrap();
        assert_eq!(animation.segment_index(), 1);
        assert!((animation.normalized_time() - 0.5).abs() < 0.0001);
        let delta = player.calculate_relative_movement(&clips, 0.75);
        assert!(
            delta
                .translation
                .abs_diff_eq(Vec3::new(0.0, 1.5, 0.0), 0.0001)
        );
    }

    #[test]
    fn full_root_priority_discards_earlier_fifo_contributors() {
        let mut clips = Assets::default();
        let first = motion(&mut clips, "root", Vec3::X);
        let second = motion(&mut clips, "root", Vec3::new(0.0, 2.0, 0.0));
        let mut player = CryAnimationPlayer::default();
        let mut first_parameters = parameters(AnimationFlags::LOOP_ANIMATION);
        first_parameters.transition_time = 1.0;
        player
            .runtime_mut()
            .start_animation(first, first_parameters)
            .unwrap();
        player.advance(0.1);

        let mut second_parameters = parameters(AnimationFlags::FULL_ROOT_PRIORITY);
        second_parameters.transition_time = 1.0;
        player
            .runtime_mut()
            .start_animation(second, second_parameters)
            .unwrap();
        player.advance(0.1);
        let delta = player.calculate_relative_movement(&clips, 0.1);

        assert!(
            delta
                .translation
                .abs_diff_eq(Vec3::new(0.0, 0.2, 0.0), 0.0001)
        );
    }

    #[test]
    fn desired_motion_parameters_freeze_blending_out_instances() {
        let mut clips = Assets::default();
        let first = parametric_motion(&mut clips, false, false);
        let second = parametric_motion(&mut clips, false, true);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                first.clone(),
                parameters(AnimationFlags::LOOP_ANIMATION),
                first.initial_state(),
            )
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.25, 0.0);

        let mut second_parameters = parameters(AnimationFlags::LOOP_ANIMATION);
        second_parameters.transition_time = 1.0;
        player
            .runtime_mut()
            .start_animation_with_state(second.clone(), second_parameters, second.initial_state())
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.75, 0.0);

        let queue = player.runtime().layer(0).unwrap();
        assert_eq!(
            queue.animations()[0]
                .state()
                .parameters()
                .get(MotionParameterId::TravelSpeed),
            Some(0.25)
        );
        assert_eq!(
            queue.animations()[1]
                .state()
                .parameters()
                .get(MotionParameterId::TravelSpeed),
            Some(0.75)
        );
        assert_eq!(
            player.desired_motion_parameter(MotionParameterId::TravelSpeed),
            Some(0.75)
        );
    }

    #[test]
    fn blending_out_instance_can_opt_into_motion_parameter_updates() {
        let mut clips = Assets::default();
        let first = parametric_motion(&mut clips, false, false);
        let second = parametric_motion(&mut clips, false, true);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                first.clone(),
                parameters(
                    AnimationFlags::LOOP_ANIMATION
                        | AnimationFlags::UPDATE_MOTION_PARAMETERS_WHILE_BLENDING_OUT,
                ),
                first.initial_state(),
            )
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.25, 0.0);

        let mut second_parameters = parameters(AnimationFlags::LOOP_ANIMATION);
        second_parameters.transition_time = 1.0;
        player
            .runtime_mut()
            .start_animation_with_state(second.clone(), second_parameters, second.initial_state())
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.75, 0.0);

        let queue = player.runtime().layer(0).unwrap();
        assert_eq!(
            queue.animations()[0]
                .state()
                .parameters()
                .get(MotionParameterId::TravelSpeed),
            Some(0.75)
        );
        assert_eq!(
            queue.animations()[1]
                .state()
                .parameters()
                .get(MotionParameterId::TravelSpeed),
            Some(0.75)
        );
    }

    #[test]
    fn locked_motion_parameter_is_initialized_once() {
        let mut clips = Assets::default();
        let motion = parametric_motion(&mut clips, true, false);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                motion.clone(),
                parameters(AnimationFlags::LOOP_ANIMATION),
                motion.initial_state(),
            )
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.2, 0.0);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.9, 0.0);

        assert_eq!(
            player.desired_motion_parameter(MotionParameterId::TravelSpeed),
            Some(0.2)
        );
    }

    #[test]
    fn same_motion_time_warp_copies_parametric_sampler_state() {
        let mut clips = Assets::default();
        let motion = parametric_motion(&mut clips, false, false);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                motion.clone(),
                parameters(AnimationFlags::LOOP_ANIMATION),
                motion.initial_state(),
            )
            .unwrap();
        player.advance(0.1);
        player.set_desired_motion_parameter(MotionParameterId::TravelSpeed, 0.6, 0.0);

        let mut next = parameters(
            AnimationFlags::LOOP_ANIMATION
                | AnimationFlags::ALLOW_ANIMATION_RESTART
                | AnimationFlags::TRANSITION_TIME_WARPING,
        );
        next.transition_time = 1.0;
        player
            .runtime_mut()
            .start_animation_with_state(motion.clone(), next, motion.initial_state())
            .unwrap();
        player.advance(0.1);

        let queue = player.runtime().layer(0).unwrap();
        assert_eq!(
            queue.animations()[1]
                .state()
                .parameters()
                .get(MotionParameterId::TravelSpeed),
            Some(0.6)
        );
    }

    /// `CSkeletonAnim::AnimCallback` remaps a direct CAF's window through
    /// `GetNTimeforEntireClip` on both ends (`SkeletonAnim_BlendMan.cpp:724-725`),
    /// so a 0.75 s step over two half-second segments reports 0.75 of the entire
    /// clip: the clock crosses into segment 1 at phase 0.5
    /// (0.75 s / 0.5 s segment = 1.5), and segment 1 starting at 0.5 puts phase
    /// 0.5 at 0.5 + 0.5 * 0.5. `looped` stays clear because a non-looping CAF
    /// only sets `CA_LOOPED_THIS_UPDATE` when its segment counter wraps
    /// (`SkeletonAnim_BlendMan.cpp:512-516`).
    #[test]
    fn direct_event_window_uses_entire_clip_time_across_segments() {
        let mut clips = Assets::default();
        let mut motion = motion(&mut clips, "event-root", Vec3::Y);
        Arc::make_mut(&mut motion.clips)[0].timing = MotionTiming::from_segments(
            1.0,
            1.0,
            SAMPLE_RATE,
            &[0.0, 0.5, 1.0],
            &[1.0, 1.0],
            &[2.0, 2.0],
        )
        .unwrap();
        motion.timings = motion.clips.iter().map(|clip| clip.timing).collect();
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                motion.clone(),
                parameters(AnimationFlags::empty()),
                motion.initial_state(),
            )
            .unwrap();

        player.advance(0.75);
        let animation = player.runtime().layer(0).unwrap().top().unwrap();
        assert_eq!(
            motion.event_window(animation),
            Some(MotionEventWindow {
                previous: 0.0,
                current: 0.75,
                cycles: 0,
                include_start: true,
            })
        );
    }

    /// A parametric group is the other half of `AnimCallback`: it leaves the
    /// window in raw segment phase, because the `GetNTimeforEntireClip` remap is
    /// only reached on the CAF branch (`SkeletonAnim_BlendMan.cpp:713-739`).
    /// Its `looped` flag is also coarser — `CryEngine` raises
    /// `CA_LOOPED_THIS_UPDATE` on *every* segment advance of a parametric group,
    /// not only on the wrap (`SkeletonAnim_BlendMan.cpp:498-504`), so one
    /// segment step reports one cycle.
    #[test]
    fn parametric_event_window_uses_segment_phase_and_advance_count() {
        let mut clips = Assets::default();
        let motion = parametric_motion(&mut clips, false, true);
        let mut player = CryAnimationPlayer::default();
        player
            .runtime_mut()
            .start_animation_with_state(
                motion.clone(),
                parameters(AnimationFlags::LOOP_ANIMATION),
                motion.initial_state(),
            )
            .unwrap();

        player.advance(0.75);
        let animation = player.runtime().layer(0).unwrap().top().unwrap();
        assert_eq!(
            motion.event_window(animation),
            Some(MotionEventWindow {
                previous: 0.0,
                current: 0.5,
                cycles: 1,
                include_start: true,
            })
        );
    }
}
