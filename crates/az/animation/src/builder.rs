use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::normalize_source_path;
use bevy_math::{Quat, Vec3};
use gltf::animation::Property;
use gltf::animation::util::ReadOutputs;
use serde::Deserialize;
use uuid::uuid;

use crate::blend_space::MotionTiming;
use crate::controller_target::{
    AnimationControllerBindingExtras, AnimationControllerNodeExtras, CONTROLLER_TARGET_ROOT_NAME,
    controller_target_node_name,
};
use crate::{AnimationRuntimeGltfProductFormat, AnimationSourceFormat};

pub const NAME: &str = "azoth.animation";
pub const ID: BuilderId = BuilderId::new(uuid!("e70d86a6-e0ec-4be0-9b10-ce5e830cf4bb"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<AnimationSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<AnimationRuntimeGltfProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(req: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let jobs = req
        .platforms
        .iter()
        .copied()
        .map(JobDescriptor::default_for_platform)
        .collect();
    CreateJobsResponse {
        jobs,
        ..CreateJobsResponse::default()
    }
}

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product = match transform_product(&req.source_path, req.source_bytes) {
        Ok(product) => product,
        Err(err) => {
            tracing::warn!(source = %req.source_path, error = %err, "animation product failed");
            return ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            };
        }
    };

    ProcessJobResponse {
        products: vec![product],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

/// Rewrites an authored `.anim.glb` source into its runtime `.motion.glb`
/// product once the animation data it carries has been validated.
///
/// # Errors
///
/// Returns `AnimationProductError::NotBinaryGlb` or `AnimationProductError::Gltf`
/// when `source_bytes` is not a parseable binary glTF container, and one of the
/// authoring-validation variants - `MissingBinaryBuffer`, `NoAnimationChannels`,
/// `UnnamedTargetNode`, `MissingInputTimes`, `NonIncreasingTimes`,
/// `MissingOutputs`, `NonUnitRotation`, `UnsupportedProperty`,
/// `UnsupportedControllerTargetSpace`, `MissingControllerTargetId`,
/// `InvalidControllerTargetName`, `MissingRootControllerTarget` or
/// `InvalidEmptyControllerAnimation` - when a validation check fails.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, AnimationProductError> {
    let gltf = validate_glb(source_path, source_bytes)?;
    validate_animation_authoring_source(source_path, &gltf)?;

    Ok(
        TypedBuildProduct::<AnimationRuntimeGltfProductFormat>::from_trusted_path(
            animation_product_path(source_path),
            0,
            source_bytes.to_vec(),
        )
        .erase(),
    )
}

#[must_use]
pub fn animation_product_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".anim.glb").unwrap_or(&normalized);
    format!("{stem}.motion.glb")
}

#[derive(Debug, thiserror::Error)]
pub enum AnimationProductError {
    #[error("animation source {source_path} must be a binary .glb")]
    NotBinaryGlb { source_path: String },
    #[error("parse glTF animation source {source_path}: {source}")]
    Gltf {
        source_path: String,
        source: gltf::Error,
    },
    #[error("animation source {source_path} must embed a binary buffer")]
    MissingBinaryBuffer { source_path: String },
    #[error("animation source {source_path} contains no animation channels")]
    NoAnimationChannels { source_path: String },
    #[error("animation source {source_path} channel targets unnamed node")]
    UnnamedTargetNode { source_path: String },
    #[error("animation source {source_path} channel has no input key times")]
    MissingInputTimes { source_path: String },
    #[error("animation source {source_path} channel key times are not strictly increasing")]
    NonIncreasingTimes { source_path: String },
    #[error("animation source {source_path} channel output sampler is missing")]
    MissingOutputs { source_path: String },
    #[error(
        "animation source {source_path} channel uses unsupported animation property {property:?}"
    )]
    UnsupportedProperty {
        source_path: String,
        property: Property,
    },
    #[error("animation source {source_path} rotation output is not unit length: {norm}")]
    NonUnitRotation { source_path: String, norm: f32 },
    #[error(
        "animation source {source_path} declares unsupported controller target space {target_space}"
    )]
    UnsupportedControllerTargetSpace {
        source_path: String,
        target_space: String,
    },
    #[error("animation source {source_path} controller target node {node} has no controller ID")]
    MissingControllerTargetId { source_path: String, node: String },
    #[error(
        "animation source {source_path} controller {controller_id:#010x} target is named {actual}, expected {expected}"
    )]
    InvalidControllerTargetName {
        source_path: String,
        controller_id: u32,
        actual: String,
        expected: String,
    },
    #[error(
        "animation source {source_path} root controller {controller_id:#010x} has no target node"
    )]
    MissingRootControllerTarget {
        source_path: String,
        controller_id: u32,
    },
    #[error(
        "animation source {source_path} controller-space root may only carry an identity translation timing channel"
    )]
    InvalidEmptyControllerAnimation { source_path: String },
}

/// Parses `bytes` as the binary glTF container behind `source_path`.
///
/// # Errors
///
/// Returns `AnimationProductError::NotBinaryGlb` when `bytes` does not start
/// with the `glTF` magic, and `AnimationProductError::Gltf` when the container
/// itself fails to parse.
pub fn validate_glb(source_path: &str, bytes: &[u8]) -> Result<gltf::Gltf, AnimationProductError> {
    if !bytes.starts_with(b"glTF") {
        return Err(AnimationProductError::NotBinaryGlb {
            source_path: source_path.to_string(),
        });
    }

    gltf::Gltf::from_slice(bytes).map_err(|source| AnimationProductError::Gltf {
        source_path: source_path.to_string(),
        source,
    })
}

/// Derives the time-warp inputs for one animation root from its authored keys.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "these mirror CryEngine's own `uint32`/`float` conversions on the key grid; the \
              sample count is clamped to 2..=1023, the segment boundaries are validated \
              normalized times in 0..=1, and the pose count never exceeds the sample count"
)]
#[expect(
    clippy::suboptimal_flops,
    reason = "bit-exact port of `Init_MoveSpeed`'s per-key accumulation; fusing the multiply \
              and the add changes the rounding and therefore the compiled blend-space grid"
)]
#[must_use]
pub fn root_motion_timing(
    gltf: &gltf::Gltf,
    animation_index: usize,
    root_node_index: usize,
    duration: f32,
) -> Option<MotionTiming> {
    let blob = gltf.blob.as_deref()?;
    let animation = gltf.animations().nth(animation_index)?;
    let buffer_data = |buffer: gltf::Buffer<'_>| (buffer.index() == 0).then_some(blob);
    let extras = animation_extras(&animation)?;
    if !extras.cry_sample_rate.is_finite() || extras.cry_sample_rate <= 0.0 {
        return None;
    }
    let boundaries = motion_segment_boundaries(&extras.cry_events)?;
    let channels = animation
        .channels()
        .filter(|channel| {
            channel.target().node().index() == root_node_index
                && channel.target().property() == Property::Translation
        })
        .collect::<Vec<_>>();
    if channels.len() > 1 {
        return None;
    }

    let mut travel_distances = vec![0.0; boundaries.len() - 1];
    let mut mean_travel_speeds = vec![0.0; boundaries.len() - 1];
    if let Some(channel) = channels.first() {
        let reader = channel.reader(buffer_data);
        let times = reader.read_inputs()?.collect::<Vec<_>>();
        let ReadOutputs::Translations(outputs) = reader.read_outputs()? else {
            return None;
        };
        let values = outputs.collect::<Vec<_>>();
        let expected_values = times.len()
            * if channel.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline {
                3
            } else {
                1
            };
        if times.is_empty() || values.len() != expected_values {
            return None;
        }
        let curve = TranslationCurve {
            times,
            values,
            interpolation: channel.sampler().interpolation(),
        };
        let sample_count = ((duration * extras.cry_sample_rate) as usize + 1).clamp(2, 1023);
        let positions = (0..sample_count)
            .map(|index| curve.sample(duration * index as f32 / (sample_count - 1) as f32))
            .collect::<Option<Vec<_>>>()?;
        for segment_index in 0..travel_distances.len() {
            let start_key = (boundaries[segment_index] * (sample_count - 1) as f32) as usize;
            let end_key = (boundaries[segment_index + 1] * (sample_count - 1) as f32) as usize;
            let mut pose_count = 0usize;
            for pair in positions[start_key..=end_key].windows(2) {
                let distance = vec3_distance(pair[0], pair[1]);
                travel_distances[segment_index] += distance;
                mean_travel_speeds[segment_index] += distance * extras.cry_sample_rate;
                pose_count += 1;
            }
            if pose_count != 0 {
                mean_travel_speeds[segment_index] /= pose_count as f32;
            }
        }
    }

    MotionTiming::from_segments(
        duration,
        1.0,
        extras.cry_sample_rate,
        &boundaries,
        &travel_distances,
        &mean_travel_speeds,
    )
}

/// Root-joint pose keys for one animation, sampled on `CryEngine`'s key grid
/// and expressed in `CryEngine`'s axes.
///
/// `CryEngine`'s blend-space parameter extraction reads the root joint of the
/// referenced clip at `uint32(duration * sampleRate + 1)` evenly spaced keys.
/// `positions` and `rotations` are empty when the root joint carries no channel
/// of that kind and are otherwise both [`Self::key_count`] long.
///
/// Keys are converted out of glTF axes by [`cry_translation`] and
/// [`cry_rotation`] so that consumers can apply `CryEngine`'s Z-up, +Y-forward
/// motion math unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct RootMotionSamples {
    positions: Vec<Vec3>,
    rotations: Vec<Quat>,
    sample_rate: f32,
}

impl RootMotionSamples {
    #[must_use]
    pub const fn new(positions: Vec<Vec3>, rotations: Vec<Quat>, sample_rate: f32) -> Self {
        Self {
            positions,
            rotations,
            sample_rate,
        }
    }

    /// Root-joint translations, or `None` when the clip has no root translation
    /// channel and position-derived parameters cannot be extracted.
    #[must_use]
    pub fn positions(&self) -> Option<&[Vec3]> {
        (!self.positions.is_empty()).then_some(self.positions.as_slice())
    }

    /// Root-joint rotations, or `None` when the clip has no root rotation
    /// channel and rotation-derived parameters cannot be extracted.
    #[must_use]
    pub fn rotations(&self) -> Option<&[Quat]> {
        (!self.rotations.is_empty()).then_some(self.rotations.as_slice())
    }

    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// `CryEngine`'s `numKeys` for this clip.
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.positions.len().max(self.rotations.len())
    }
}

/// Upper bound on sampled root keys. `CryEngine` has no such cap; this only
/// keeps a corrupt duration or sample rate from requesting an unbounded
/// allocation.
const MAX_ROOT_MOTION_KEYS: usize = 1 << 16;

/// Rewrites an authored root translation into `CryEngine`'s axes.
///
/// The `.anim.glb` exporter writes root motion in glTF axes under the mapping
/// `Cry (x, y, z) -> glTF (-x, z, y)`; this is its inverse. `CryEngine`'s
/// motion extractors assume its own right-handed Z-up, +Y-forward frame, so the
/// conversion has to happen before any of that math runs.
///
/// The mapping converts Cry's right-handed Z-up coordinates to glTF's
/// right-handed Y-up coordinates and can be written
/// `CryToGltfVec3`. It is an involution, so the same permutation inverts it.
#[must_use]
pub const fn cry_translation(value: [f32; 3]) -> Vec3 {
    Vec3::new(-value[0], value[2], value[1])
}

/// The same change of basis for a rotation.
///
/// The exporter conjugates the matrix (`SWAP * m * SWAP`); since `SWAP` is a
/// proper rotation, conjugation maps the quaternion's axis by it and leaves the
/// angle alone, so permuting the vector part is equivalent and avoids a matrix
/// round-trip.
#[must_use]
pub fn cry_rotation(value: Quat) -> Quat {
    Quat::from_xyzw(-value.x, value.z, value.y, value.w)
}

/// Samples the root joint of `animation_index` on `CryEngine`'s key grid.
///
/// Returns `None` when the clip declares no root controller, carries unusable
/// timing extras, or resolves to a single key — `CryEngine` skips extraction in
/// exactly those cases.
#[must_use]
pub fn root_motion_samples(gltf: &gltf::Gltf, animation_index: usize) -> Option<RootMotionSamples> {
    let blob = gltf.blob.as_deref()?;
    let animation = gltf.animations().nth(animation_index)?;
    let buffer_data = |buffer: gltf::Buffer<'_>| (buffer.index() == 0).then_some(blob);
    let extras = animation_extras(&animation)?;
    if !extras.cry_sample_rate.is_finite()
        || extras.cry_sample_rate <= 0.0
        || !extras.cry_duration.is_finite()
        || extras.cry_duration <= 0.0
    {
        return None;
    }
    let root_controller_id = extras
        .controller_binding
        .as_ref()
        .filter(|binding| binding.uses_controller_targets())
        .and_then(|binding| binding.azoth_root_controller_id)?;
    let root_node_index = animation_controller_node_index(gltf, root_controller_id)?;
    let key_count = root_motion_key_count(extras.cry_duration, extras.cry_sample_rate)?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "`key_count` is bounded by `MAX_ROOT_MOTION_KEYS` (65536), which converts to \
                  `f32` exactly, and the division order mirrors CryEngine's key grid"
    )]
    let sample_times = (0..key_count)
        .map(|key| extras.cry_duration * key as f32 / (key_count - 1) as f32)
        .collect::<Vec<_>>();

    let mut positions = Vec::new();
    let mut rotations = Vec::new();
    for channel in animation
        .channels()
        .filter(|channel| channel.target().node().index() == root_node_index)
    {
        let interpolation = channel.sampler().interpolation();
        let reader = channel.reader(buffer_data);
        let times = reader.read_inputs()?.collect::<Vec<_>>();
        if times.is_empty() {
            return None;
        }
        let expected_values = animation_output_count(&channel, times.len());
        match reader.read_outputs()? {
            ReadOutputs::Translations(outputs) => {
                let values = outputs.collect::<Vec<_>>();
                if !positions.is_empty() || values.len() != expected_values {
                    return None;
                }
                let curve = TranslationCurve {
                    times,
                    values,
                    interpolation,
                };
                positions = sample_times
                    .iter()
                    .map(|time| curve.sample(*time).map(cry_translation))
                    .collect::<Option<Vec<_>>>()?;
            }
            ReadOutputs::Rotations(outputs) => {
                let values = outputs.into_f32().collect::<Vec<_>>();
                if !rotations.is_empty() || values.len() != expected_values {
                    return None;
                }
                let curve = RotationCurve {
                    times,
                    values,
                    interpolation,
                };
                rotations = sample_times
                    .iter()
                    .map(|time| curve.sample(*time).map(cry_rotation))
                    .collect::<Option<Vec<_>>>()?;
            }
            ReadOutputs::Scales(_) | ReadOutputs::MorphTargetWeights(_) => {}
        }
    }

    Some(RootMotionSamples {
        positions,
        rotations,
        sample_rate: extras.cry_sample_rate,
    })
}

/// `CryEngine`'s `numKeys = uint32(duration * sampleRate + 1)`; a single-key
/// clip carries no motion and is skipped.
fn root_motion_key_count(duration: f32, sample_rate: f32) -> Option<usize> {
    // Kept as two steps so the product rounds exactly like CryEngine's
    // `fDuration * fSampleRate + 1` instead of being fused into one rounding.
    let sampled = duration * sample_rate;
    let keys = sampled + 1.0;
    if !keys.is_finite() {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "this is CryEngine's own `uint32(...)` truncation; `keys` is finite here and \
                  Rust's saturating cast leaves out-of-range values outside the accepted \
                  2..=MAX_ROOT_MOTION_KEYS range checked below"
    )]
    let keys = keys as usize;
    (2..=MAX_ROOT_MOTION_KEYS).contains(&keys).then_some(keys)
}

#[must_use]
pub fn animation_duration(gltf: &gltf::Gltf, animation_index: usize) -> Option<f32> {
    let animation = gltf.animations().nth(animation_index)?;
    let duration = animation_extras(&animation)?.cry_duration;
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

#[must_use]
pub fn animation_controller_binding(
    gltf: &gltf::Gltf,
    animation_index: usize,
) -> Option<AnimationControllerBindingExtras> {
    let animation = gltf.animations().nth(animation_index)?;
    animation_extras(&animation)?.controller_binding
}

#[must_use]
pub fn animation_controller_node_index(gltf: &gltf::Gltf, controller_id: u32) -> Option<usize> {
    gltf.nodes().find_map(|node| {
        let extras = node.extras().as_ref()?;
        let extras = serde_json::from_str::<AnimationControllerNodeExtras>(extras.get()).ok()?;
        (extras.azoth_animation_controller_id == controller_id).then_some(node.index())
    })
}

fn animation_extras(animation: &gltf::Animation<'_>) -> Option<AnimationTimingExtras> {
    animation
        .extras()
        .as_ref()
        .and_then(|extras| serde_json::from_str::<AnimationTimingExtras>(extras.get()).ok())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnimationTimingExtras {
    cry_duration: f32,
    cry_sample_rate: f32,
    #[serde(default)]
    cry_events: Vec<AnimationTimingEvent>,
    #[serde(flatten)]
    controller_binding: Option<AnimationControllerBindingExtras>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnimationTimingEvent {
    name: String,
    normalized_time: f32,
}

fn motion_segment_boundaries(events: &[AnimationTimingEvent]) -> Option<Vec<f32>> {
    let mut interior = [None; 3];
    for event in events {
        let index = match event.name.to_ascii_lowercase().as_str() {
            "segment1" => 0,
            "segment2" => 1,
            "segment3" => 2,
            _ => continue,
        };
        interior[index] = Some(event.normalized_time);
    }

    let mut boundaries = Vec::with_capacity(5);
    boundaries.push(0.0);
    let mut missing = false;
    for time in interior {
        match time {
            Some(_) if missing => return None,
            Some(time) => boundaries.push(time),
            None => missing = true,
        }
    }
    boundaries.push(1.0);
    boundaries
        .windows(2)
        .all(|pair| pair[0].is_finite() && pair[0] < pair[1])
        .then_some(boundaries)
}

struct TranslationCurve {
    times: Vec<f32>,
    values: Vec<[f32; 3]>,
    interpolation: gltf::animation::Interpolation,
}

impl TranslationCurve {
    #[expect(
        clippy::suboptimal_flops,
        reason = "the cubic-spline basis reproduces the glTF specification's Hermite \
                  polynomials term by term; fusing a multiply into an add changes the \
                  rounding of every sampled root key"
    )]
    fn sample(&self, time: f32) -> Option<[f32; 3]> {
        let upper = self.times.partition_point(|key_time| *key_time <= time);
        if upper == 0 {
            return self.value(0);
        }
        if upper >= self.times.len() {
            return self.value(self.times.len() - 1);
        }
        let lower = upper - 1;
        let duration = self.times[upper] - self.times[lower];
        if duration <= 0.0 {
            return None;
        }
        let t = ((time - self.times[lower]) / duration).clamp(0.0, 1.0);
        match self.interpolation {
            gltf::animation::Interpolation::Step => self.value(lower),
            gltf::animation::Interpolation::Linear => {
                Some(vec3_lerp(self.value(lower)?, self.value(upper)?, t))
            }
            gltf::animation::Interpolation::CubicSpline => {
                let p0 = self.value(lower)?;
                let p1 = self.value(upper)?;
                let m0 = vec3_scale(self.values[lower * 3 + 2], duration);
                let m1 = vec3_scale(self.values[upper * 3], duration);
                let t2 = t * t;
                let t3 = t2 * t;
                Some(vec3_add(
                    vec3_add(
                        vec3_scale(p0, 2.0 * t3 - 3.0 * t2 + 1.0),
                        vec3_scale(m0, t3 - 2.0 * t2 + t),
                    ),
                    vec3_add(
                        vec3_scale(p1, -2.0 * t3 + 3.0 * t2),
                        vec3_scale(m1, t3 - t2),
                    ),
                ))
            }
        }
    }

    fn value(&self, index: usize) -> Option<[f32; 3]> {
        let index = if self.interpolation == gltf::animation::Interpolation::CubicSpline {
            index * 3 + 1
        } else {
            index
        };
        self.values.get(index).copied()
    }
}

struct RotationCurve {
    times: Vec<f32>,
    values: Vec<[f32; 4]>,
    interpolation: gltf::animation::Interpolation,
}

impl RotationCurve {
    #[expect(
        clippy::suboptimal_flops,
        reason = "the cubic-spline basis reproduces the glTF specification's Hermite \
                  polynomials term by term; fusing a multiply into an add changes the \
                  rounding of every sampled root key"
    )]
    fn sample(&self, time: f32) -> Option<Quat> {
        let upper = self.times.partition_point(|key_time| *key_time <= time);
        if upper == 0 {
            return self.value(0);
        }
        if upper >= self.times.len() {
            return self.value(self.times.len() - 1);
        }
        let lower = upper - 1;
        let duration = self.times[upper] - self.times[lower];
        if duration <= 0.0 {
            return None;
        }
        let t = ((time - self.times[lower]) / duration).clamp(0.0, 1.0);
        match self.interpolation {
            gltf::animation::Interpolation::Step => self.value(lower),
            gltf::animation::Interpolation::Linear => {
                Some(self.value(lower)?.slerp(self.value(upper)?, t))
            }
            gltf::animation::Interpolation::CubicSpline => {
                let p0 = self.value(lower)?;
                let p1 = self.value(upper)?;
                let m0 = quat_from_array(self.values.get(lower * 3 + 2)?) * duration;
                let m1 = quat_from_array(self.values.get(upper * 3)?) * duration;
                let t2 = t * t;
                let t3 = t2 * t;
                let sampled = p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                    + m0 * (t3 - 2.0 * t2 + t)
                    + p1 * (-2.0 * t3 + 3.0 * t2)
                    + m1 * (t3 - t2);
                (sampled.length_squared() > 0.0).then(|| sampled.normalize())
            }
        }
    }

    fn value(&self, index: usize) -> Option<Quat> {
        let index = if self.interpolation == gltf::animation::Interpolation::CubicSpline {
            index * 3 + 1
        } else {
            index
        };
        self.values.get(index).map(quat_from_array)
    }
}

const fn quat_from_array(value: &[f32; 4]) -> Quat {
    Quat::from_array(*value)
}

fn vec3_add(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|index| lhs[index] + rhs[index])
}

fn vec3_scale(value: [f32; 3], scale: f32) -> [f32; 3] {
    value.map(|component| component * scale)
}

#[expect(
    clippy::suboptimal_flops,
    reason = "linear key interpolation is written as `a + (b - a) * t` exactly as the sampled \
              root keys were produced; fusing it changes the rounding"
)]
fn vec3_lerp(start: [f32; 3], end: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|index| start[index] + (end[index] - start[index]) * t)
}

fn vec3_distance(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs.iter()
        .zip(rhs)
        .map(|(lhs, rhs)| (rhs - lhs).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn validate_animation_authoring_source(
    source_path: &str,
    gltf: &gltf::Gltf,
) -> Result<(), AnimationProductError> {
    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| AnimationProductError::MissingBinaryBuffer {
            source_path: source_path.to_string(),
        })?;
    let buffer_data = |buffer: gltf::Buffer<'_>| (buffer.index() == 0).then_some(blob);

    let mut channel_count = 0usize;
    for animation in gltf.animations() {
        let controller_binding =
            animation_extras(&animation).and_then(|extras| extras.controller_binding);
        if let Some(binding) = &controller_binding {
            validate_controller_binding(source_path, gltf, binding)?;
        }
        for channel in animation.channels() {
            channel_count += 1;
            let target_node = channel.target().node();
            let Some(target_name) = target_node.name() else {
                return Err(AnimationProductError::UnnamedTargetNode {
                    source_path: source_path.to_string(),
                });
            };
            let controller_root_channel =
                controller_binding.is_some() && target_name == CONTROLLER_TARGET_ROOT_NAME;
            if controller_binding.is_some() && !controller_root_channel {
                validate_controller_target_name(source_path, &target_node, target_name)?;
            }

            let reader = channel.reader(buffer_data);
            let times = reader
                .read_inputs()
                .ok_or_else(|| AnimationProductError::MissingInputTimes {
                    source_path: source_path.to_string(),
                })?
                .collect::<Vec<_>>();
            validate_channel_key_times(source_path, &times)?;
            let expected_outputs = animation_output_count(&channel, times.len());

            match reader
                .read_outputs()
                .ok_or_else(|| missing_outputs(source_path))?
            {
                ReadOutputs::Translations(values) => {
                    let values = values.collect::<Vec<_>>();
                    if values.len() != expected_outputs {
                        return Err(missing_outputs(source_path));
                    }
                    if controller_root_channel
                        && !controller_root_translation_is_identity(&channel, &values)
                    {
                        return Err(invalid_empty_controller_animation(source_path));
                    }
                }
                ReadOutputs::Scales(values) => {
                    if controller_root_channel {
                        return Err(invalid_empty_controller_animation(source_path));
                    }
                    if values.count() != expected_outputs {
                        return Err(missing_outputs(source_path));
                    }
                }
                ReadOutputs::Rotations(values) => {
                    if controller_root_channel {
                        return Err(invalid_empty_controller_animation(source_path));
                    }
                    let rotations = values.into_f32().collect::<Vec<_>>();
                    if rotations.len() != expected_outputs {
                        return Err(missing_outputs(source_path));
                    }
                    validate_unit_rotations(source_path, &channel, &rotations)?;
                }
                ReadOutputs::MorphTargetWeights(_) => {
                    return Err(AnimationProductError::UnsupportedProperty {
                        source_path: source_path.to_string(),
                        property: channel.target().property(),
                    });
                }
            }
        }
    }

    if channel_count == 0 {
        return Err(AnimationProductError::NoAnimationChannels {
            source_path: source_path.to_string(),
        });
    }

    Ok(())
}

/// Rejects a controller binding that targets an unsupported space or that names
/// a root controller with no matching target node.
fn validate_controller_binding(
    source_path: &str,
    gltf: &gltf::Gltf,
    binding: &AnimationControllerBindingExtras,
) -> Result<(), AnimationProductError> {
    if !binding.uses_controller_targets() {
        return Err(AnimationProductError::UnsupportedControllerTargetSpace {
            source_path: source_path.to_string(),
            target_space: binding.azoth_animation_target_space.clone(),
        });
    }
    if let Some(controller_id) = binding.azoth_root_controller_id
        && animation_controller_node_index(gltf, controller_id).is_none()
    {
        return Err(AnimationProductError::MissingRootControllerTarget {
            source_path: source_path.to_string(),
            controller_id,
        });
    }
    Ok(())
}

/// Confirms that a channel of a controller-bound animation targets a node whose
/// name matches the controller ID carried in that node's extras.
fn validate_controller_target_name(
    source_path: &str,
    target_node: &gltf::Node<'_>,
    target_name: &str,
) -> Result<(), AnimationProductError> {
    let controller = target_node
        .extras()
        .as_ref()
        .and_then(|extras| serde_json::from_str::<AnimationControllerNodeExtras>(extras.get()).ok())
        .ok_or_else(|| AnimationProductError::MissingControllerTargetId {
            source_path: source_path.to_string(),
            node: target_name.to_string(),
        })?;
    let expected = controller_target_node_name(controller.azoth_animation_controller_id);
    if target_name != expected {
        return Err(AnimationProductError::InvalidControllerTargetName {
            source_path: source_path.to_string(),
            controller_id: controller.azoth_animation_controller_id,
            actual: target_name.to_string(),
            expected,
        });
    }
    Ok(())
}

/// Requires a channel to carry at least one key time and to keep those times
/// strictly increasing.
fn validate_channel_key_times(
    source_path: &str,
    times: &[f32],
) -> Result<(), AnimationProductError> {
    if times.is_empty() {
        return Err(AnimationProductError::MissingInputTimes {
            source_path: source_path.to_string(),
        });
    }
    if !times.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(AnimationProductError::NonIncreasingTimes {
            source_path: source_path.to_string(),
        });
    }
    Ok(())
}

/// Requires every rotation key to be unit length. Cubic-spline tangents are
/// skipped, so only the value key of each triple is checked.
fn validate_unit_rotations(
    source_path: &str,
    channel: &gltf::animation::Channel<'_>,
    rotations: &[[f32; 4]],
) -> Result<(), AnimationProductError> {
    for (index, rotation) in rotations.iter().enumerate() {
        if channel.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline
            && index % 3 != 1
        {
            continue;
        }
        let norm = rotation
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if (norm - 1.0).abs() > 0.001 {
            return Err(AnimationProductError::NonUnitRotation {
                source_path: source_path.to_string(),
                norm,
            });
        }
    }
    Ok(())
}

/// Whether a controller-space root channel carries nothing but the authored
/// identity translation.
#[expect(
    clippy::float_cmp,
    reason = "the controller-space root channel is required to carry exactly zero, so the \
              comparison against `[0.0; 3]` is deliberately exact"
)]
fn controller_root_translation_is_identity(
    channel: &gltf::animation::Channel<'_>,
    values: &[[f32; 3]],
) -> bool {
    channel.target().property() == Property::Translation
        && !values.iter().any(|value| *value != [0.0; 3])
}

/// The error raised whenever a channel's output sampler is missing or the wrong
/// length.
fn missing_outputs(source_path: &str) -> AnimationProductError {
    AnimationProductError::MissingOutputs {
        source_path: source_path.to_string(),
    }
}

/// The error raised whenever the controller-space root channel carries anything
/// but an identity translation.
fn invalid_empty_controller_animation(source_path: &str) -> AnimationProductError {
    AnimationProductError::InvalidEmptyControllerAnimation {
        source_path: source_path.to_string(),
    }
}

fn animation_output_count(channel: &gltf::animation::Channel<'_>, input_count: usize) -> usize {
    input_count
        * if channel.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline {
            3
        } else {
            1
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_animation_glb_sources() {
        let registries = az_gem_contract::Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, "azoth.animation");
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("animations/player/idle.anim.glb"));
        assert!(!desc.matches("animations/player/idle.caf"));
        assert!(!desc.matches("animations/player/idle.glb"));
    }

    #[test]
    fn product_path_maps_anim_glb_to_motion_glb() {
        assert_eq!(
            animation_product_path("animations/player/idle.anim.glb"),
            "animations/player/idle.motion.glb"
        );
    }

    #[test]
    fn product_path_normalizes_backslashes() {
        assert_eq!(
            animation_product_path("animations\\player\\idle.anim.glb"),
            "animations/player/idle.motion.glb"
        );
    }

    #[test]
    fn animation_events_define_ordered_motion_segments() {
        let events = vec![
            AnimationTimingEvent {
                name: "segment2".to_owned(),
                normalized_time: 0.75,
            },
            AnimationTimingEvent {
                name: "segment1".to_owned(),
                normalized_time: 0.25,
            },
        ];

        assert_eq!(
            motion_segment_boundaries(&events),
            Some(vec![0.0, 0.25, 0.75, 1.0])
        );
    }

    #[test]
    fn cry_axes_map_gltf_up_forward_and_right_onto_cryengine_axes() {
        // glTF +Y is up and maps to CryEngine +Z; glTF +Z is the exporter's
        // forward and maps to CryEngine +Y; glTF +X mirrors onto CryEngine -X.
        assert_eq!(cry_translation([0.0, 1.0, 0.0]), Vec3::Z);
        assert_eq!(cry_translation([0.0, 0.0, 1.0]), Vec3::Y);
        assert_eq!(cry_translation([1.0, 0.0, 0.0]), Vec3::NEG_X);
    }

    #[test]
    fn cry_axes_turn_a_gltf_yaw_into_a_cryengine_yaw_of_the_same_sign() {
        let yaw = std::f32::consts::FRAC_PI_3;
        let converted = cry_rotation(Quat::from_rotation_y(yaw));
        let expected = Quat::from_rotation_z(yaw);

        assert!(
            converted.abs_diff_eq(expected, 1e-6),
            "expected {expected:?}, got {converted:?}"
        );
        // Rotating in either frame has to agree: converting a rotated vector
        // must equal rotating the converted vector by the converted rotation.
        for axis in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            let rotated_then_converted =
                cry_translation(Quat::from_rotation_y(yaw).mul_vec3(Vec3::from(axis)).into());
            let converted_then_rotated = converted * cry_translation(axis);
            assert!(
                rotated_then_converted.abs_diff_eq(converted_then_rotated, 1e-6),
                "expected {rotated_then_converted:?}, got {converted_then_rotated:?}"
            );
        }
    }

    #[test]
    fn root_motion_key_count_matches_cryengine_and_skips_single_key_clips() {
        assert_eq!(root_motion_key_count(1.0, 30.0), Some(31));
        assert_eq!(root_motion_key_count(0.5, 30.0), Some(16));
        // `numKeys == 1` is CryEngine's skip condition.
        assert_eq!(root_motion_key_count(0.0, 30.0), None);
        assert_eq!(root_motion_key_count(f32::INFINITY, 30.0), None);
        assert_eq!(root_motion_key_count(1.0e30, 30.0), None);
    }

    #[test]
    fn root_motion_key_count_matches_the_shipped_clip_timings() {
        // Some legacy exporters write `crySampleRate` as 29.999998, so the key
        // grid depends on how `duration * rate + 1` rounds in `f32`. These are
        // the exact extras of three shipped clips whose extracted `MoveSpeed`
        // the vendor's own baked `<VGrid>` confirms, so the rounding here is
        // load bearing: one key either way changes every extracted coordinate.
        assert_eq!(root_motion_key_count(1.666_666_7, 29.999_998), Some(51));
        assert_eq!(root_motion_key_count(1.333_333_4, 29.999_998), Some(41));
        assert_eq!(root_motion_key_count(0.466_666_7, 29.999_998), Some(15));
    }

    #[test]
    fn motion_segments_must_be_contiguous_and_increasing() {
        let gap = [AnimationTimingEvent {
            name: "segment2".to_owned(),
            normalized_time: 0.5,
        }];
        let reversed = [
            AnimationTimingEvent {
                name: "segment1".to_owned(),
                normalized_time: 0.75,
            },
            AnimationTimingEvent {
                name: "segment2".to_owned(),
                normalized_time: 0.25,
            },
        ];

        assert_eq!(motion_segment_boundaries(&gap), None);
        assert_eq!(motion_segment_boundaries(&reversed), None);
    }
}
