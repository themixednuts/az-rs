//! Editable and cooked parametric animation assets.

use std::{collections::BTreeSet, io, path::Path};

use arrayvec::ArrayVec;
use az_asset::AssetRef;
use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductDependency,
    ProductFormat, SourceFormat, TypedBuildProduct, resolve_referenced_product_id,
};
use az_core::{AssetData, AssetId, AzRtti, AzTypeInfo};
use az_filesystem::{engine_path_with_extension_key, normalize_source_path};
use bevy_asset::Asset;
use bevy_math::{Quat, Vec3};
use bevy_reflect::{Reflect, ReflectDeserialize, ReflectSerialize};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::{Uuid, uuid};

use crate::{
    animation_set::{AnimationMotionRef, AnimationRef},
    blend_space::{
        BlendSpaceDimension as RuntimeBlendSpaceDimension, BlendSpaceFace,
        CombinedBlendSpaceDescription,
        CombinedBlendSpaceDimension as RuntimeCombinedBlendSpaceDimension, CombinedSubSpace,
        DirectDeltaMotion, ExampleParameters, InvalidBlendSpace, InvalidCombinedBlendSpace,
        MAX_BLEND_SPACE_DIMENSIONS, MotionParameterId, ParametricBlendSpace,
        ParametricBlendSpaceDescription, PseudoExample, VirtualExample,
    },
    builder::RootMotionSamples,
};

pub const BLEND_SPACE_SOURCE_SCHEMA_NAME: &str = "azoth.animation.BlendSpaceSource";
pub const COMBINED_BLEND_SPACE_SOURCE_SCHEMA_NAME: &str =
    "azoth.animation.CombinedBlendSpaceSource";
pub const BLEND_SPACE_PRODUCT_FORMAT_NAME: &str = "azoth.animation.blend-space";
pub const COMBINED_BLEND_SPACE_PRODUCT_FORMAT_NAME: &str = "azoth.animation.combined-blend-space";
pub const VERSION: u32 = 1;
pub const PRIMARY_PRODUCT_SUB_ID: u32 = 0;

#[derive(SourceFormat)]
#[source(schema = "azoth.animation.BlendSpaceSource", ext = "bspace.ron")]
pub struct BlendSpaceSourceFormat;

#[derive(SourceFormat)]
#[source(schema = "azoth.animation.CombinedBlendSpaceSource", ext = "comb.ron")]
pub struct CombinedBlendSpaceSourceFormat;

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceSource {
    pub source_path: String,
    pub blend_space: BlendSpace,
}

impl BlendSpaceSource {
    /// Serializes this source as pretty-printed RON with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns the `ron::Error` raised by `ron::ser::to_string_pretty` when the
    /// source cannot be serialized.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Parses a RON authoring source.
    ///
    /// # Errors
    ///
    /// Returns the `ron::error::SpannedError` reported by the deserializer,
    /// which carries the position at which `bytes` stopped being a valid
    /// `BlendSpaceSource`.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct CombinedBlendSpaceSource {
    pub source_path: String,
    pub combined_blend_space: CombinedBlendSpace,
}

impl CombinedBlendSpaceSource {
    /// Serializes this source as pretty-printed RON with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns the `ron::Error` raised by `ron::ser::to_string_pretty` when the
    /// source cannot be serialized.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Parses a RON authoring source.
    ///
    /// # Errors
    ///
    /// Returns the `ron::error::SpannedError` reported by the deserializer,
    /// which carries the position at which `bytes` stopped being a valid
    /// `CombinedBlendSpaceSource`.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpace {
    pub threshold: Option<f32>,
    pub idle_to_move: bool,
    pub dimensions: Vec<BlendSpaceDimension>,
    pub examples: Vec<BlendSpaceExample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timewarp_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pseudo_examples: Vec<BlendSpacePseudoExample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_extraction: Vec<BlendSpaceAdditionalExtraction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<BlendSpaceAnnotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Retired authoring nodes preserved for source round trips; they have no product effect.
    pub motion_combinations: Vec<BlendSpaceMotionCombination>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<BlendSpaceJoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub virtual_examples: Vec<BlendSpaceVirtualExample>,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct CombinedBlendSpace {
    pub idle_to_move: bool,
    pub dimensions: Vec<CombinedBlendSpaceDimension>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timewarp_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_extraction: Vec<BlendSpaceAdditionalExtraction>,
    pub blend_spaces: Vec<BlendSpaceReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Retired authoring nodes preserved for source round trips; they have no product effect.
    pub motion_combinations: Vec<BlendSpaceMotionCombination>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<BlendSpaceJoint>,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceDimension {
    pub name: String,
    pub parameter_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_parameter_reason: Option<String>,
    pub min: f32,
    pub max: f32,
    pub cells: u8,
    pub debug_visual_scale: f32,
    pub start_key: f32,
    pub end_key: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joint_name: Option<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct CombinedBlendSpaceDimension {
    pub name: String,
    pub parameter_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_parameter_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    pub locked: bool,
    pub parameter_scale: f32,
    pub choose_blend_space: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceAdditionalExtraction {
    pub name: String,
    pub parameter_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_parameter_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceExample {
    pub animation: BlendSpaceAnimationRef,
    pub coordinates: Vec<BlendSpaceCoordinate>,
    pub playback_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceAnimationRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_motion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceCoordinate {
    pub dimension: String,
    pub value: Option<f32>,
    pub use_directly_for_delta_motion: bool,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpacePseudoExample {
    pub i0: i32,
    pub i1: i32,
    pub w0: f32,
    pub w1: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceAnnotation {
    pub indices: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceJoint {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceMotionCombination {
    pub animation: BlendSpaceAnimationRef,
}

#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceVirtualExample {
    pub indices: Vec<i32>,
    pub weights: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BlendSpaceReference {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_reference_reason: Option<String>,
}

#[derive(Asset, Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(from_reflect = false)]
pub struct BlendSpaceAsset {
    pub motions: Vec<BlendSpaceMotion>,
    pub timewarp_group: Option<String>,
    #[reflect(ignore)]
    pub sampler: ParametricBlendSpace,
}

impl AzTypeInfo for BlendSpaceAsset {
    const NAME: &'static str = "Azoth::Animation::BlendSpaceAsset";
    const TYPE_ID: Uuid = uuid!("b4c0bc52-75c8-4502-829e-2ffeb75e339c");
}

impl AzRtti for BlendSpaceAsset {}

impl AssetData for BlendSpaceAsset {
    const STABLE_NAME: &'static str = BLEND_SPACE_PRODUCT_FORMAT_NAME;
}

impl BlendSpaceAsset {
    pub fn referenced_asset_ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.motions
            .iter()
            .filter_map(|motion| motion.animation.id())
    }
}

#[derive(Asset, Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(from_reflect = false)]
pub struct CombinedBlendSpaceAsset {
    pub motions: Vec<BlendSpaceMotion>,
    #[reflect(ignore)]
    pub sampler: crate::blend_space::CombinedBlendSpace,
}

impl AzTypeInfo for CombinedBlendSpaceAsset {
    const NAME: &'static str = "Azoth::Animation::CombinedBlendSpaceAsset";
    const TYPE_ID: Uuid = uuid!("413196aa-8df1-4e87-a1db-39f9a4c3504d");
}

impl AzRtti for CombinedBlendSpaceAsset {}

impl AssetData for CombinedBlendSpaceAsset {
    const STABLE_NAME: &'static str = COMBINED_BLEND_SPACE_PRODUCT_FORMAT_NAME;
}

impl CombinedBlendSpaceAsset {
    pub fn referenced_asset_ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.motions
            .iter()
            .filter_map(|motion| motion.animation.id())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct BlendSpaceMotion {
    pub animation: AnimationRef,
    pub playback_scale: f32,
    pub direct_delta_motion: DirectDeltaMotion,
}

#[derive(ProductFormat)]
#[product_format(id = "azoth.animation.blend-space", version = 1, asset = BlendSpaceAsset)]
pub struct BlendSpaceProductFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.animation.combined-blend-space",
    version = 1,
    asset = CombinedBlendSpaceAsset
)]
pub struct CombinedBlendSpaceProductFormat;

pub trait MotionReferenceResolver {
    fn motion(&mut self, path: &str) -> Option<AnimationMotionRef>;

    /// Root-joint keys for the animation at `path`, used to extract example
    /// coordinates that the authoring source leaves unset. Returns `None` when
    /// the animation cannot be read or carries no sampled root motion.
    fn root_motion_samples(&mut self, path: &str) -> Option<RootMotionSamples>;
}

pub trait BlendSpaceSourceLoader {
    /// Loads the child blend-space authoring source referenced by `path`.
    ///
    /// # Errors
    ///
    /// Returns a `BlendSpaceSourceLoadError` when `path` is not a blend-space
    /// authoring path, when the source cannot be read, or when it does not
    /// parse as a `BlendSpaceSource`.
    fn load_blend_space(
        &mut self,
        path: &str,
    ) -> Result<BlendSpaceSource, BlendSpaceSourceLoadError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BlendSpaceCompiler;

impl BlendSpaceCompiler {
    /// Compiles an authored blend space into its runtime product.
    ///
    /// # Errors
    ///
    /// Returns a `BlendSpaceCompileError` when the source declares an
    /// out-of-range dimension count, names a motion parameter that cannot be
    /// resolved, carries an invalid explicit motion path, leaves an example
    /// coordinate unmapped, exceeds the runtime's example,
    /// annotation or virtual-contributor limits, or fails
    /// `ParametricBlendSpace` validation.
    pub fn blend_space(
        self,
        source: &BlendSpaceSource,
        resolver: &mut impl MotionReferenceResolver,
    ) -> Result<BlendSpaceAsset, BlendSpaceCompileError> {
        Self::blend_space_body(&source.blend_space, resolver)
    }

    /// Compiles an authored combined blend space, flattening every child blend
    /// space into one motion table.
    ///
    /// # Errors
    ///
    /// Returns a `BlendSpaceCompileError` for every failure
    /// [`Self::blend_space`] can report on a child source, plus
    /// `UnresolvedBlendSpaceSource` when a reference carries no authoring path,
    /// `SourceLoad` when a child source cannot be loaded, `TooManyExamples`
    /// when the merged motion table overflows, and the
    /// `InvalidCombinedBlendSpace` variants raised by combined-sampler
    /// validation.
    pub fn combined_blend_space(
        self,
        source: &CombinedBlendSpaceSource,
        resolver: &mut (impl MotionReferenceResolver + BlendSpaceSourceLoader),
    ) -> Result<CombinedBlendSpaceAsset, BlendSpaceCompileError> {
        Self::combined_blend_space_body(&source.combined_blend_space, resolver)
    }

    fn blend_space_body(
        source: &BlendSpace,
        resolver: &mut impl MotionReferenceResolver,
    ) -> Result<BlendSpaceAsset, BlendSpaceCompileError> {
        if !(1..=MAX_BLEND_SPACE_DIMENSIONS).contains(&source.dimensions.len()) {
            return Err(InvalidBlendSpace::DimensionCount(source.dimensions.len()).into());
        }

        let dimensions = runtime_dimensions(&source.dimensions)?;
        let additional_extraction = extraction_parameters(&source.additional_extraction)?;
        let (motions, example_parameters): (Vec<_>, Vec<_>) = source
            .examples
            .iter()
            .map(|example| blend_space_motion(example, &source.dimensions, resolver))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        let example_count = u8::try_from(motions.len())
            .map_err(|_| BlendSpaceCompileError::TooManyExamples(motions.len()))?;
        let pseudo_examples = runtime_pseudo_examples(&source.pseudo_examples)?;
        let faces = annotation_faces(&source.annotations)?;
        let virtual_examples = runtime_virtual_examples(&source.virtual_examples)?;
        let sampler = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions,
            additional_extraction,
            example_count,
            example_parameters,
            pseudo_examples,
            faces,
            virtual_examples,
            threshold: source.threshold,
            idle_to_move: source.idle_to_move,
        })?;

        let timewarp_group = source
            .timewarp_groups
            .last()
            .filter(|group| !group.is_empty())
            .cloned();

        Ok(BlendSpaceAsset {
            motions,
            timewarp_group,
            sampler,
        })
    }

    fn combined_blend_space_body(
        source: &CombinedBlendSpace,
        resolver: &mut (impl MotionReferenceResolver + BlendSpaceSourceLoader),
    ) -> Result<CombinedBlendSpaceAsset, BlendSpaceCompileError> {
        let dimensions = source
            .dimensions
            .iter()
            .map(|dimension| {
                Ok(RuntimeCombinedBlendSpaceDimension {
                    parameter: resolve_motion_parameter(
                        &dimension.name,
                        dimension.parameter_id,
                        dimension.unresolved_parameter_reason.as_deref(),
                    )?,
                    parameter_scale: dimension.parameter_scale,
                    choose_blend_space: dimension.choose_blend_space,
                    locked: dimension.locked,
                })
            })
            .collect::<Result<Vec<_>, BlendSpaceCompileError>>()?;
        let additional_extraction = extraction_parameters(&source.additional_extraction)?;

        let mut motions = Vec::<BlendSpaceMotion>::new();
        let mut blend_spaces = Vec::new();
        for reference in &source.blend_spaces {
            let path = reference.authoring_path.as_deref().ok_or_else(|| {
                BlendSpaceCompileError::UnresolvedBlendSpaceSource {
                    path: reference.path.clone(),
                    reason: reference.unresolved_reference_reason.clone(),
                }
            })?;
            let child_source = resolver.load_blend_space(path)?;
            let child = Self::blend_space_body(&child_source.blend_space, resolver)?;
            let mut example_indices = ArrayVec::new();
            for mut motion in child.motions {
                motion.direct_delta_motion = DirectDeltaMotion::default();
                let index = motions
                    .iter()
                    .position(|existing| {
                        existing.animation.references_same_motion(&motion.animation)
                            && existing.playback_scale.to_bits() == motion.playback_scale.to_bits()
                    })
                    .unwrap_or_else(|| {
                        motions.push(motion);
                        motions.len() - 1
                    });
                example_indices
                    .try_push(
                        u8::try_from(index)
                            .map_err(|_| BlendSpaceCompileError::TooManyExamples(motions.len()))?,
                    )
                    .map_err(|_| BlendSpaceCompileError::TooManyExamples(motions.len()))?;
            }
            blend_spaces.push(CombinedSubSpace {
                blend_space: child.sampler,
                example_indices,
            });
        }
        let example_count = u8::try_from(motions.len())
            .map_err(|_| BlendSpaceCompileError::TooManyExamples(motions.len()))?;
        let sampler =
            crate::blend_space::CombinedBlendSpace::try_from(CombinedBlendSpaceDescription {
                dimensions,
                additional_extraction,
                example_count,
                blend_spaces,
                idle_to_move: source.idle_to_move,
            })?;

        Ok(CombinedBlendSpaceAsset { motions, sampler })
    }
}

/// Converts an authored example index to the runtime `u8` slot.
fn example_index(index: i32) -> Result<u8, BlendSpaceCompileError> {
    u8::try_from(index).map_err(|_| BlendSpaceCompileError::InvalidExampleIndex { index })
}

/// Resolves the authored blend-space dimensions into their runtime form.
fn runtime_dimensions(
    dimensions: &[BlendSpaceDimension],
) -> Result<Vec<RuntimeBlendSpaceDimension>, BlendSpaceCompileError> {
    dimensions
        .iter()
        .map(|dimension| {
            Ok(RuntimeBlendSpaceDimension {
                parameter: resolve_motion_parameter(
                    &dimension.name,
                    dimension.parameter_id,
                    dimension.unresolved_parameter_reason.as_deref(),
                )?,
                min: dimension.min,
                max: dimension.max,
                cells: dimension.cells,
                locked: dimension.locked,
            })
        })
        .collect::<Result<Vec<_>, BlendSpaceCompileError>>()
}

/// Resolves the additional parameters a blend space extracts but does not
/// blend on.
fn extraction_parameters(
    parameters: &[BlendSpaceAdditionalExtraction],
) -> Result<Vec<MotionParameterId>, BlendSpaceCompileError> {
    parameters
        .iter()
        .map(|parameter| {
            resolve_motion_parameter(
                &parameter.name,
                parameter.parameter_id,
                parameter.unresolved_parameter_reason.as_deref(),
            )
        })
        .collect()
}

/// Converts the authored pseudo examples into their runtime form.
fn runtime_pseudo_examples(
    pseudo_examples: &[BlendSpacePseudoExample],
) -> Result<Vec<PseudoExample>, BlendSpaceCompileError> {
    pseudo_examples
        .iter()
        .map(|pseudo| {
            Ok(PseudoExample {
                i0: example_index(pseudo.i0)?,
                w0: pseudo.w0,
                i1: example_index(pseudo.i1)?,
                w1: pseudo.w1,
            })
        })
        .collect::<Result<Vec<_>, BlendSpaceCompileError>>()
}

/// Converts the authored annotations into the runtime faces spanning the
/// example cloud.
fn annotation_faces(
    annotations: &[BlendSpaceAnnotation],
) -> Result<Vec<BlendSpaceFace>, BlendSpaceCompileError> {
    annotations
        .iter()
        .map(|annotation| {
            let mut indices = ArrayVec::new();
            for &index in &annotation.indices {
                indices.try_push(example_index(index)?).map_err(|_| {
                    BlendSpaceCompileError::TooManyFacePoints {
                        actual: annotation.indices.len(),
                    }
                })?;
            }
            Ok(BlendSpaceFace { indices })
        })
        .collect::<Result<Vec<_>, BlendSpaceCompileError>>()
}

/// Converts the authored virtual-example grid into its runtime form, in packed
/// grid order.
fn runtime_virtual_examples(
    virtual_examples: &[BlendSpaceVirtualExample],
) -> Result<Vec<VirtualExample>, BlendSpaceCompileError> {
    virtual_examples
        .iter()
        .enumerate()
        .map(|(grid_index, example)| {
            let mut indices = ArrayVec::new();
            for &index in &example.indices {
                indices
                    .try_push(u8::try_from(index).map_err(|_| {
                        BlendSpaceCompileError::InvalidVirtualExampleIndex { grid_index, index }
                    })?)
                    .map_err(|_| BlendSpaceCompileError::TooManyVirtualContributors {
                        grid_index,
                    })?;
            }
            let mut weights = ArrayVec::new();
            for &weight in &example.weights {
                weights.try_push(weight).map_err(|_| {
                    BlendSpaceCompileError::TooManyVirtualContributors { grid_index }
                })?;
            }
            Ok(VirtualExample { indices, weights })
        })
        .collect::<Result<Vec<_>, BlendSpaceCompileError>>()
}

/// Compiles one example, returning its motion and the motion-parameter
/// coordinates the virtual grid is built from.
fn blend_space_motion(
    source: &BlendSpaceExample,
    dimensions: &[BlendSpaceDimension],
    resolver: &mut impl MotionReferenceResolver,
) -> Result<(BlendSpaceMotion, ExampleParameters), BlendSpaceCompileError> {
    let path = source.animation.motion_path.as_deref();
    let animation = match path {
        Some(path) => AnimationRef::new(
            &source.animation.name,
            resolver.motion(path).ok_or_else(|| {
                BlendSpaceCompileError::UnresolvedMotionReference {
                    path: path.to_owned(),
                }
            })?,
        ),
        None => AnimationRef::alias(&source.animation.name),
    };

    // CryEngine treats an omitted `SetPara<N>` as "derive this coordinate from
    // the clip's root motion", so the animation is only sampled on demand and
    // at most once per example.
    let mut samples = None;
    let mut direct = [(false, 0.0f32); 4];
    let mut parameters: ExampleParameters = [0.0; MAX_BLEND_SPACE_DIMENSIONS];
    for (index, dimension) in dimensions.iter().enumerate() {
        let coordinate = source
            .coordinates
            .iter()
            .find(|coordinate| coordinate.dimension == dimension.name)
            .ok_or_else(|| BlendSpaceCompileError::MissingExampleCoordinate {
                animation: source.animation.name.clone(),
                dimension: dimension.name.clone(),
            })?;
        let value = if let Some(value) = coordinate.value {
            value
        } else {
            let parameter = resolve_motion_parameter(
                &dimension.name,
                dimension.parameter_id,
                dimension.unresolved_parameter_reason.as_deref(),
            )?;
            let extracted = samples
                .get_or_insert_with(|| path.and_then(|path| resolver.root_motion_samples(path)))
                .as_ref()
                .ok_or(ZeroCoordinateReason::NoRootMotion)
                .and_then(|samples| {
                    extract_example_coordinate(parameter, dimension, source.playback_scale, samples)
                });
            extracted.unwrap_or_else(|reason| {
                // `BSParameter::BSParameter` zero-initializes `m_Para`, and every
                // `Init_*` extractor `continue`s instead of failing, so CryEngine
                // leaves the example at 0.0 in each of these cases.
                tracing::debug!(
                    animation = %source.animation.name,
                    dimension = %dimension.name,
                    reason = reason.as_str(),
                    "blend-space example coordinate falls back to CryEngine's zero default"
                );
                0.0
            })
        };
        direct[index] = (coordinate.use_directly_for_delta_motion, value);
        if let Some(slot) = parameters.get_mut(index) {
            *slot = value;
        }
    }

    Ok((
        BlendSpaceMotion {
            animation,
            playback_scale: source.playback_scale,
            direct_delta_motion: DirectDeltaMotion::from_dimensions(direct),
        },
        parameters,
    ))
}

/// Why an omitted example coordinate kept `CryEngine`'s zero default instead of
/// an extracted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZeroCoordinateReason {
    /// `ParameterExtraction` has no `Init_*` case for this parameter.
    NoExtractor,
    /// The clip declares no root controller, or its animation product cannot be
    /// read. Every `Init_*` extractor opens with
    /// `if (pController == 0) { continue; }`.
    NoRootMotion,
    /// `CryEngine`'s `numKeys == 1`, which the extractors skip.
    SingleKeyClip,
    /// The root controller carries no channel this parameter reads.
    MissingRootChannel,
}

impl ZeroCoordinateReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NoExtractor => "parameter has no CryEngine root-motion extractor",
            Self::NoRootMotion => "animation has no readable root controller",
            Self::SingleKeyClip => "clip resolves to a single key",
            Self::MissingRootChannel => "root controller lacks the channel this parameter reads",
        }
    }
}

/// `CryEngine`'s `ParameterExtraction` dispatch table
/// (`GlobalAnimationHeaderLMG.cpp`).
///
/// Only these six parameters have an `Init_*` extractor. `BlendWeight*` and
/// `StopLeg` are runtime-driven, so their examples keep the value
/// `BSParameter::BSParameter` zero-initialized.
#[derive(Debug, Clone, Copy)]
enum RootMotionExtractor {
    TravelSpeed,
    TurnSpeed,
    TurnAngle,
    TravelAngle,
    TravelSlope,
    TravelDistance,
}

impl RootMotionExtractor {
    const fn for_parameter(parameter: MotionParameterId) -> Option<Self> {
        match parameter {
            MotionParameterId::TravelSpeed => Some(Self::TravelSpeed),
            MotionParameterId::TurnSpeed => Some(Self::TurnSpeed),
            MotionParameterId::TurnAngle => Some(Self::TurnAngle),
            MotionParameterId::TravelAngle => Some(Self::TravelAngle),
            MotionParameterId::TravelSlope => Some(Self::TravelSlope),
            MotionParameterId::TravelDistance => Some(Self::TravelDistance),
            _ => None,
        }
    }

    fn extract(
        self,
        samples: &RootMotionSamples,
        start_key: usize,
        end_key: usize,
        playback_scale: f32,
    ) -> Option<f32> {
        match self {
            // `Init_MoveSpeed` is the only extractor CryEngine scales by the
            // example's playback scale.
            Self::TravelSpeed => {
                Some(extract_travel_speed(samples, start_key, end_key)? * playback_scale)
            }
            Self::TurnSpeed => extract_turn_speed(samples, end_key),
            Self::TurnAngle => extract_turn_angle(samples, end_key),
            Self::TravelAngle => extract_travel_angle(samples, start_key, end_key),
            Self::TravelSlope => extract_travel_slope(samples, end_key),
            Self::TravelDistance => extract_travel_distance(samples),
        }
    }
}

/// Derives one omitted example coordinate from sampled root motion.
///
/// This is a port of the `Init_*` extractors in Lumberyard's
/// `GlobalAnimationHeaderLMG.cpp`, including their asymmetric use of the
/// dimension's start key. Each `Err` is a case `CryEngine` `continue`s past,
/// leaving the example at its zero-initialized `m_Para`.
fn extract_example_coordinate(
    parameter: MotionParameterId,
    dimension: &BlendSpaceDimension,
    playback_scale: f32,
    samples: &RootMotionSamples,
) -> Result<f32, ZeroCoordinateReason> {
    let extractor =
        RootMotionExtractor::for_parameter(parameter).ok_or(ZeroCoordinateReason::NoExtractor)?;
    // CryEngine skips extraction entirely when `numKeys == 1`.
    let last_key = samples
        .key_count()
        .checked_sub(1)
        .filter(|last| *last > 0)
        .ok_or(ZeroCoordinateReason::SingleKeyClip)?;
    let start_key = extraction_key(dimension.start_key, last_key);
    let end_key = extraction_key(dimension.end_key, last_key);
    extractor
        .extract(samples, start_key, end_key, playback_scale)
        .ok_or(ZeroCoordinateReason::MissingRootChannel)
}

/// `CryEngine`'s `uint32(normalizedKey * (numKeys - 1))` window bound. Rust's
/// float-to-integer casts saturate, so non-finite or out-of-range authoring
/// values land inside the sampled key range.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "this is CryEngine's own `uint32(...)` window bound; `last_key` is a sampled key \
              index bounded by `MAX_ROOT_MOTION_KEYS`, and the saturating cast plus the `min` \
              keep the result inside the sampled key range"
)]
fn extraction_key(normalized: f32, last_key: usize) -> usize {
    ((normalized * last_key as f32) as usize).min(last_key)
}

/// `Init_MoveSpeed`: the mean per-key travel across `[startKey, endKey)`.
///
/// The caller applies the example's playback scale; this is the only extracted
/// parameter `CryEngine` scales that way.
#[expect(
    clippy::suboptimal_flops,
    clippy::cast_precision_loss,
    reason = "bit-exact port of `Init_MoveSpeed`: the multiply and the add must stay separate \
              to keep the compiled coordinate, and `poses` is a sampled key count bounded by \
              `MAX_ROOT_MOTION_KEYS`"
)]
fn extract_travel_speed(
    samples: &RootMotionSamples,
    start_key: usize,
    end_key: usize,
) -> Option<f32> {
    let positions = samples.positions()?;
    let mut travel_speed = 0.0f32;
    let mut poses = 0usize;
    for key in start_key..end_key {
        let from = *positions.get(key)?;
        let to = *positions.get(key + 1)?;
        travel_speed += from.distance(to) * samples.sample_rate();
        poses += 1;
    }
    if poses != 0 {
        travel_speed /= poses as f32;
    }
    Some(travel_speed)
}

/// `Init_TurnSpeed`: the mean per-key heading change.
#[expect(
    clippy::cast_precision_loss,
    reason = "`poses` is a sampled key count bounded by `MAX_ROOT_MOTION_KEYS`, which converts \
              to `f32` exactly"
)]
fn extract_turn_speed(samples: &RootMotionSamples, end_key: usize) -> Option<f32> {
    let rotations = samples.rotations()?;
    let mut turn_speed = 0.0f32;
    let mut poses = 0usize;
    // CryEngine quirk: turn speed integrates from key zero and ignores the
    // dimension's start key.
    for key in 0..end_key {
        let from = heading_axis(*rotations.get(key)?);
        let to = heading_axis(*rotations.get(key + 1)?);
        turn_speed += create_rad_z(from, to) * samples.sample_rate();
        poses += 1;
    }
    if poses != 0 {
        turn_speed /= poses as f32;
    }
    Some(turn_speed)
}

/// `Init_TurnAngle`: the summed heading change.
fn extract_turn_angle(samples: &RootMotionSamples, end_key: usize) -> Option<f32> {
    let rotations = samples.rotations()?;
    let mut turn_angle = 0.0f32;
    // CryEngine quirk: turn angle integrates from key zero, ignores the
    // dimension's start key, and is a sum rather than a mean.
    for key in 0..end_key {
        let from = heading_axis(*rotations.get(key)?);
        let to = heading_axis(*rotations.get(key + 1)?);
        turn_angle += create_rad_z(from, to);
    }
    Some(turn_angle)
}

/// `Init_TravelAngle`: the heading of the summed root-relative movement.
fn extract_travel_angle(
    samples: &RootMotionSamples,
    start_key: usize,
    end_key: usize,
) -> Option<f32> {
    let positions = samples.positions()?;
    let rotations = samples.rotations()?;
    let mut total_movement = Vec3::ZERO;
    for key in start_key..end_key {
        let from = *positions.get(key)?;
        let to = *positions.get(key + 1)?;
        let rotation = *rotations.get(key + 1)?;
        total_movement += rotation.inverse() * (to - from);
    }
    Some(create_rad_z(Vec3::Y, total_movement.normalize_or(Vec3::Y)))
}

/// `Init_SlopeAngle`: the mean vertical angle of each key step.
#[expect(
    clippy::cast_precision_loss,
    reason = "`poses` is a sampled key count bounded by `MAX_ROOT_MOTION_KEYS`, which converts \
              to `f32` exactly"
)]
fn extract_travel_slope(samples: &RootMotionSamples, end_key: usize) -> Option<f32> {
    let positions = samples.positions()?;
    let mut slope = 0.0f32;
    let mut poses = 0usize;
    // CryEngine quirk: the slope integrates from key zero and ignores the
    // dimension's start key.
    for key in 0..end_key {
        let from = *positions.get(key)?;
        let to = *positions.get(key + 1)?;
        let relative = (to - from).normalize_or(Vec3::Y);
        let heading = (-relative.x).atan2(relative.y);
        // CryEngine multiplies the row vector by `Matrix33::CreateRotationZ`,
        // which is the transposed - that is, inverted - rotation in the
        // column-vector convention used here. It aligns the step with +Y so the
        // remaining angle is purely vertical.
        let aligned = Quat::from_rotation_z(-heading) * relative;
        slope += aligned.z.atan2(aligned.y);
        poses += 1;
    }
    if poses != 0 {
        slope /= poses as f32;
    }
    Some(slope)
}

/// `Init_TravelDist`: the straight-line distance between the first and last
/// key. `CryEngine` applies neither the dimension window nor the playback
/// scale.
fn extract_travel_distance(samples: &RootMotionSamples) -> Option<f32> {
    let positions = samples.positions()?;
    Some(positions.first()?.distance(*positions.last()?))
}

/// `CryEngine`'s `Ang3::CreateRadZ`: the signed angle from `from` to `to` in
/// the XY plane. Z is ignored.
#[expect(
    clippy::suboptimal_flops,
    reason = "bit-exact port of `Ang3::CreateRadZ`; the two cross and dot terms are each a \
              multiply followed by an add or subtract, and fusing either changes the extracted \
              example coordinate"
)]
fn create_rad_z(from: Vec3, to: Vec3) -> f32 {
    (from.x * to.y - from.y * to.x).atan2(from.x * to.x + from.y * to.y)
}

/// `CryEngine`'s `Quat::GetColumn1`: the rotation's local +Y axis, which is the
/// forward direction its heading extractors compare.
fn heading_axis(rotation: Quat) -> Vec3 {
    rotation * Vec3::Y
}

fn resolve_motion_parameter(
    name: &str,
    parameter_id: Option<u8>,
    reason: Option<&str>,
) -> Result<MotionParameterId, BlendSpaceCompileError> {
    let Some(parameter_id) = parameter_id else {
        // Older sources can omit the numeric ID, leaving the authored CryEngine
        // parameter name as the only surviving link.
        return MotionParameterId::from_cry_name(name).ok_or_else(|| {
            BlendSpaceCompileError::UnresolvedMotionParameter {
                name: name.to_owned(),
                reason: reason.map(ToOwned::to_owned),
            }
        });
    };
    MotionParameterId::try_from(parameter_id).map_err(|_| {
        BlendSpaceCompileError::UnknownMotionParameter {
            name: name.to_owned(),
            parameter_id,
        }
    })
}

#[derive(Debug, thiserror::Error)]
pub enum BlendSpaceCompileError {
    #[error(transparent)]
    InvalidBlendSpace(#[from] InvalidBlendSpace),
    #[error(transparent)]
    InvalidCombinedBlendSpace(#[from] InvalidCombinedBlendSpace),
    #[error(transparent)]
    SourceLoad(#[from] BlendSpaceSourceLoadError),
    #[error("cannot resolve animation motion source `{path}`")]
    UnresolvedMotionReference { path: String },
    #[error("motion parameter `{name}` is unresolved{reason}", reason = .reason.as_deref().map(|reason| format!(": {reason}")).unwrap_or_default())]
    UnresolvedMotionParameter {
        name: String,
        reason: Option<String>,
    },
    #[error("motion parameter `{name}` has unknown id {parameter_id}")]
    UnknownMotionParameter { name: String, parameter_id: u8 },
    #[error("blend-space example `{animation}` has no `{dimension}` coordinate")]
    MissingExampleCoordinate {
        animation: String,
        dimension: String,
    },
    #[error("blend space has {0} examples; the runtime maximum is 40")]
    TooManyExamples(usize),
    #[error("virtual-grid entry {grid_index} has an invalid example index {index}")]
    InvalidVirtualExampleIndex { grid_index: usize, index: i32 },
    #[error("virtual-grid entry {grid_index} has more than eight contributors")]
    TooManyVirtualContributors { grid_index: usize },
    #[error("blend-space annotation has an invalid example index {index}")]
    InvalidExampleIndex { index: i32 },
    #[error("blend-space annotation has {actual} points; the runtime maximum is 8")]
    TooManyFacePoints { actual: usize },
    #[error("cannot load child blend space `{path}`{reason}", reason = .reason.as_deref().map(|reason| format!(": {reason}")).unwrap_or_default())]
    UnresolvedBlendSpaceSource {
        path: String,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot load blend-space source `{path}`: {reason}")]
pub struct BlendSpaceSourceLoadError {
    pub path: String,
    pub reason: String,
}

impl BlendSpaceSourceLoadError {
    pub fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlendSpaceAssetCodec<T> {
    marker: std::marker::PhantomData<T>,
}

impl<T> Default for BlendSpaceAssetCodec<T> {
    fn default() -> Self {
        Self {
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: Serialize + DeserializeOwned> BlendSpaceAssetCodec<T> {
    /// Writes `asset` to `writer` in the cooked postcard encoding.
    ///
    /// # Errors
    ///
    /// Returns `BlendSpaceAssetCodecError::Codec` when the asset cannot be
    /// encoded and `BlendSpaceAssetCodecError::Write` when `writer` fails.
    pub fn write(
        &self,
        asset: &T,
        writer: &mut impl io::Write,
    ) -> Result<(), BlendSpaceAssetCodecError> {
        writer.write_all(&postcard::to_allocvec(asset)?)?;
        Ok(())
    }

    /// Decodes a cooked asset from `bytes`.
    ///
    /// # Errors
    ///
    /// Returns `BlendSpaceAssetCodecError::Codec` when `bytes` is not a valid
    /// postcard encoding of `T`.
    pub fn read(&self, bytes: &[u8]) -> Result<T, BlendSpaceAssetCodecError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

/// Writes a cooked blend-space asset to `writer`.
///
/// # Errors
///
/// Returns `BlendSpaceAssetCodecError::Codec` when the asset cannot be encoded
/// and `BlendSpaceAssetCodecError::Write` when `writer` fails.
pub fn write_blend_space_asset(
    asset: &BlendSpaceAsset,
    writer: &mut impl io::Write,
) -> Result<(), BlendSpaceAssetCodecError> {
    BlendSpaceAssetCodec::<BlendSpaceAsset>::default().write(asset, writer)
}

/// Decodes a cooked blend-space asset.
///
/// # Errors
///
/// Returns `BlendSpaceAssetCodecError::Codec` when `bytes` is not a valid
/// encoding of a `BlendSpaceAsset`.
pub fn read_blend_space_asset(bytes: &[u8]) -> Result<BlendSpaceAsset, BlendSpaceAssetCodecError> {
    BlendSpaceAssetCodec::<BlendSpaceAsset>::default().read(bytes)
}

/// Writes a cooked combined blend-space asset to `writer`.
///
/// # Errors
///
/// Returns `BlendSpaceAssetCodecError::Codec` when the asset cannot be encoded
/// and `BlendSpaceAssetCodecError::Write` when `writer` fails.
pub fn write_combined_blend_space_asset(
    asset: &CombinedBlendSpaceAsset,
    writer: &mut impl io::Write,
) -> Result<(), BlendSpaceAssetCodecError> {
    BlendSpaceAssetCodec::<CombinedBlendSpaceAsset>::default().write(asset, writer)
}

/// Decodes a cooked combined blend-space asset.
///
/// # Errors
///
/// Returns `BlendSpaceAssetCodecError::Codec` when `bytes` is not a valid
/// encoding of a `CombinedBlendSpaceAsset`.
pub fn read_combined_blend_space_asset(
    bytes: &[u8],
) -> Result<CombinedBlendSpaceAsset, BlendSpaceAssetCodecError> {
    BlendSpaceAssetCodec::<CombinedBlendSpaceAsset>::default().read(bytes)
}

#[derive(Debug, thiserror::Error)]
pub enum BlendSpaceAssetCodecError {
    #[error("encode blend-space asset: {0}")]
    Codec(#[from] postcard::Error),
    #[error("write blend-space asset: {0}")]
    Write(#[from] io::Error),
}

pub const BLEND_SPACE_BUILDER_NAME: &str = "azoth.animation.blend-space";
pub const BLEND_SPACE_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("4f21f771-e730-40bc-aee7-4c48a44bbd3b"));
pub const COMBINED_BLEND_SPACE_BUILDER_NAME: &str = "azoth.animation.combined-blend-space";
pub const COMBINED_BLEND_SPACE_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("2f709cae-c292-4a28-a3b1-ecfba20477f6"));

#[must_use]
pub fn blend_space_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<BlendSpaceSourceFormat>()
        .named(BLEND_SPACE_BUILDER_NAME)
        .id(BLEND_SPACE_BUILDER_ID)
        .version(VERSION)
        .produces::<BlendSpaceProductFormat>()
        .create_jobs(create_jobs)
        .process(blend_space_process_job)
}

#[must_use]
pub fn combined_blend_space_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<CombinedBlendSpaceSourceFormat>()
        .named(COMBINED_BLEND_SPACE_BUILDER_NAME)
        .id(COMBINED_BLEND_SPACE_BUILDER_ID)
        .version(VERSION)
        .produces::<CombinedBlendSpaceProductFormat>()
        .create_jobs(create_jobs)
        .process(combined_blend_space_process_job)
}

fn create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        ..CreateJobsResponse::default()
    }
}

fn blend_space_process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    process_product(transform_blend_space_product_at_root(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ))
}

fn combined_blend_space_process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    process_product(transform_combined_blend_space_product_at_root(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ))
}

fn process_product(build: Result<BlendSpaceBuild, BlendSpaceProductError>) -> ProcessJobResponse {
    match build {
        Ok(build) => ProcessJobResponse {
            products: vec![build.product],
            product_dependencies: build
                .dependencies
                .into_iter()
                .map(|dependency| (PRIMARY_PRODUCT_SUB_ID as usize, dependency))
                .collect(),
            result: ProcessJobResult::Success,
        },
        Err(error) => {
            tracing::warn!(%error, "blend-space product failed");
            ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            }
        }
    }
}

pub struct BlendSpaceBuild {
    pub product: BuildProduct,
    pub dependencies: Vec<ProductDependency>,
}

/// Compiles a blend-space authoring source rooted at `source_root` into its
/// cooked product and the product dependencies it references.
///
/// # Errors
///
/// Returns `BlendSpaceProductError::ParseSource` when `source_bytes` is not a
/// valid `BlendSpaceSource`, `BlendSpaceProductError::CompileSource` for every
/// failure [`BlendSpaceCompiler::blend_space`] can report, and
/// `BlendSpaceProductError::Serialize` when the cooked asset cannot be encoded.
pub fn transform_blend_space_product_at_root(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BlendSpaceBuild, BlendSpaceProductError> {
    let source = BlendSpaceSource::from_ron_bytes(source_bytes)?;
    let mut resolver = SourceReferenceResolver::new(source_root);
    let asset = BlendSpaceCompiler.blend_space(&source, &mut resolver)?;
    let dependencies = unique_dependencies(asset.referenced_asset_ids());
    let mut bytes = Vec::new();
    write_blend_space_asset(&asset, &mut bytes)?;
    let product = TypedBuildProduct::<BlendSpaceProductFormat>::from_trusted_path(
        blend_space_product_path(source_path),
        PRIMARY_PRODUCT_SUB_ID,
        bytes,
    )
    .erase();
    Ok(BlendSpaceBuild {
        product,
        dependencies,
    })
}

/// Compiles a combined blend-space authoring source rooted at `source_root`
/// into its cooked product and the product dependencies it references.
///
/// # Errors
///
/// Returns `BlendSpaceProductError::ParseSource` when `source_bytes` is not a
/// valid `CombinedBlendSpaceSource`, `BlendSpaceProductError::CompileSource`
/// for every failure [`BlendSpaceCompiler::combined_blend_space`] can report,
/// and `BlendSpaceProductError::Serialize` when the cooked asset cannot be
/// encoded.
pub fn transform_combined_blend_space_product_at_root(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BlendSpaceBuild, BlendSpaceProductError> {
    let source = CombinedBlendSpaceSource::from_ron_bytes(source_bytes)?;
    let mut resolver = SourceReferenceResolver::new(source_root);
    let asset = BlendSpaceCompiler.combined_blend_space(&source, &mut resolver)?;
    let dependencies = unique_dependencies(asset.referenced_asset_ids());
    let mut bytes = Vec::new();
    write_combined_blend_space_asset(&asset, &mut bytes)?;
    let product = TypedBuildProduct::<CombinedBlendSpaceProductFormat>::from_trusted_path(
        combined_blend_space_product_path(source_path),
        PRIMARY_PRODUCT_SUB_ID,
        bytes,
    )
    .erase();
    Ok(BlendSpaceBuild {
        product,
        dependencies,
    })
}

fn unique_dependencies(ids: impl IntoIterator<Item = AssetId>) -> Vec<ProductDependency> {
    ids.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(ProductDependency::new)
        .collect()
}

struct SourceReferenceResolver<'a> {
    source_root: &'a Path,
}

impl<'a> SourceReferenceResolver<'a> {
    const fn new(source_root: &'a Path) -> Self {
        Self { source_root }
    }
}

impl MotionReferenceResolver for SourceReferenceResolver<'_> {
    fn motion(&mut self, path: &str) -> Option<AnimationMotionRef> {
        let source_path = normalize_source_path(path);
        source_path.ends_with(".anim.glb").then(|| {
            resolve_referenced_product_id(self.source_root, &source_path, PRIMARY_PRODUCT_SUB_ID)
                .ok()
                .map(|asset_id| AssetRef::new(asset_id, Some(source_path.clone())))
        })?
    }

    fn root_motion_samples(&mut self, path: &str) -> Option<RootMotionSamples> {
        let source_path = normalize_source_path(path);
        if !source_path.ends_with(".anim.glb") {
            return None;
        }
        let path_on_disk = self.source_root.join(&source_path);
        let bytes = std::fs::read(&path_on_disk)
            .inspect_err(|error| {
                tracing::debug!(
                    path = %path_on_disk.display(),
                    %error,
                    "blend-space example animation is unreadable"
                );
            })
            .ok()?;
        let gltf = gltf::Gltf::from_slice(&bytes).ok()?;
        crate::builder::root_motion_samples(&gltf, 0)
    }
}

impl BlendSpaceSourceLoader for SourceReferenceResolver<'_> {
    fn load_blend_space(
        &mut self,
        path: &str,
    ) -> Result<BlendSpaceSource, BlendSpaceSourceLoadError> {
        let source_path = blend_space_source_path(path).ok_or_else(|| {
            BlendSpaceSourceLoadError::new(path, "invalid blend-space authoring path")
        })?;
        let path_on_disk = self.source_root.join(&source_path);
        let bytes = std::fs::read(&path_on_disk).map_err(|error| {
            BlendSpaceSourceLoadError::new(
                path,
                format!("read {}: {error}", path_on_disk.display()),
            )
        })?;
        BlendSpaceSource::from_ron_bytes(&bytes)
            .map_err(|error| BlendSpaceSourceLoadError::new(path, error.to_string()))
    }
}

fn blend_space_source_path(path: &str) -> Option<String> {
    let normalized = normalize_source_path(path);
    if normalized.is_empty() {
        None
    } else if normalized.ends_with(".bspace.ron") {
        Some(normalized)
    } else {
        normalized
            .strip_suffix(".bspace")
            .map(|stem| format!("{stem}.bspace.ron"))
    }
}

#[must_use]
pub fn blend_space_product_path(source_path: &str) -> String {
    engine_path_with_extension_key(source_path, "", "blend-space.bin", Some("bspace.ron"))
}

#[must_use]
pub fn combined_blend_space_product_path(source_path: &str) -> String {
    engine_path_with_extension_key(
        source_path,
        "",
        "combined-blend-space.bin",
        Some("comb.ron"),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum BlendSpaceProductError {
    #[error("parse blend-space source RON: {0}")]
    ParseSource(#[from] ron::error::SpannedError),
    #[error("compile blend-space source: {0}")]
    CompileSource(#[from] BlendSpaceCompileError),
    #[error("serialize blend-space product: {0}")]
    Serialize(#[from] BlendSpaceAssetCodecError),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::source_asset_id;

    use super::*;

    /// Tolerance for extractor assertions. The expected values are hand
    /// computed from the synthetic keys, so only `f32` rounding differs.
    const EXTRACTION_EPSILON: f32 = 1e-5;

    #[derive(Default)]
    struct TestResolver {
        sources: Vec<(String, BlendSpaceSource)>,
        samples: Vec<(String, RootMotionSamples)>,
    }

    impl MotionReferenceResolver for TestResolver {
        fn motion(&mut self, path: &str) -> Option<AnimationMotionRef> {
            let path = normalize_source_path(path);
            path.ends_with(".anim.glb")
                .then(|| AssetRef::new(source_asset_id(&path, PRIMARY_PRODUCT_SUB_ID), Some(path)))
        }

        fn root_motion_samples(&mut self, path: &str) -> Option<RootMotionSamples> {
            let path = normalize_source_path(path);
            self.samples
                .iter()
                .find(|(candidate, _)| candidate == &path)
                .map(|(_, samples)| samples.clone())
        }
    }

    impl BlendSpaceSourceLoader for TestResolver {
        fn load_blend_space(
            &mut self,
            path: &str,
        ) -> Result<BlendSpaceSource, BlendSpaceSourceLoadError> {
            let path = normalize_source_path(path);
            self.sources
                .iter()
                .find(|(candidate, _)| candidate == &path)
                .map(|(_, source)| source.clone())
                .ok_or_else(|| BlendSpaceSourceLoadError::new(path, "missing test source"))
        }
    }

    fn source_dimension(min: f32, max: f32) -> BlendSpaceDimension {
        BlendSpaceDimension {
            name: "TravelSpeed".to_owned(),
            parameter_id: Some(u8::from(MotionParameterId::TravelSpeed)),
            unresolved_parameter_reason: None,
            min,
            max,
            cells: 2,
            debug_visual_scale: 1.0,
            start_key: 0.0,
            end_key: 1.0,
            joint_name: None,
            locked: false,
        }
    }

    fn source_example(name: &str, path: &str, value: f32) -> BlendSpaceExample {
        BlendSpaceExample {
            animation: BlendSpaceAnimationRef {
                name: name.to_owned(),
                motion_path: Some(path.to_owned()),
                unresolved_motion_reason: None,
            },
            coordinates: vec![BlendSpaceCoordinate {
                dimension: "TravelSpeed".to_owned(),
                value: Some(value),
                use_directly_for_delta_motion: true,
            }],
            playback_scale: 1.0,
        }
    }

    fn child_source(
        source_path: &str,
        animation_name: &str,
        motion_path: &str,
        min: f32,
        max: f32,
    ) -> BlendSpaceSource {
        BlendSpaceSource {
            source_path: source_path.to_owned(),
            blend_space: BlendSpace {
                threshold: None,
                idle_to_move: false,
                dimensions: vec![source_dimension(min, max)],
                examples: vec![source_example(animation_name, motion_path, min)],
                timewarp_groups: vec!["Locomotion".to_owned(), "LocomotionUpper".to_owned()],
                pseudo_examples: Vec::new(),
                additional_extraction: Vec::new(),
                annotations: Vec::new(),
                motion_combinations: Vec::new(),
                joints: Vec::new(),
                virtual_examples: vec![
                    BlendSpaceVirtualExample {
                        indices: vec![0, 0],
                        weights: vec![1.0, 0.0],
                    },
                    BlendSpaceVirtualExample {
                        indices: vec![0, 0],
                        weights: vec![1.0, 0.0],
                    },
                ],
            },
        }
    }

    const EXTRACTION_ANIMATION: &str = "animations/extract.anim.glb";

    fn sample_positions(values: [[f32; 3]; 4]) -> Vec<Vec3> {
        values.into_iter().map(Vec3::from).collect()
    }

    fn sample_yaws(values: [f32; 4]) -> Vec<Quat> {
        values.into_iter().map(Quat::from_rotation_z).collect()
    }

    /// Bounds wide enough that no extraction in this module lands near them.
    ///
    /// Extraction never consults the dimension bounds at all - see
    /// [`travel_speed_extraction_ignores_the_dimension_bounds`] - so every test
    /// that is not specifically about bounds uses these.
    const WIDE_EXTRACTION_BOUNDS: (f32, f32) = (-10.0, 10.0);

    fn extraction_source(
        parameter: MotionParameterId,
        start_key: f32,
        end_key: f32,
        playback_scale: f32,
        value: Option<f32>,
        bounds: (f32, f32),
    ) -> BlendSpaceSource {
        BlendSpaceSource {
            source_path: "animations/extract.bspace".to_owned(),
            blend_space: BlendSpace {
                threshold: None,
                idle_to_move: false,
                dimensions: vec![BlendSpaceDimension {
                    name: "Extracted".to_owned(),
                    parameter_id: Some(u8::from(parameter)),
                    unresolved_parameter_reason: None,
                    min: bounds.0,
                    max: bounds.1,
                    cells: 2,
                    debug_visual_scale: 1.0,
                    start_key,
                    end_key,
                    joint_name: None,
                    locked: false,
                }],
                examples: vec![BlendSpaceExample {
                    animation: BlendSpaceAnimationRef {
                        name: "extract".to_owned(),
                        motion_path: Some(EXTRACTION_ANIMATION.to_owned()),
                        unresolved_motion_reason: None,
                    },
                    coordinates: vec![BlendSpaceCoordinate {
                        dimension: "Extracted".to_owned(),
                        value,
                        use_directly_for_delta_motion: true,
                    }],
                    playback_scale,
                }],
                timewarp_groups: Vec::new(),
                pseudo_examples: Vec::new(),
                additional_extraction: Vec::new(),
                annotations: Vec::new(),
                motion_combinations: Vec::new(),
                joints: Vec::new(),
                virtual_examples: vec![
                    BlendSpaceVirtualExample {
                        indices: vec![0, 0],
                        weights: vec![1.0, 0.0],
                    },
                    BlendSpaceVirtualExample {
                        indices: vec![0, 0],
                        weights: vec![1.0, 0.0],
                    },
                ],
            },
        }
    }

    fn compile_extraction(
        parameter: MotionParameterId,
        start_key: f32,
        end_key: f32,
        playback_scale: f32,
        value: Option<f32>,
        bounds: (f32, f32),
        samples: RootMotionSamples,
    ) -> Result<BlendSpaceAsset, BlendSpaceCompileError> {
        let source =
            extraction_source(parameter, start_key, end_key, playback_scale, value, bounds);
        let mut resolver = TestResolver {
            sources: Vec::new(),
            samples: vec![(EXTRACTION_ANIMATION.to_owned(), samples)],
        };
        BlendSpaceCompiler.blend_space(&source, &mut resolver)
    }

    fn extracted_coordinate(
        parameter: MotionParameterId,
        start_key: f32,
        end_key: f32,
        playback_scale: f32,
        samples: RootMotionSamples,
    ) -> f32 {
        let asset = compile_extraction(
            parameter,
            start_key,
            end_key,
            playback_scale,
            None,
            WIDE_EXTRACTION_BOUNDS,
            samples,
        )
        .expect("blend space compiles from extracted coordinates");
        asset.motions[0]
            .direct_delta_motion
            .get(0)
            .expect("dimension zero is authored for direct delta motion")
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= EXTRACTION_EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn travel_speed_averages_key_travel_and_applies_the_playback_scale() {
        // Steps of 1, 2 and 3 units at 2 keys per second average to 4 units per
        // second; only travel speed carries the example's playback scale.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [0.0, 1.0, 0.0], [0.0, 3.0, 0.0], [0.0, 6.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelSpeed, 0.0, 1.0, 1.5, samples),
            6.0,
        );
    }

    /// Verifies `Init_MoveSpeed` with straight-line root-motion samples.
    ///
    /// A baked `<VGrid>` cell stores only example indices and blend weights -
    /// never a parameter value. A cell represents the coordinate sampled at
    /// `min + i * (max - min) / (cells - 1)` and equals an example's `m_Para`
    /// only when that coordinate lands on the example.
    ///
    /// For a straight-line root path, the per-key mean collapses to
    /// `distance / duration`; the sample rate cancels.
    #[test]
    fn travel_speed_matches_straight_line_coordinates() {
        for (key_count, last_key, distance, sample_rate, expected) in [
            // 2.4137917 m over 1.3333334 s.
            (
                41_usize,
                40.0_f32,
                2.413_791_7_f32,
                29.999_998_f32,
                1.810_343_7_f32,
            ),
            // 0.6249997 m over 1.6666667 s.
            (51, 50.0, 0.624_999_7, 29.999_998, 0.374_999_8),
        ] {
            let mut key = 0.0f32;
            let mut positions = Vec::with_capacity(key_count);
            for _ in 0..key_count {
                positions.push(Vec3::new(0.0, distance * key / last_key, 0.0));
                key += 1.0;
            }
            let samples = RootMotionSamples::new(positions, Vec::new(), sample_rate);

            assert_close(
                extracted_coordinate(MotionParameterId::TravelSpeed, 0.0, 1.0, 1.0, samples),
                expected,
            );
        }
    }

    /// Extraction does not clamp a coordinate to its blend-space dimension.
    /// The assignment at
    /// Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/GlobalAnimationHeaderLMG.cpp:1505`
    /// stores the extracted value directly, and `ParameterExtraction` at line
    /// 1401 only dispatches to the parameter-specific extractor. Bounds belong
    /// to grid sampling.
    #[test]
    fn travel_speed_extraction_ignores_the_dimension_bounds() {
        // Steps of 1, 2 and 3 units at 2 keys per second average to 4 units
        // per second - eight times this dimension's `max`.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [0.0, 1.0, 0.0], [0.0, 3.0, 0.0], [0.0, 6.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        let asset = compile_extraction(
            MotionParameterId::TravelSpeed,
            0.0,
            1.0,
            1.0,
            None,
            (0.0, 0.5),
            samples,
        )
        .expect("blend space compiles when extraction overshoots its dimension");

        assert_close(
            asset.motions[0]
                .direct_delta_motion
                .get(0)
                .expect("dimension zero is authored for direct delta motion"),
            4.0,
        );
    }

    #[test]
    fn travel_speed_honours_the_dimension_start_key() {
        // `start_key` 0.5 truncates to key 1, dropping the leading 1-unit step
        // and leaving the 2- and 3-unit steps to average to 5 units per second.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [0.0, 1.0, 0.0], [0.0, 3.0, 0.0], [0.0, 6.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelSpeed, 0.5, 1.0, 1.0, samples),
            5.0,
        );
    }

    #[test]
    fn turn_speed_averages_heading_change_from_key_zero() {
        // The only heading change is the final quarter turn. Averaging it over
        // all three key steps - not the two inside the window - is CryEngine's
        // start-key quirk: `2 * FRAC_PI_2 / 3`.
        let samples = RootMotionSamples::new(
            Vec::new(),
            sample_yaws([0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2]),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TurnSpeed, 0.5, 1.0, 1.0, samples),
            std::f32::consts::FRAC_PI_3,
        );
    }

    #[test]
    fn turn_angle_sums_heading_change_from_key_zero() {
        // Same keys as turn speed: CryEngine sums the heading change without
        // dividing by the pose count and without the sample rate.
        let samples = RootMotionSamples::new(
            Vec::new(),
            sample_yaws([0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2]),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TurnAngle, 0.5, 1.0, 1.0, samples),
            std::f32::consts::FRAC_PI_2,
        );
    }

    #[test]
    fn travel_angle_is_the_heading_of_root_relative_movement() {
        // Three +X steps seen from a root yawed a quarter turn left read as
        // movement to the back right: -3 * FRAC_PI_4.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]),
            sample_yaws([
                0.0,
                std::f32::consts::FRAC_PI_4,
                std::f32::consts::FRAC_PI_4,
                std::f32::consts::FRAC_PI_4,
            ]),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelAngle, 0.0, 1.0, 1.0, samples),
            -3.0 * std::f32::consts::FRAC_PI_4,
        );
    }

    #[test]
    fn travel_angle_honours_the_dimension_start_key() {
        // Dropping the leading +X step leaves pure +Y movement, so the heading
        // is zero instead of the -1.1902899 the whole clip would give.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [5.0, 0.0, 0.0], [5.0, 1.0, 0.0], [5.0, 2.0, 0.0]]),
            vec![Quat::IDENTITY; 4],
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelAngle, 0.5, 1.0, 1.0, samples),
            0.0,
        );
    }

    #[test]
    fn travel_slope_averages_the_vertical_angle_of_each_step() {
        // Every step rises as much as it travels along +X, so aligning the step
        // with +Y leaves a 45 degree climb.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 1.0], [2.0, 0.0, 2.0], [3.0, 0.0, 3.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelSlope, 0.0, 1.0, 1.0, samples),
            std::f32::consts::FRAC_PI_4,
        );
    }

    #[test]
    fn travel_slope_ignores_the_dimension_start_key() {
        // Only the middle step climbs, at atan(2). CryEngine averages it over
        // all three steps rather than the two inside the window.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 2.0], [3.0, 0.0, 2.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelSlope, 0.5, 1.0, 1.0, samples),
            2.0f32.atan() / 3.0,
        );
    }

    #[test]
    fn travel_distance_spans_the_whole_clip() {
        // CryEngine measures the first and last key only, ignoring both the
        // dimension window and the playback scale.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TravelDistance, 0.25, 0.5, 2.0, samples),
            3.0,
        );
    }

    #[test]
    fn authored_coordinates_are_not_overwritten_by_extraction() {
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [0.0, 1.0, 0.0], [0.0, 3.0, 0.0], [0.0, 6.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        let asset = compile_extraction(
            MotionParameterId::TravelSpeed,
            0.0,
            1.0,
            1.5,
            Some(0.25),
            WIDE_EXTRACTION_BOUNDS,
            samples,
        )
        .expect("blend space compiles from authored coordinates");

        assert_close(
            asset.motions[0]
                .direct_delta_motion
                .get(0)
                .expect("dimension zero is authored for direct delta motion"),
            0.25,
        );
    }

    #[test]
    fn single_key_clips_keep_the_zero_default() {
        // `Init_*` extractors `continue` when `numKeys == 1`, leaving the
        // example at the value `BSParameter` zero-initialized.
        let samples = RootMotionSamples::new(vec![Vec3::ZERO], vec![Quat::IDENTITY], 2.0);

        assert_close(
            extracted_coordinate(MotionParameterId::TravelSpeed, 0.0, 1.0, 1.0, samples),
            0.0,
        );
    }

    #[test]
    fn parameters_without_a_cryengine_extractor_keep_the_zero_default() {
        // `ParameterExtraction` never dispatches for `DesiredFacing`, so the
        // example keeps its zero-initialized `m_Para`.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]),
            vec![Quat::IDENTITY; 4],
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::DesiredFacing, 0.0, 1.0, 1.0, samples),
            0.0,
        );
    }

    #[test]
    fn missing_root_channels_keep_the_zero_default() {
        // Turn speed needs rotations; a clip with only root translation cannot
        // supply it, so CryEngine's accumulator stays at zero.
        let samples = RootMotionSamples::new(
            sample_positions([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]),
            Vec::new(),
            2.0,
        );

        assert_close(
            extracted_coordinate(MotionParameterId::TurnSpeed, 0.0, 1.0, 1.0, samples),
            0.0,
        );
    }

    #[test]
    fn clips_without_a_root_controller_keep_the_zero_default() {
        // `root_motion_samples` yields nothing for an additive or upper-body
        // layer with `azothRootControllerId: null`, and for an unreadable or
        // missing animation product. CryEngine's
        // `if (pController == 0) { continue; }` treats both the same way.
        let source = extraction_source(
            MotionParameterId::TravelSpeed,
            0.0,
            1.0,
            1.0,
            None,
            WIDE_EXTRACTION_BOUNDS,
        );
        let asset = BlendSpaceCompiler
            .blend_space(&source, &mut TestResolver::default())
            .expect("a clip without root motion still compiles");

        assert_close(
            asset.motions[0]
                .direct_delta_motion
                .get(0)
                .expect("dimension zero is authored for direct delta motion"),
            0.0,
        );
    }

    #[test]
    fn source_ron_preserves_timewarp_groups_and_combined_authoring_bounds() {
        let source = CombinedBlendSpaceSource {
            source_path: "animations/locomotion.comb".to_owned(),
            combined_blend_space: CombinedBlendSpace {
                idle_to_move: true,
                dimensions: vec![CombinedBlendSpaceDimension {
                    name: "TravelSpeed".to_owned(),
                    parameter_id: Some(u8::from(MotionParameterId::TravelSpeed)),
                    unresolved_parameter_reason: None,
                    min: Some(-2.0),
                    max: Some(3.0),
                    locked: false,
                    parameter_scale: 0.5,
                    choose_blend_space: true,
                }],
                timewarp_groups: vec!["Locomotion".to_owned()],
                additional_extraction: Vec::new(),
                blend_spaces: vec![BlendSpaceReference {
                    path: "animations/walk.bspace".to_owned(),
                    authoring_path: Some("animations/walk.bspace.ron".to_owned()),
                    unresolved_reference_reason: None,
                }],
                motion_combinations: Vec::new(),
                joints: Vec::new(),
            },
        };

        let decoded = CombinedBlendSpaceSource::from_ron_bytes(&source.to_ron_bytes().unwrap())
            .expect("canonical combined blend-space source RON");

        assert_eq!(decoded, source);
    }

    #[test]
    fn compiler_uses_the_last_authored_timewarp_group_and_ignores_retired_motion_combinations() {
        let mut source = child_source(
            "animations/walk.bspace",
            "walk",
            "animations/walk.anim.glb",
            0.0,
            1.0,
        );
        source
            .blend_space
            .motion_combinations
            .push(BlendSpaceMotionCombination {
                animation: BlendSpaceAnimationRef {
                    name: "retired".to_owned(),
                    motion_path: None,
                    unresolved_motion_reason: Some("retired authoring node".to_owned()),
                },
            });

        let asset = BlendSpaceCompiler
            .blend_space(&source, &mut TestResolver::default())
            .expect("valid blend space");

        assert_eq!(asset.timewarp_group.as_deref(), Some("LocomotionUpper"));
    }

    #[test]
    fn compiler_preserves_an_animation_set_alias_without_inventing_a_product() {
        let mut source = child_source(
            "animations/walk.bspace",
            "character_set_walk",
            "animations/walk.anim.glb",
            0.0,
            1.0,
        );
        source.blend_space.examples[0].animation.motion_path = None;
        source.blend_space.examples[0]
            .animation
            .unresolved_motion_reason = Some(
            "animation reference could not be resolved without a character animation set"
                .to_owned(),
        );

        let asset = BlendSpaceCompiler
            .blend_space(&source, &mut TestResolver::default())
            .expect("alias-only example remains a valid character-link input");

        assert_eq!(
            asset.motions[0].animation.alias.as_ref(),
            "character_set_walk"
        );
        assert!(asset.motions[0].animation.product.is_none());
        assert_eq!(asset.referenced_asset_ids().count(), 0);
    }

    #[test]
    fn combined_compiler_flattens_child_motions_to_asset_ids() {
        let left = child_source(
            "animations/left.bspace",
            "left",
            "animations/left.anim.glb",
            -1.0,
            0.0,
        );
        let right = child_source(
            "animations/right.bspace",
            "right",
            "animations/right.anim.glb",
            0.0,
            1.0,
        );
        let mut resolver = TestResolver {
            sources: vec![
                ("animations/left.bspace.ron".to_owned(), left),
                ("animations/right.bspace.ron".to_owned(), right),
            ],
            samples: Vec::new(),
        };
        let source = CombinedBlendSpaceSource {
            source_path: "animations/locomotion.comb".to_owned(),
            combined_blend_space: CombinedBlendSpace {
                idle_to_move: false,
                dimensions: vec![CombinedBlendSpaceDimension {
                    name: "TravelSpeed".to_owned(),
                    parameter_id: Some(u8::from(MotionParameterId::TravelSpeed)),
                    unresolved_parameter_reason: None,
                    min: Some(-1.0),
                    max: Some(1.0),
                    locked: false,
                    parameter_scale: 1.0,
                    choose_blend_space: true,
                }],
                timewarp_groups: Vec::new(),
                additional_extraction: Vec::new(),
                blend_spaces: vec![
                    BlendSpaceReference {
                        path: "animations/left.bspace".to_owned(),
                        authoring_path: Some("animations/left.bspace.ron".to_owned()),
                        unresolved_reference_reason: None,
                    },
                    BlendSpaceReference {
                        path: "animations/right.bspace".to_owned(),
                        authoring_path: Some("animations/right.bspace.ron".to_owned()),
                        unresolved_reference_reason: None,
                    },
                ],
                motion_combinations: vec![BlendSpaceMotionCombination {
                    animation: BlendSpaceAnimationRef {
                        name: "retired".to_owned(),
                        motion_path: None,
                        unresolved_motion_reason: Some("retired authoring node".to_owned()),
                    },
                }],
                joints: Vec::new(),
            },
        };

        let asset = BlendSpaceCompiler
            .combined_blend_space(&source, &mut resolver)
            .expect("valid combined blend space");

        assert_eq!(asset.motions.len(), 2);
        assert_eq!(asset.sampler.example_count(), 2);
        assert_eq!(
            asset.referenced_asset_ids().collect::<BTreeSet<_>>(),
            [
                source_asset_id("animations/left.anim.glb", PRIMARY_PRODUCT_SUB_ID),
                source_asset_id("animations/right.anim.glb", PRIMARY_PRODUCT_SUB_ID),
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
    }
}
