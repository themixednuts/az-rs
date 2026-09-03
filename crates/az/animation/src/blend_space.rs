//! Fixed-capacity parametric animation sampling.

use arrayvec::ArrayVec;
use bevy_reflect::Reflect;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use std::ops::Index;

pub use crate::motion::{MotionParameterId, UnknownMotionParameterId};

pub const MOTION_PARAMETER_COUNT: usize = 18;
pub const MAX_BLEND_SPACE_DIMENSIONS: usize = 3;
pub const MAX_COMBINED_BLEND_SPACE_DIMENSIONS: usize = 4;
pub const MAX_COMBINED_BLEND_SPACES: usize = 32;
pub const MAX_BLEND_SPACE_MOTIONS: usize = 40;
pub const MAX_VIRTUAL_EXAMPLE_CONTRIBUTORS: usize = 8;
pub const MAX_BLEND_SPACE_EXTRACTION_PARAMETERS: usize = 4;
pub const MAX_MOTION_SEGMENTS: usize = 4;
/// `BSBlendable` stores eight indices (`GlobalAnimationHeaderLMG.h:99`).
pub const MAX_BLEND_SPACE_FACE_POINTS: usize = 8;

const WEIGHT_EPSILON: f32 = 0.0001;
const GRID_EDGE_EPSILON: f32 = 0.001;
const VIRTUAL_WEIGHT_TOLERANCE: f32 = 0.005;
const TRAVEL_DISTANCE_EPSILON: f32 = 0.001;
/// Below this, a 3D annotation's vertices all sit on the `z == 0` plane and
/// `GetConvex8` refuses to build a volume from them
/// (`ParametricSampler.cpp:2014`, `:2096`, `:2189`).
const DEGENERATE_VOLUME_EPSILON: f32 = 0.01;
/// Upper bound of the widening barycentric tolerance in `GetWeights2D`
/// (`ParametricSampler.cpp:1522`).
const HULL_TOLERANCE_LIMIT: f32 = 2.35;
/// Step by which that tolerance widens (`ParametricSampler.cpp:1522`).
const HULL_TOLERANCE_STEP: f32 = 0.05;

/// Per-character motion controls with explicit initialization state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionParameters {
    values: [f32; MOTION_PARAMETER_COUNT],
    desired_values: [f32; MOTION_PARAMETER_COUNT],
    previous_desired_values: [f32; MOTION_PARAMETER_COUNT],
    initialized: u32,
}

impl Default for MotionParameters {
    fn default() -> Self {
        Self {
            values: [0.0; MOTION_PARAMETER_COUNT],
            desired_values: [0.0; MOTION_PARAMETER_COUNT],
            previous_desired_values: [0.0; MOTION_PARAMETER_COUNT],
            initialized: 0,
        }
    }
}

impl MotionParameters {
    pub const fn set(&mut self, parameter: MotionParameterId, value: f32) -> &mut Self {
        let index = parameter.index();
        self.values[index] = value;
        self.desired_values[index] = value;
        self.previous_desired_values[index] = value;
        self.initialized |= 1 << index;
        self
    }

    /// Record one desired-parameter request and optionally accept its value.
    ///
    /// The request history advances even when a locked or blending-out
    /// dimension rejects the new value.
    pub const fn record_desired(
        &mut self,
        parameter: MotionParameterId,
        value: f32,
        accept: bool,
    ) -> bool {
        let index = parameter.index();
        self.previous_desired_values[index] = self.desired_values[index];
        if accept {
            self.values[index] = value;
            self.desired_values[index] = value;
            self.initialized |= 1 << index;
        }
        accept
    }

    pub const fn clear(&mut self, parameter: MotionParameterId) -> &mut Self {
        let index = parameter.index();
        self.values[index] = 0.0;
        self.desired_values[index] = 0.0;
        self.previous_desired_values[index] = 0.0;
        self.initialized &= !(1 << index);
        self
    }

    #[must_use]
    pub const fn is_initialized(&self, parameter: MotionParameterId) -> bool {
        self.initialized & (1 << parameter.index()) != 0
    }

    #[must_use]
    pub fn get(&self, parameter: MotionParameterId) -> Option<f32> {
        self.is_initialized(parameter)
            .then(|| self.values[parameter.index()])
    }

    #[must_use]
    pub fn get_or_default(&self, parameter: MotionParameterId) -> f32 {
        self.get(parameter).unwrap_or_default()
    }

    #[must_use]
    pub fn desired(&self, parameter: MotionParameterId) -> Option<f32> {
        self.is_initialized(parameter)
            .then(|| self.desired_values[parameter.index()])
    }

    #[must_use]
    pub fn previous_desired(&self, parameter: MotionParameterId) -> Option<f32> {
        self.is_initialized(parameter)
            .then(|| self.previous_desired_values[parameter.index()])
    }
}

impl Index<MotionParameterId> for MotionParameters {
    type Output = f32;

    fn index(&self, index: MotionParameterId) -> &Self::Output {
        &self.values[index.index()]
    }
}

/// Authored constant delta-motion coordinates for one blend-space example.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct DirectDeltaMotion {
    mask: u8,
    values: [f32; MAX_COMBINED_BLEND_SPACE_DIMENSIONS],
}

impl DirectDeltaMotion {
    #[must_use]
    pub fn from_dimensions(dimensions: impl IntoIterator<Item = (bool, f32)>) -> Self {
        let mut value = Self::default();
        for (index, (direct, coordinate)) in dimensions
            .into_iter()
            .take(MAX_COMBINED_BLEND_SPACE_DIMENSIONS)
            .enumerate()
        {
            value.mask |= u8::from(direct) << index;
            value.values[index] = coordinate;
        }
        value
    }

    #[must_use]
    pub const fn get(self, dimension: usize) -> Option<f32> {
        if dimension < MAX_COMBINED_BLEND_SPACE_DIMENSIONS && self.mask & (1 << dimension) != 0 {
            Some(self.values[dimension])
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlendSpaceDimension {
    pub parameter: MotionParameterId,
    pub min: f32,
    pub max: f32,
    pub cells: u8,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualExample {
    pub indices: ArrayVec<u8, MAX_VIRTUAL_EXAMPLE_CONTRIBUTORS>,
    pub weights: ArrayVec<f32, MAX_VIRTUAL_EXAMPLE_CONTRIBUTORS>,
}

/// Motion-parameter coordinates of one blend-space example, indexed by
/// dimension.
///
/// Mirrors `BSParameter::m_Para` (`GlobalAnimationHeaderLMG.h:59`), which
/// `CryEngine` fills from `SetPara<N>` or from root-motion extraction.
pub type ExampleParameters = [f32; MAX_BLEND_SPACE_DIMENSIONS];

/// One blend-space annotation: the example indices spanning a line (1D),
/// triangle or quad (2D), or cell (3D). Mirrors `BSBlendable`
/// (`GlobalAnimationHeaderLMG.h:96`), whose `num` is the used index count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlendSpaceFace {
    pub indices: ArrayVec<u8, MAX_BLEND_SPACE_FACE_POINTS>,
}

/// A point in parameter space defined as a linear blend of two real examples.
///
/// Mirrors the `i0/w0/i1/w1` fields of `BSParameter`, which `CryEngine` appends
/// after the real examples in `m_arrParameter`
/// (`GlobalAnimationHeaderLMG.cpp:700-703`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PseudoExample {
    pub i0: u8,
    pub w0: f32,
    pub i1: u8,
    pub w1: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParametricBlendSpaceDescription {
    pub dimensions: Vec<BlendSpaceDimension>,
    pub additional_extraction: Vec<MotionParameterId>,
    pub example_count: u8,
    /// Per-example motion-parameter coordinates, in example order. These are
    /// the real input to the virtual grid; see [`ParametricBlendSpace`].
    #[serde(default)]
    pub example_parameters: Vec<ExampleParameters>,
    #[serde(default)]
    pub pseudo_examples: Vec<PseudoExample>,
    #[serde(default)]
    pub faces: Vec<BlendSpaceFace>,
    pub virtual_examples: Vec<VirtualExample>,
    pub threshold: Option<f32>,
    pub idle_to_move: bool,
}

/// A validated virtual-example grid. Dimension zero is the fastest-changing
/// grid coordinate, matching the animation sampler's packed layout.
///
/// `CryEngine` treats the authored `<VGrid>` as a *cache* of a computation over
/// the per-example motion parameters, not as authority: `ReadVGrid` fills the
/// runtime grid only when the node's child count equals the product of the
/// dimension cell counts and otherwise leaves it empty
/// (`GlobalAnimationHeaderLMG.cpp:126-130`), and the sampler rebuilds any grid
/// that is still empty (`ParametricSampler.cpp:443-446` for 1D, `:729-732` for
/// 2D). [`ParametricBlendSpace::try_from`] reproduces exactly that rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ParametricBlendSpace {
    dimensions: ArrayVec<BlendSpaceDimension, MAX_BLEND_SPACE_DIMENSIONS>,
    additional_extraction: ArrayVec<MotionParameterId, MAX_BLEND_SPACE_EXTRACTION_PARAMETERS>,
    example_count: u8,
    example_parameters: Box<[ExampleParameters]>,
    pseudo_examples: Box<[PseudoExample]>,
    faces: Box<[BlendSpaceFace]>,
    virtual_examples: Box<[VirtualExample]>,
    threshold: Option<f32>,
    idle_to_move: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CombinedBlendSpaceDimension {
    pub parameter: MotionParameterId,
    pub parameter_scale: f32,
    pub choose_blend_space: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombinedSubSpace {
    pub blend_space: ParametricBlendSpace,
    pub example_indices: ArrayVec<u8, MAX_BLEND_SPACE_MOTIONS>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombinedBlendSpaceDescription {
    pub dimensions: Vec<CombinedBlendSpaceDimension>,
    pub additional_extraction: Vec<MotionParameterId>,
    pub example_count: u8,
    pub blend_spaces: Vec<CombinedSubSpace>,
    pub idle_to_move: bool,
}

/// A combined parametric sampler with its child grids flattened into the
/// cooked product and local examples mapped into one fixed motion table.
#[derive(Debug, Clone, PartialEq)]
pub struct CombinedBlendSpace {
    dimensions: ArrayVec<CombinedBlendSpaceDimension, MAX_COMBINED_BLEND_SPACE_DIMENSIONS>,
    additional_extraction: ArrayVec<MotionParameterId, MAX_BLEND_SPACE_EXTRACTION_PARAMETERS>,
    example_count: u8,
    blend_spaces: Box<[CombinedSubSpace]>,
    idle_to_move: bool,
}

impl CombinedBlendSpace {
    #[must_use]
    pub fn dimensions(&self) -> &[CombinedBlendSpaceDimension] {
        &self.dimensions
    }

    #[must_use]
    pub fn additional_extraction(&self) -> &[MotionParameterId] {
        &self.additional_extraction
    }

    #[must_use]
    pub const fn example_count(&self) -> usize {
        self.example_count as usize
    }

    #[must_use]
    pub const fn is_idle_to_move(&self) -> bool {
        self.idle_to_move
    }

    /// Evaluates the combined sampler for `parameters`, writing the per-example
    /// weights into `output`.
    ///
    /// # Panics
    ///
    /// Panics when a child blend space declares a motion parameter that no
    /// master dimension binds. `try_from` rejects that description, so a
    /// validated sampler cannot reach it.
    pub fn evaluate(&self, parameters: &MotionParameters, output: &mut BlendWeights) {
        output.clear(self.example_count());
        let mut viable = u32::MAX;

        for dimension in self
            .dimensions
            .iter()
            .filter(|dimension| dimension.choose_blend_space)
        {
            let desired = parameters.get_or_default(dimension.parameter);
            let mut closest = 0u32;
            let mut best_distance = f32::MAX;
            for (index, blend_space) in self.blend_spaces.iter().enumerate() {
                let Some(child_dimension) = blend_space
                    .blend_space
                    .dimensions()
                    .iter()
                    .find(|child| child.parameter == dimension.parameter)
                else {
                    continue;
                };
                let mut distance_from_min = child_dimension.min - desired;
                let mut distance_from_max = desired - child_dimension.max;
                if dimension.parameter.is_cyclic() {
                    distance_from_min = wrap_pi(distance_from_min);
                    distance_from_max = wrap_pi(distance_from_max);
                }
                let distance = distance_from_min.max(distance_from_max);
                if distance < best_distance {
                    best_distance = distance;
                    closest = 1 << index;
                }
            }
            viable &= closest;
        }

        let mut local_weights = BlendWeights::default();
        for (sub_space_index, sub_space) in self.blend_spaces.iter().enumerate() {
            if viable & (1 << sub_space_index) == 0 {
                continue;
            }
            let mut local_parameters = MotionParameters::default();
            for child_dimension in sub_space.blend_space.dimensions() {
                let master_dimension = self
                    .dimensions
                    .iter()
                    .find(|master| master.parameter == child_dimension.parameter)
                    .expect("combined blend-space validation binds every child parameter");
                local_parameters.set(
                    child_dimension.parameter,
                    parameters.get_or_default(child_dimension.parameter)
                        * master_dimension.parameter_scale,
                );
            }
            sub_space
                .blend_space
                .evaluate(&local_parameters, &mut local_weights);
            for (local_index, weight) in local_weights.active() {
                let master_index = usize::from(sub_space.example_indices[local_index]);
                output.weights[master_index] += weight;
            }
        }

        output.normalize();
    }
}

impl TryFrom<CombinedBlendSpaceDescription> for CombinedBlendSpace {
    type Error = InvalidCombinedBlendSpace;

    fn try_from(value: CombinedBlendSpaceDescription) -> Result<Self, Self::Error> {
        if !(1..=MAX_COMBINED_BLEND_SPACE_DIMENSIONS).contains(&value.dimensions.len()) {
            return Err(InvalidCombinedBlendSpace::DimensionCount(
                value.dimensions.len(),
            ));
        }
        if value.example_count == 0 || usize::from(value.example_count) > MAX_BLEND_SPACE_MOTIONS {
            return Err(InvalidCombinedBlendSpace::ExampleCount(value.example_count));
        }
        if value.additional_extraction.len() > MAX_BLEND_SPACE_EXTRACTION_PARAMETERS {
            return Err(InvalidCombinedBlendSpace::ExtractionParameterCount(
                value.additional_extraction.len(),
            ));
        }
        if value.blend_spaces.is_empty() || value.blend_spaces.len() > MAX_COMBINED_BLEND_SPACES {
            return Err(InvalidCombinedBlendSpace::BlendSpaceCount(
                value.blend_spaces.len(),
            ));
        }
        for (index, dimension) in value.dimensions.iter().enumerate() {
            if !dimension.parameter_scale.is_finite() {
                return Err(InvalidCombinedBlendSpace::ParameterScale(index));
            }
            if value.dimensions[..index]
                .iter()
                .any(|existing| existing.parameter == dimension.parameter)
            {
                return Err(InvalidCombinedBlendSpace::DuplicateParameter(
                    dimension.parameter,
                ));
            }
            if dimension.choose_blend_space
                && !value.blend_spaces.iter().any(|blend_space| {
                    blend_space
                        .blend_space
                        .dimensions()
                        .iter()
                        .any(|child| child.parameter == dimension.parameter)
                })
            {
                return Err(InvalidCombinedBlendSpace::UnboundSelectionParameter(
                    dimension.parameter,
                ));
            }
        }
        for (space_index, blend_space) in value.blend_spaces.iter().enumerate() {
            if blend_space.example_indices.len() != blend_space.blend_space.example_count() {
                return Err(InvalidCombinedBlendSpace::ExampleMapSize(space_index));
            }
            for child_dimension in blend_space.blend_space.dimensions() {
                if !value
                    .dimensions
                    .iter()
                    .any(|master| master.parameter == child_dimension.parameter)
                {
                    return Err(InvalidCombinedBlendSpace::UnboundChildParameter {
                        space_index,
                        parameter: child_dimension.parameter,
                    });
                }
            }
            for &example_index in &blend_space.example_indices {
                if example_index >= value.example_count {
                    return Err(InvalidCombinedBlendSpace::InvalidExampleMap {
                        space_index,
                        example_index,
                    });
                }
            }
        }

        Ok(Self {
            dimensions: value
                .dimensions
                .into_iter()
                .collect::<ArrayVec<_, MAX_COMBINED_BLEND_SPACE_DIMENSIONS>>(),
            additional_extraction: value.additional_extraction.into_iter().collect(),
            example_count: value.example_count,
            blend_spaces: value.blend_spaces.into_boxed_slice(),
            idle_to_move: value.idle_to_move,
        })
    }
}

impl Serialize for CombinedBlendSpace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        CombinedBlendSpaceDescription {
            dimensions: self.dimensions.iter().copied().collect(),
            additional_extraction: self.additional_extraction.iter().copied().collect(),
            example_count: self.example_count,
            blend_spaces: self.blend_spaces.to_vec(),
            idle_to_move: self.idle_to_move,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CombinedBlendSpace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(CombinedBlendSpaceDescription::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCombinedBlendSpace {
    #[error(
        "combined blend space has {0} dimensions; expected 1..={MAX_COMBINED_BLEND_SPACE_DIMENSIONS}"
    )]
    DimensionCount(usize),
    #[error("combined blend space has {0} examples; expected 1..={MAX_BLEND_SPACE_MOTIONS}")]
    ExampleCount(u8),
    #[error("combined blend space has {0} child spaces; expected 1..={MAX_COMBINED_BLEND_SPACES}")]
    BlendSpaceCount(usize),
    #[error(
        "combined blend space has {0} extraction parameters; expected at most {MAX_BLEND_SPACE_EXTRACTION_PARAMETERS}"
    )]
    ExtractionParameterCount(usize),
    #[error("combined blend-space dimension {0} has a non-finite parameter scale")]
    ParameterScale(usize),
    #[error("combined blend space declares parameter {0:?} more than once")]
    DuplicateParameter(MotionParameterId),
    #[error("selection parameter {0:?} is absent from every child blend space")]
    UnboundSelectionParameter(MotionParameterId),
    #[error("child blend space {0} has an invalid example map size")]
    ExampleMapSize(usize),
    #[error("child blend space {space_index} uses undeclared parameter {parameter:?}")]
    UnboundChildParameter {
        space_index: usize,
        parameter: MotionParameterId,
    },
    #[error("child blend space {space_index} maps to invalid example {example_index}")]
    InvalidExampleMap {
        space_index: usize,
        example_index: u8,
    },
}

fn wrap_pi(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

impl ParametricBlendSpace {
    #[must_use]
    pub fn dimensions(&self) -> &[BlendSpaceDimension] {
        &self.dimensions
    }

    #[must_use]
    pub fn additional_extraction(&self) -> &[MotionParameterId] {
        &self.additional_extraction
    }

    #[must_use]
    pub const fn example_count(&self) -> usize {
        self.example_count as usize
    }

    #[must_use]
    pub const fn threshold(&self) -> Option<f32> {
        self.threshold
    }

    #[must_use]
    pub const fn is_idle_to_move(&self) -> bool {
        self.idle_to_move
    }

    /// Per-example motion-parameter coordinates, in example order. These are
    /// the real input the virtual grid is derived from.
    #[must_use]
    pub const fn example_parameters(&self) -> &[ExampleParameters] {
        &self.example_parameters
    }

    /// Blend-space annotations: the faces spanning the example cloud.
    #[must_use]
    pub const fn faces(&self) -> &[BlendSpaceFace] {
        &self.faces
    }

    #[must_use]
    pub const fn pseudo_examples(&self) -> &[PseudoExample] {
        &self.pseudo_examples
    }

    /// The virtual-example grid, in packed grid order.
    #[must_use]
    pub const fn virtual_examples(&self) -> &[VirtualExample] {
        &self.virtual_examples
    }

    /// Evaluates the parametric sampler for `parameters`, writing the
    /// per-example weights into `output`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the grid coordinate is clamped to `0.0..=cells - 1 - GRID_EDGE_EPSILON` before \
                  the truncating cast, and the resulting cell index is bounded by the u8 cell \
                  count, so both directions round exactly as CryEngine's do"
    )]
    #[expect(
        clippy::suboptimal_flops,
        reason = "the corner weights accumulate as `weight * corner_weight` added to the slot, \
                  exactly as `ParametricSampler` does; fusing the multiply into the add changes \
                  the sampled blend weights"
    )]
    pub fn evaluate(&self, parameters: &MotionParameters, output: &mut BlendWeights) {
        output.clear(self.example_count());

        let mut grid_cells = [0usize; MAX_BLEND_SPACE_DIMENSIONS];
        let mut fractions = [0.0f32; MAX_BLEND_SPACE_DIMENSIONS];
        let mut desired = [0.0f32; MAX_BLEND_SPACE_DIMENSIONS];
        for (index, dimension) in self.dimensions.iter().enumerate() {
            desired[index] = parameters
                .get_or_default(dimension.parameter)
                .clamp(dimension.min, dimension.max);
        }

        if self.dimensions.len() == 3
            && let Some(threshold) = self.threshold
        {
            let secondary = desired[1].abs();
            desired[2] = if secondary > threshold {
                0.0
            } else {
                desired[2] * (1.0 - secondary / threshold)
            };
        }

        for (index, dimension) in self.dimensions.iter().enumerate() {
            let cells_minus_one = f32::from(dimension.cells - 1);
            let coordinate = ((desired[index] - dimension.min)
                / ((dimension.max - dimension.min) / cells_minus_one))
                .clamp(0.0, cells_minus_one - GRID_EDGE_EPSILON);
            grid_cells[index] = coordinate as usize;
            fractions[index] = coordinate - grid_cells[index] as f32;
        }

        let corner_count = 1usize << self.dimensions.len();
        for corner in 0..corner_count {
            let mut grid_index = 0usize;
            let mut stride = 1usize;
            let mut corner_weight = 1.0f32;
            for dimension_index in 0..self.dimensions.len() {
                let upper = (corner & (1 << dimension_index)) != 0;
                let coordinate = grid_cells[dimension_index] + usize::from(upper);
                grid_index += coordinate * stride;
                stride *= usize::from(self.dimensions[dimension_index].cells);
                corner_weight *= if upper {
                    fractions[dimension_index]
                } else {
                    1.0 - fractions[dimension_index]
                };
            }

            let virtual_example = &self.virtual_examples[grid_index];
            for (&example, &weight) in virtual_example.indices.iter().zip(&virtual_example.weights)
            {
                output.weights[usize::from(example)] += weight * corner_weight;
            }
        }

        output.normalize();
    }
}

// ---------------------------------------------------------------------------
// Virtual-example grid construction.
//
// Port of CryEngine's lazy grid builder. The authored `<VGrid>` is a cache of
// this computation; see [`ParametricBlendSpace`] for the cache rule.
// ---------------------------------------------------------------------------

/// One entry of `CryEngine`'s `m_arrParameter`: a point in motion-parameter
/// space plus its decomposition into at most two real examples.
#[derive(Debug, Clone, Copy)]
struct ParameterPoint {
    parameters: ExampleParameters,
    i0: u8,
    w0: f32,
    i1: u8,
    w1: f32,
}

/// Builds `m_arrParameter`: the real examples followed by the pseudo examples.
#[expect(
    clippy::suboptimal_flops,
    reason = "a pseudo example's position is `first * w0 + second * w1`, the exact form \
              `ParametricSampler.cpp:429-438` uses; fusing it moves the interpolated point"
)]
fn parameter_points(
    example_parameters: &[ExampleParameters],
    pseudo_examples: &[PseudoExample],
) -> Vec<ParameterPoint> {
    let mut points: Vec<ParameterPoint> = example_parameters
        .iter()
        .enumerate()
        .map(|(index, parameters)| ParameterPoint {
            parameters: *parameters,
            // "real examples always have a weight of 1.0f"
            // (`GlobalAnimationHeaderLMG.cpp:617-620`).
            i0: u8::try_from(index).unwrap_or(0),
            w0: 1.0,
            i1: 0,
            w1: 0.0,
        })
        .collect();
    for pseudo in pseudo_examples {
        // A pseudo example's position is interpolated from its two parents
        // before every grid build (`ParametricSampler.cpp:429-438`, `:714-724`).
        let parent = |index: u8| {
            points
                .get(usize::from(index))
                .map_or([0.0; MAX_BLEND_SPACE_DIMENSIONS], |point| point.parameters)
        };
        let (first, second) = (parent(pseudo.i0), parent(pseudo.i1));
        let mut parameters = [0.0f32; MAX_BLEND_SPACE_DIMENSIONS];
        for (axis, value) in parameters.iter_mut().enumerate() {
            *value = first[axis] * pseudo.w0 + second[axis] * pseudo.w1;
        }
        points.push(ParameterPoint {
            parameters,
            i0: pseudo.i0,
            w0: pseudo.w0,
            i1: pseudo.i1,
            w1: pseudo.w1,
        });
    }
    points
}

/// Adds `blend` of parameter point `point_index` to the example weights,
/// following the point's decomposition exactly as
/// `ParametricSampler.cpp:1375-1383` does.
#[expect(
    clippy::suboptimal_flops,
    reason = "`ParametricSampler.cpp:1375-1383` accumulates `w * blend` into the slot as a \
              separate multiply and add; fusing it changes every packed grid weight"
)]
fn accumulate_point(
    points: &[ParameterPoint],
    point_index: usize,
    blend: f32,
    weights: &mut [f32],
) {
    let Some(point) = points.get(point_index) else {
        return;
    };
    if let Some(slot) = weights.get_mut(usize::from(point.i0)) {
        *slot += point.w0 * blend;
    }
    if let Some(slot) = weights.get_mut(usize::from(point.i1)) {
        *slot += point.w1 * blend;
    }
}

/// The two endpoints of a 1D annotation line, or `None` when the annotation is
/// not a line `CryEngine` can interpolate along.
///
/// `CryEngine` only considers `num == 2` faces in 1D
/// (`ParametricSampler.cpp:1330`) and then divides by `x1 - x0` on both the
/// bracketing path and the extrapolating one, asserting the divisor first
/// (`CRY_ASSERT(fDistance)`, `ParametricSampler.cpp:1370`, `:1419`). A checked
/// build also refuses the whole space when two endpoints are closer than `0.01`
/// or are not sorted, warning "parameters in 1D-Blend-Space are too close" and
/// "motion parameter must be sorted by size" and returning `-1` with every
/// weight left at zero (`ParametricSampler.cpp:1338-1354`).
///
/// A zero-extent line is therefore not a line: interpolating along it yields
/// `0 / 0`. The line is dropped so invalid data cannot poison the grid with
/// `NaN` values.
fn line_segment(
    face: &BlendSpaceFace,
    points: &[ParameterPoint],
) -> Option<(usize, usize, f32, f32)> {
    if face.indices.len() != 2 {
        return None;
    }
    let i0 = usize::from(*face.indices.first()?);
    let i1 = usize::from(*face.indices.get(1)?);
    let x0 = points.get(i0)?.parameters[0];
    let x1 = points.get(i1)?.parameters[0];
    let distance = x1 - x0;
    if distance == 0.0 || !distance.is_finite() {
        return None;
    }
    Some((i0, i1, x0, x1))
}

/// Which annotation gave one rebuilt grid cell its weights, and how.
///
/// `CryEngine`'s `GetWeights1D/2D/3D` each return the index of the annotation
/// they used, or `-1` (`ParametricSampler.cpp:1400`/`:1504`, `:1557`/`:1562`,
/// `:1614`/`:1618`), and the samplers keep it as `selFace`
/// (`ParametricSampler.cpp:304`, `:578`, `:903`). The grid builder discards the
/// weights' provenance, which makes an out-of-hull or unclaimed cell
/// indistinguishable from a well-behaved one; this records it instead.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GridCellSource {
    /// Annotation `face` accepted the cell coordinate once the hull tolerance
    /// had widened to `tolerance`, so each of its barycentric weights lies in
    /// `[-tolerance, 1 + tolerance]`
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:1540-1547`).
    /// 1D has no tolerance ladder: a line either brackets the coordinate or it
    /// does not, so `tolerance` is zero there.
    Accepted { face: usize, tolerance: f32 },
    /// 1D only: no line bracketed the coordinate, so the sampler extrapolated
    /// onto the least out-of-range line (`ParametricSampler.cpp:1406-1485`).
    /// The two weights still sum to one but are not confined to `[0, 1]`.
    Extrapolated { face: usize },
    /// No annotation accepted the coordinate. In 2D and 3D the weights are
    /// whatever the last annotation evaluated at the widest tolerance left
    /// behind (`ParametricSampler.cpp:1562`, `:1618`); where no annotation has
    /// a usable arity they are all zero.
    Unaccepted,
}

impl GridCellSource {
    /// The annotation whose corners the cell drew on, when one produced the
    /// weights at all. Only the grid invariants need this; the builder itself
    /// reports the variants directly.
    #[cfg(test)]
    const fn face(self) -> Option<usize> {
        match self {
            Self::Accepted { face, .. } | Self::Extrapolated { face } => Some(face),
            Self::Unaccepted => None,
        }
    }
}

/// A virtual-example grid rebuilt from the per-example motion parameters, with
/// the provenance of every cell alongside it.
struct RebuiltGrid {
    cells: Vec<VirtualExample>,
    sources: Vec<GridCellSource>,
}

impl RebuiltGrid {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            cells: Vec::with_capacity(capacity),
            sources: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, cell: VirtualExample, source: GridCellSource) {
        self.cells.push(cell);
        self.sources.push(source);
    }
}

/// Port of `SParametricSamplerInternal::GetWeights1D`
/// (`ParametricSampler.cpp:1315-1505`).
fn weights_1d(
    desired: f32,
    points: &[ParameterPoint],
    faces: &[BlendSpaceFace],
    weights: &mut [f32],
) -> GridCellSource {
    weights.fill(0.0);

    for (index, face) in faces.iter().enumerate() {
        let Some((i0, i1, x0, x1)) = line_segment(face, points) else {
            continue;
        };
        if x0 <= desired && x1 >= desired {
            let distance = x1 - x0;
            let offset = desired - x0;
            accumulate_point(points, i0, 1.0 - offset / distance, weights);
            accumulate_point(points, i1, offset / distance, weights);
            return GridCellSource::Accepted {
                face: index,
                tolerance: 0.0,
            };
        }
    }

    // No line contains the sample, so CryEngine extrapolates onto whichever
    // line is least out of range (`ParametricSampler.cpp:1406-1502`). The four
    // comparisons run in this order and each may claim the best line.
    let mut closest_excess = 9999.0f32;
    let mut closest = None;
    for (index, face) in faces.iter().enumerate() {
        let Some((_, _, x0, x1)) = line_segment(face, points) else {
            continue;
        };
        let distance = x1 - x0;
        let offset = desired - x0;
        let w0 = 1.0 - offset / distance;
        let w1 = offset / distance;
        for excess in [
            (w0 < 0.0).then(|| -w0),
            (w0 > 1.0).then_some(w0 - 1.0),
            (w1 < 0.0).then(|| -w1),
            (w1 > 1.0).then_some(w1 - 1.0),
        ]
        .into_iter()
        .flatten()
        {
            if closest_excess > excess {
                closest_excess = excess;
                closest = Some(index);
            }
        }
    }

    if let Some(index) = closest
        && let Some(face) = faces.get(index)
        && let Some((i0, i1, x0, x1)) = line_segment(face, points)
    {
        let distance = x1 - x0;
        let offset = desired - x0;
        accumulate_point(points, i0, 1.0 - offset / distance, weights);
        accumulate_point(points, i1, offset / distance, weights);
        return GridCellSource::Extrapolated { face: index };
    }
    GridCellSource::Unaccepted
}

/// Port of `SParametricSamplerInternal::ComputeWeightExtrapolate4`
/// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:1624-1658`).
fn weight_extrapolate_4(sample: [f32; 2], corners: [[f32; 2]; 4]) -> [f32; 4] {
    /// Port of the nested `TW3::Weight3`. Returns the unnormalised barycentric
    /// weights of `sample` in triangle `(v0, v1, v2)`, or zeros when `sample`
    /// lies on the far side of edge `v0 -> v1`.
    #[expect(
        clippy::suboptimal_flops,
        reason = "the Lumberyard algorithm uses separate multiplies and adds; fusing them \
                  changes the sign test and therefore the compiled blend-space grid"
    )]
    fn weight_3(sample: [f32; 2], v0: [f32; 2], v1: [f32; 2], v2: [f32; 2]) -> [f32; 3] {
        // `Plane::CreatePlane(v0, v1, Vec3(v0.x, v0.y, 1))` is the vertical
        // plane through edge `v0 -> v1`; the cross product collapses to
        // `(dy, -dx, 0)` and `GetNormalized` scales it by the inverse length.
        // CryEngine's `isqrt_safe_tpl` adds `FLT_MIN` before taking the inverse
        // square root.
        let dx = v1[0] - v0[0];
        let dy = v1[1] - v0[1];
        let inverse_length = 1.0 / (dy * dy + dx * dx + f32::MIN_POSITIVE).sqrt();
        let nx = dy * inverse_length;
        let ny = -dx * inverse_length;
        // `Plane::operator|` is `(n | point) + d` with `d = -(n | v0)`, so the
        // two dot products stay separate.
        let distance = (sample[0] * nx + sample[1] * ny) - (v0[0] * nx + v0[1] * ny);
        // Written as the negation of `<=` so a NaN distance rejects the triangle.
        #[expect(
            clippy::neg_cmp_op_on_partial_ord,
            reason = "the negated form is what rejects a NaN distance; `distance > 0.0` would accept it"
        )]
        if !(distance <= 0.0) {
            return [0.0; 3];
        }
        let e0 = [v0[0] - v2[0], v0[1] - v2[1]];
        let e1 = [v1[0] - v2[0], v1[1] - v2[1]];
        let relative = [sample[0] - v2[0], sample[1] - v2[1]];
        let w0 = relative[0] * e1[1] - e1[0] * relative[1];
        let w1 = e0[0] * relative[1] - relative[0] * e0[1];
        let w2 = e0[0] * e1[1] - e1[0] * e0[1] - w0 - w1;
        [w0, w1, w2]
    }

    // The four calls route each triangle's weights into the quad slot of the
    // matching corner and zero the unused fourth slot, so summing the four
    // results reproduces `Weight4 = w; Weight4 += w; ...` verbatim
    // (`ParametricSampler.cpp:1648-1655`).
    const TRIANGLES: [[usize; 3]; 4] = [[1, 3, 0], [3, 1, 2], [2, 0, 1], [0, 2, 3]];
    let mut total = [0.0f32; 4];
    for triangle in TRIANGLES {
        let weights = weight_3(
            sample,
            corners[triangle[0]],
            corners[triangle[1]],
            corners[triangle[2]],
        );
        for (slot, weight) in triangle.into_iter().zip(weights) {
            total[slot] += weight;
        }
    }
    // Use one reciprocal so every component shares the same normalization factor.
    let sum = total[3] + total[2] + total[0] + total[1];
    let inverse = 1.0 / sum;
    total.map(|value| inverse * value)
}

/// Port of `SParametricSamplerInternal::GetConvex4`
/// (`ParametricSampler.cpp:1661-1824`). Writes the example weights and returns
/// the face-corner barycentric weights used for the inside-hull test.
#[expect(
    clippy::suboptimal_flops,
    reason = "`ParametricSampler.cpp:1714-1722` uses separate multiplies and subtracts; \
              fusing them changes the barycentric weights"
)]
fn convex_4(
    face: &BlendSpaceFace,
    sample: [f32; 2],
    points: &[ParameterPoint],
    weights: &mut [f32],
) -> [f32; 4] {
    weights.fill(0.0);
    // CryEngine seeds every component with 9999, so any face arity it does not
    // handle can never pass the inside-hull test
    // (`ParametricSampler.cpp:1686-1690`).
    let mut barycentric = [9999.0f32; 4];

    let corner = |slot: usize| -> Option<(usize, [f32; 2])> {
        let index = usize::from(*face.indices.get(slot)?);
        let parameters = points.get(index)?.parameters;
        Some((index, [parameters[0], parameters[1]]))
    };

    match face.indices.len() {
        3 => {
            let (Some((_, v0)), Some((_, v1)), Some((_, v2))) = (corner(0), corner(1), corner(2))
            else {
                return barycentric;
            };
            let px = sample[0] - v2[0];
            let py = sample[1] - v2[1];
            let z0 = [v0[0] - v2[0], v0[1] - v2[1]];
            let z1 = [v1[0] - v2[0], v1[1] - v2[1]];
            let u = px * z1[1] - py * z1[0];
            let v = py * z0[0] - px * z0[1];
            let determinant = z0[0] * z1[1] - z1[0] * z0[1];
            let w = determinant - u - v;
            // A degenerate triangle keeps the unnormalised weights rather than
            // dividing by a near-zero determinant
            // (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:1722`).
            if determinant.abs() > f32::EPSILON {
                let inverse = 1.0 / determinant;
                barycentric = [inverse * u, inverse * v, inverse * w, 0.0];
            } else {
                barycentric = [u, v, w, 0.0];
            }
        }
        4 => {
            let (Some((_, v0)), Some((_, v1)), Some((_, v2)), Some((_, v3))) =
                (corner(0), corner(1), corner(2), corner(3))
            else {
                return barycentric;
            };
            barycentric = weight_extrapolate_4(sample, [v0, v1, v2, v3]);
        }
        _ => return barycentric,
    }

    #[expect(
        clippy::needless_range_loop,
        reason = "the index selects the face corner as well as the weight, so this loops over corner slots rather than over the weight slice"
    )]
    for slot in 0..face.indices.len().min(barycentric.len()) {
        if let Some((index, _)) = corner(slot) {
            accumulate_point(points, index, barycentric[slot], weights);
        }
    }
    barycentric
}

/// Port of `SParametricSamplerInternal::GetWeights2D`
/// (`ParametricSampler.cpp:1509-1563`).
#[expect(
    clippy::while_float,
    reason = "CryEngine widens the hull tolerance by accumulating `HULL_TOLERANCE_STEP` in \
              `f32` and loops on that float comparison; an integer loop counter would visit a \
              different set of tolerances and accept different faces"
)]
fn weights_2d(
    sample: [f32; 2],
    points: &[ParameterPoint],
    faces: &[BlendSpaceFace],
    weights: &mut [f32],
) -> GridCellSource {
    weights.fill(0.0);
    // CryEngine widens the accepted barycentric range in 0.05 steps until some
    // face accepts the sample, accumulating the tolerance in `f32`.
    let mut tolerance = 0.0f32;
    while tolerance < HULL_TOLERANCE_LIMIT {
        for (index, face) in faces.iter().enumerate() {
            let barycentric = convex_4(face, sample, points, weights);
            // Use a positive range check so a NaN weight fails both comparisons
            // and the face is rejected
            // (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:1540-1547`).
            let inside = (0..face.indices.len().min(barycentric.len())).all(|slot| {
                -tolerance <= barycentric[slot] && barycentric[slot] <= 1.0 + tolerance
            });
            if inside {
                return GridCellSource::Accepted {
                    face: index,
                    tolerance,
                };
            }
        }
        tolerance += HULL_TOLERANCE_STEP;
    }
    // CryEngine returns -1 here and leaves the weights holding whatever the
    // last evaluated face produced (`ParametricSampler.cpp:1562`).
    GridCellSource::Unaccepted
}

// ---------------------------------------------------------------------------
// 3D convex decomposition. CryEngine splits every 3D annotation into
// tetrahedra: a 4-point face is one, a 5-point face is a pyramid built from
// four of them, and a 6-point face is a wedge built from a pyramid plus a
// tetrahedron (`ParametricSampler.cpp:1830-1950`).
// ---------------------------------------------------------------------------

fn subtract_3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `Vec3::operator%`, `CryEngine`'s cross product (`Cry_Vector3.h:846`).
#[expect(
    clippy::suboptimal_flops,
    reason = "bit-exact port of `Vec3::operator%` (`Cry_Vector3.h:846`); each component is a \
              multiply and a subtract, and fusing them changes every plane normal derived \
              from this cross product"
)]
fn cross_3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vec3::operator|`, `CryEngine`'s dot product (`Cry_Vector3.h:863`).
#[expect(
    clippy::suboptimal_flops,
    reason = "bit-exact port of `Vec3::operator|` (`Cry_Vector3.h:863`); the three products \
              are summed left to right with separate adds, and fusing them changes every \
              plane distance and tetrahedron weight"
)]
fn dot_3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Signed distance from `point` to `Plane::CreatePlane(v0, v1, v2)`
/// (`Cry_Vector3.h:1356-1366`, `:1378`).
#[expect(
    clippy::suboptimal_flops,
    reason = "the squared length is summed component by component before `FLT_MIN` is added, \
              mirroring Lumberyard's `isqrt_safe_tpl`; fusing the operations changes the normal"
)]
fn plane_distance(v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], point: [f32; 3]) -> f32 {
    let normal = cross_3(subtract_3(v1, v0), subtract_3(v2, v0));
    // Same `isqrt_safe_tpl` shape as the 2D edge plane: `FLT_MIN` is added to
    // the squared length before the inverse square root.
    let length_squared =
        normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2] + f32::MIN_POSITIVE;
    let inverse_length = 1.0 / length_squared.sqrt();
    let normal = [
        normal[0] * inverse_length,
        normal[1] * inverse_length,
        normal[2] * inverse_length,
    ];
    dot_3(normal, point) - dot_3(normal, v0)
}

/// Port of `SParametricSamplerInternal::WeightTetrahedron`
/// (`ParametricSampler.cpp:1830-1844`).
#[expect(
    clippy::suboptimal_flops,
    reason = "each weight is `dot * inverse + 1.0`; fusing the multiply and add changes \
              the authored grid weights"
)]
fn weight_tetrahedron(sample: [f32; 3], t: [[f32; 3]; 4]) -> [f32; 4] {
    let normal = cross_3(subtract_3(t[3], t[0]), subtract_3(t[2], t[0]));
    let m = dot_3(normal, subtract_3(t[1], t[0]));
    // Reciprocate `m` once so all four components use the same normalization
    // factor and rounding behavior.
    let inverse = 1.0 / m;
    [
        dot_3(
            cross_3(subtract_3(t[2], t[1]), subtract_3(t[3], t[1])),
            subtract_3(sample, t[0]),
        ) * inverse
            + 1.0,
        dot_3(
            cross_3(subtract_3(t[0], t[2]), subtract_3(t[3], t[2])),
            subtract_3(sample, t[1]),
        ) * inverse
            + 1.0,
        dot_3(
            cross_3(subtract_3(t[0], t[3]), subtract_3(t[1], t[3])),
            subtract_3(sample, t[2]),
        ) * inverse
            + 1.0,
        dot_3(
            cross_3(subtract_3(t[2], t[0]), subtract_3(t[1], t[0])),
            subtract_3(sample, t[3]),
        ) * inverse
            + 1.0,
    ]
}

/// Port of `SParametricSamplerInternal::WeightPyramid`
/// (`ParametricSampler.cpp:1846-1886`).
fn weight_pyramid(sample: [f32; 3], t: [[f32; 3]; 5]) -> [f32; 5] {
    let mut weights = [0.0f32; 5];
    for edge in 0..4usize {
        let i0 = edge & 3;
        let i1 = (edge + 1) & 3;
        let i2 = (edge + 2) & 3;
        let tetrahedron = weight_tetrahedron(sample, [t[i0], t[i1], t[i2], t[4]]);
        // CryEngine keeps a tetrahedron only when its second weight is
        // non-negative, which selects the wedge the sample actually falls in.
        if tetrahedron[1] >= 0.0 {
            weights[i0] += tetrahedron[0];
            weights[i1] += tetrahedron[1];
            weights[i2] += tetrahedron[2];
            weights[4] += tetrahedron[3];
        }
    }
    // Preserve the source accumulation order and use one reciprocal for all
    // components.
    let sum = weights[1] + weights[0] + weights[2] + weights[3] + weights[4];
    if sum != 0.0 {
        let inverse = 1.0 / sum;
        for weight in &mut weights {
            *weight *= inverse;
        }
    }
    weights
}

/// Port of `SParametricSamplerInternal::WeightPrism`
/// (`ParametricSampler.cpp:1888-1950`). The face is a wedge: `t0..t3` are the
/// quad base and `t4`/`t5` the opposite edge.
fn weight_prism(sample: [f32; 3], t: [[f32; 3]; 6]) -> [f32; 6] {
    let mut weights = [0.0f32; 6];
    if plane_distance(t[0], t[1], t[5], sample) <= 0.0 {
        let pyramid = weight_pyramid(sample, [t[0], t[1], t[2], t[3], t[5]]);
        weights[0] += pyramid[0];
        weights[1] += pyramid[1];
        weights[2] += pyramid[2];
        weights[3] += pyramid[3];
        weights[5] += pyramid[4];
    }
    // The second plane is built from the same three points in the opposite
    // winding, so it is the first plane negated.
    if plane_distance(t[1], t[0], t[5], sample) <= 0.0 {
        let tetrahedron = weight_tetrahedron(sample, [t[0], t[1], t[5], t[4]]);
        weights[0] += tetrahedron[0];
        weights[1] += tetrahedron[1];
        weights[5] += tetrahedron[2];
        weights[4] += tetrahedron[3];
    }
    let sum = weights[1] + weights[0] + weights[2] + weights[3] + weights[4] + weights[5];
    if sum != 0.0 {
        let inverse = 1.0 / sum;
        for weight in &mut weights {
            *weight *= inverse;
        }
    }
    weights
}

/// Port of `SParametricSamplerInternal::GetConvex8`
/// (`ParametricSampler.cpp:1956-2268`). Writes the example weights and returns
/// the face-corner barycentric weights used for the inside-hull test.
#[expect(
    clippy::many_single_char_names,
    reason = "`a`..`f` are the up to six face corners of `GetConvex8` in vertex order and `v` \
              is the vertex the planarity test walks; the letters keep the one-to-one match \
              with the `t0..t5` argument order of `weight_tetrahedron`, `weight_pyramid` and \
              `weight_prism`"
)]
fn convex_8(
    face: &BlendSpaceFace,
    sample: [f32; 3],
    points: &[ParameterPoint],
    weights: &mut [f32],
) -> [f32; 8] {
    weights.fill(0.0);
    // As in `GetConvex4`, every component starts at 9999 so an unhandled face
    // arity can never pass the inside-hull test
    // (`ParametricSampler.cpp:1981-1989`).
    let mut barycentric = [9999.0f32; 8];

    let vertex = |slot: usize| -> Option<(usize, [f32; 3])> {
        let index = usize::from(*face.indices.get(slot)?);
        Some((index, points.get(index)?.parameters))
    };
    // "Something went wrong. This is a plane. we need a volume": a face whose
    // vertices all sit on `z == 0` is rejected outright
    // (`ParametricSampler.cpp:2014-2021`).
    let is_planar = |vertices: &[[f32; 3]]| {
        vertices
            .iter()
            .all(|v| v[2].abs() < DEGENERATE_VOLUME_EPSILON)
    };

    let arity = face.indices.len();
    match arity {
        4 => {
            let (Some(a), Some(b), Some(c), Some(d)) = (vertex(0), vertex(1), vertex(2), vertex(3))
            else {
                return barycentric;
            };
            let corners = [a.1, b.1, c.1, d.1];
            if is_planar(&corners) {
                return barycentric;
            }
            let tetrahedron = weight_tetrahedron(sample, corners);
            barycentric[..4].copy_from_slice(&tetrahedron);
            barycentric[4..].fill(0.0);
        }
        5 => {
            let (Some(a), Some(b), Some(c), Some(d), Some(e)) =
                (vertex(0), vertex(1), vertex(2), vertex(3), vertex(4))
            else {
                return barycentric;
            };
            let corners = [a.1, b.1, c.1, d.1, e.1];
            if is_planar(&corners) {
                return barycentric;
            }
            let pyramid = weight_pyramid(sample, corners);
            barycentric[..5].copy_from_slice(&pyramid);
            barycentric[5..].fill(0.0);
        }
        6 => {
            let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                vertex(0),
                vertex(1),
                vertex(2),
                vertex(3),
                vertex(4),
                vertex(5),
            ) else {
                return barycentric;
            };
            let corners = [a.1, b.1, c.1, d.1, e.1, f.1];
            // Test every vertex. Omitting the sixth point can accept a wedge
            // whose final vertex leaves the plane.
            if is_planar(&corners) {
                return barycentric;
            }
            let prism = weight_prism(sample, corners);
            barycentric[..6].copy_from_slice(&prism);
            barycentric[6..].fill(0.0);
        }
        _ => return barycentric,
    }

    #[expect(
        clippy::needless_range_loop,
        reason = "the index selects the face corner as well as the weight, so this loops over corner slots rather than over the weight slice"
    )]
    for slot in 0..arity.min(barycentric.len()) {
        if let Some((index, _)) = vertex(slot) {
            accumulate_point(points, index, barycentric[slot], weights);
        }
    }
    barycentric
}

/// Port of `SParametricSamplerInternal::GetWeights3D`
/// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:1566-1620`),
/// with the same widening tolerance ladder and positive-range test as
/// `GetWeights2D`.
#[expect(
    clippy::while_float,
    reason = "the tolerance ladder accumulates `HULL_TOLERANCE_STEP` in `f32` and loops on \
              that float comparison exactly as `GetWeights2D` does; an integer counter would \
              visit a different set of tolerances"
)]
fn weights_3d(
    sample: [f32; 3],
    points: &[ParameterPoint],
    faces: &[BlendSpaceFace],
    weights: &mut [f32],
) -> GridCellSource {
    weights.fill(0.0);
    let mut tolerance = 0.0f32;
    while tolerance < HULL_TOLERANCE_LIMIT {
        for (index, face) in faces.iter().enumerate() {
            let barycentric = convex_8(face, sample, points, weights);
            let inside = (0..face.indices.len().min(barycentric.len())).all(|slot| {
                -tolerance <= barycentric[slot] && barycentric[slot] <= 1.0 + tolerance
            });
            if inside {
                return GridCellSource::Accepted {
                    face: index,
                    tolerance,
                };
            }
        }
        tolerance += HULL_TOLERANCE_STEP;
    }
    // As in 2D, CryEngine returns -1 and leaves the weights holding whatever
    // the last evaluated face produced.
    GridCellSource::Unaccepted
}

/// Packs the non-zero example weights into one virtual-example cell.
///
/// `CryEngine` keeps `2^dimensions` slots, zero-fills them, then appends only
/// the non-zero weights, calling `CryFatalError("Invalid Weights")` when more
/// arrive than fit (`ParametricSampler.cpp:451-472` for 1D, `:741-765` for 2D).
fn pack_cell(
    weights: &[f32],
    slots: usize,
    grid_index: usize,
) -> Result<VirtualExample, InvalidBlendSpace> {
    let mut indices = ArrayVec::new();
    let mut packed = ArrayVec::new();
    for (index, &weight) in weights.iter().enumerate() {
        // CryEngine's `if (w)` is an exact non-zero test.
        if weight == 0.0 {
            continue;
        }
        if indices.len() == slots {
            return Err(InvalidBlendSpace::GridContributorOverflow { grid_index });
        }
        let index = u8::try_from(index)
            .map_err(|_| InvalidBlendSpace::GridContributorOverflow { grid_index })?;
        let _ = indices.try_push(index);
        let _ = packed.try_push(weight);
    }
    while indices.len() < slots {
        let _ = indices.try_push(0);
        let _ = packed.try_push(0.0);
    }
    Ok(VirtualExample {
        indices,
        weights: packed,
    })
}

/// Clamps every annotation corner into `0..points`, returning how many corners
/// moved.
///
/// This is `GlobalAnimationHeaderLMG::EnsureValidFaceExampleIndex`
/// (`GlobalAnimationHeaderLMG.cpp:1816-1823`), which `ReadFaces` applies to
/// every `p0`..`p7` attribute as it reads the annotation list
/// (`:730-779`). An index past the last parameter is an authoring error
/// `CryEngine` reports and then repairs - "has been clamped. Fix it in order to
/// work properly" - never a load failure, and the bound is the parameter count
/// (real examples plus pseudo examples), not the example count.
fn clamp_face_example_indices(faces: &mut [BlendSpaceFace], points: usize) -> usize {
    let Some(highest) = points.checked_sub(1) else {
        return 0;
    };
    let highest = u8::try_from(highest).unwrap_or(u8::MAX);
    let mut clamped = 0usize;
    for face in faces {
        for index in &mut face.indices {
            if *index > highest {
                *index = highest;
                clamped += 1;
            }
        }
    }
    clamped
}

/// Checks a resolved virtual grid - the authored cache or the rebuild - before
/// it becomes a [`ParametricBlendSpace`].
fn validate_virtual_grid(
    virtual_examples: &[VirtualExample],
    expected_contributors: usize,
    example_count: u8,
) -> Result<(), InvalidBlendSpace> {
    for (grid_index, example) in virtual_examples.iter().enumerate() {
        if example.indices.len() != expected_contributors
            || example.weights.len() != expected_contributors
        {
            return Err(InvalidBlendSpace::VirtualContributorCount {
                grid_index,
                actual_indices: example.indices.len(),
                actual_weights: example.weights.len(),
                expected: expected_contributors,
            });
        }
        let mut sum = 0.0f32;
        for (&index, &weight) in example.indices.iter().zip(&example.weights) {
            if index >= example_count {
                return Err(InvalidBlendSpace::VirtualExampleIndex {
                    grid_index,
                    example_index: index,
                });
            }
            if !weight.is_finite() {
                return Err(InvalidBlendSpace::VirtualWeight(grid_index));
            }
            sum += weight;
        }
        // CryEngine does not reject an off-by-one weight sum: `ReadVGrid`
        // validates only the entry count, and the 1D sampler merely draws a
        // debug label when the sum drifts (`ParametricSampler.cpp:477-481`).
        // Authored grids can contain all-zero cells outside every annotation,
        // and those must load.
        if (sum - 1.0).abs() > VIRTUAL_WEIGHT_TOLERANCE {
            tracing::debug!(
                grid_index,
                sum,
                "virtual-grid entry weights do not sum to one"
            );
        }
    }
    Ok(())
}

/// Logs how much of a rebuilt grid sits outside the annotation net.
///
/// A cell no annotation accepted holds whatever the last annotation left behind
/// (`ParametricSampler.cpp:1562`), and one accepted only after the hull
/// tolerance widened sits outside every annotation. Neither is an error -
/// `CryEngine` ships both - but they are the cells whose weights leave `[0, 1]`,
/// so say how many there are.
fn report_rebuilt_grid(rebuilt: &RebuiltGrid) {
    let mut outside = 0usize;
    let mut unaccepted = 0usize;
    let mut widest = 0.0f32;
    let mut widest_face = None;
    for source in &rebuilt.sources {
        match *source {
            GridCellSource::Accepted { face, tolerance } => {
                if tolerance > 0.0 {
                    outside += 1;
                    if tolerance > widest {
                        widest = tolerance;
                        widest_face = Some(face);
                    }
                }
            }
            GridCellSource::Extrapolated { face } => {
                outside += 1;
                widest_face = widest_face.or(Some(face));
            }
            GridCellSource::Unaccepted => unaccepted += 1,
        }
    }
    if outside > 0 || unaccepted > 0 {
        tracing::debug!(
            cells = rebuilt.sources.len(),
            outside,
            unaccepted,
            widest,
            widest_face,
            "rebuilt virtual grid has cells outside the annotations"
        );
    }
}

/// Rebuilds the virtual-example grid from the per-example motion parameters,
/// the way `CryEngine` does when the authored `<VGrid>` is absent or the wrong
/// size.
fn build_virtual_grid(
    dimensions: &[BlendSpaceDimension],
    example_count: u8,
    example_parameters: &[ExampleParameters],
    pseudo_examples: &[PseudoExample],
    faces: &[BlendSpaceFace],
) -> Result<RebuiltGrid, InvalidBlendSpace> {
    if example_parameters.len() != usize::from(example_count) {
        return Err(InvalidBlendSpace::ExampleParameterCount {
            actual: example_parameters.len(),
            expected: usize::from(example_count),
        });
    }
    let points = parameter_points(example_parameters, pseudo_examples);
    // `ReadFaces` clamps out-of-range annotation corners rather than failing, so
    // the builder must accept the same data its loader would
    // (`GlobalAnimationHeaderLMG.cpp:730-779`, `:1816-1823`). Callers that route
    // through `ParametricBlendSpace::try_from` have already been clamped, which
    // makes this a no-op for them.
    let mut faces = faces.to_vec();
    clamp_face_example_indices(&mut faces, points.len());
    let faces = faces.as_slice();

    let active = usize::from(example_count);
    let mut scratch = [0.0f32; MAX_BLEND_SPACE_MOTIONS];
    let weights = &mut scratch[..active];

    // `CryEngine` keeps `2^dimensions` contributor slots per cell
    // (`ParametricSampler.cpp:451-454`, `:741-748`, `:1140-1155`).
    let slots = match dimensions.len() {
        1 => 2,
        2 => 4,
        3 => 8,
        other => return Err(InvalidBlendSpace::GridRebuildUnsupported(other)),
    };
    let coordinates = grid_coordinates(dimensions);
    let mut grid = RebuiltGrid::with_capacity(coordinates.len());
    for (cell, coordinate) in coordinates.iter().enumerate() {
        let source = match dimensions.len() {
            1 => weights_1d(coordinate[0], &points, faces, weights),
            2 => weights_2d([coordinate[0], coordinate[1]], &points, faces, weights),
            _ => weights_3d(*coordinate, &points, faces, weights),
        };
        grid.push(pack_cell(weights, slots, cell)?, source);
    }
    Ok(grid)
}

/// The motion-parameter coordinate of every grid cell, in packed cell order.
///
/// Dimension zero is the fastest-changing coordinate, and `cell = c2 * cells1 *
/// cells0 + c1 * cells0 + c0` (`ParametricSampler.cpp:1138`), so walking z-major
/// fills the grid in `CryEngine`'s own sequence.
///
/// The 1D builder recomputes each coordinate as `f32(c0) * xstep + min`
/// (`ParametricSampler.cpp:449`) while the 2D and 3D builders accumulate it by
/// repeated addition (`:734-737`, `:1120-1128`); the two disagree in the last
/// bits, so each is reproduced as written. Lumberyard bounds the 2D and 3D
/// loops on the accumulated float (`for (f32 x = min; x <= max + 0.001f; x +=
/// xstep)`), which lets drift decide how many cells it visits. This port uses
/// the declared cell count as the bound so every cell is written exactly once.
fn grid_coordinates(dimensions: &[BlendSpaceDimension]) -> Vec<ExampleParameters> {
    let mut coordinates = Vec::new();
    match dimensions {
        [x] => {
            let step = (x.max - x.min) / f32::from(x.cells - 1);
            coordinates.reserve(usize::from(x.cells));
            for cell in 0..usize::from(x.cells) {
                coordinates.push([cell_coordinate(cell, step, x.min), 0.0, 0.0]);
            }
        }
        [x, y] => {
            let step_x = (x.max - x.min) / f32::from(x.cells - 1);
            let step_y = (y.max - y.min) / f32::from(y.cells - 1);
            coordinates.reserve(usize::from(x.cells) * usize::from(y.cells));
            let mut coordinate_y = y.min;
            for _ in 0..y.cells {
                let mut coordinate_x = x.min;
                for _ in 0..x.cells {
                    coordinates.push([coordinate_x, coordinate_y, 0.0]);
                    coordinate_x += step_x;
                }
                coordinate_y += step_y;
            }
        }
        [x, y, z] => {
            let step_x = (x.max - x.min) / f32::from(x.cells - 1);
            let step_y = (y.max - y.min) / f32::from(y.cells - 1);
            let step_z = (z.max - z.min) / f32::from(z.cells - 1);
            coordinates.reserve(usize::from(x.cells) * usize::from(y.cells) * usize::from(z.cells));
            let mut coordinate_z = z.min;
            for _ in 0..z.cells {
                let mut coordinate_y = y.min;
                for _ in 0..y.cells {
                    let mut coordinate_x = x.min;
                    for _ in 0..x.cells {
                        coordinates.push([coordinate_x, coordinate_y, coordinate_z]);
                        coordinate_x += step_x;
                    }
                    coordinate_y += step_y;
                }
                coordinate_z += step_z;
            }
        }
        _ => {}
    }
    coordinates
}

/// `f32(c0) * xstep + min` (`ParametricSampler.cpp:449`).
#[expect(
    clippy::suboptimal_flops,
    reason = "`ParametricSampler.cpp:449` recomputes the 1D coordinate as a multiply followed \
              by an add; fusing them shifts the sampled grid coordinate"
)]
fn cell_coordinate(cell: usize, step: f32, min: f32) -> f32 {
    let cell = u16::try_from(cell).unwrap_or(u16::MAX);
    f32::from(cell) * step + min
}

impl TryFrom<ParametricBlendSpaceDescription> for ParametricBlendSpace {
    type Error = InvalidBlendSpace;

    fn try_from(mut value: ParametricBlendSpaceDescription) -> Result<Self, Self::Error> {
        let dimension_count = value.dimensions.len();
        if !(1..=MAX_BLEND_SPACE_DIMENSIONS).contains(&dimension_count) {
            return Err(InvalidBlendSpace::DimensionCount(dimension_count));
        }
        if value.example_count == 0 || usize::from(value.example_count) > MAX_BLEND_SPACE_MOTIONS {
            return Err(InvalidBlendSpace::ExampleCount(value.example_count));
        }
        if value.additional_extraction.len() > MAX_BLEND_SPACE_EXTRACTION_PARAMETERS {
            return Err(InvalidBlendSpace::ExtractionParameterCount(
                value.additional_extraction.len(),
            ));
        }

        let mut expected_grid_size = 1usize;
        for (index, dimension) in value.dimensions.iter().enumerate() {
            if !dimension.min.is_finite()
                || !dimension.max.is_finite()
                || dimension.min >= dimension.max
            {
                return Err(InvalidBlendSpace::DimensionRange(index));
            }
            if dimension.cells < 2 {
                return Err(InvalidBlendSpace::DimensionCells(index));
            }
            expected_grid_size = expected_grid_size
                .checked_mul(usize::from(dimension.cells))
                .ok_or(InvalidBlendSpace::GridSizeOverflow)?;
        }
        if let Some(threshold) = value.threshold
            && (!threshold.is_finite() || threshold <= 0.0)
        {
            return Err(InvalidBlendSpace::Threshold);
        }

        // `ReadFaces` clamps an out-of-range annotation corner into the
        // parameter list as it reads it, so every later stage - the grid
        // rebuild and anything reading `faces()` - sees the repaired indices
        // (`GlobalAnimationHeaderLMG.cpp:730-779`, `:1816-1823`).
        let parameter_count = usize::from(value.example_count) + value.pseudo_examples.len();
        let clamped = clamp_face_example_indices(&mut value.faces, parameter_count);
        if clamped > 0 {
            tracing::warn!(
                clamped,
                parameter_count,
                "blend-space annotations reference examples the space does not have; clamped"
            );
        }

        // CryEngine's `<VGrid>` is a cache, not authority. `ReadVGrid` fills the
        // runtime grid only when the node's child count equals the product of
        // the dimension cell counts and otherwise returns without touching it
        // (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/GlobalAnimationHeaderLMG.cpp:126-130`).
        // The sampler then rebuilds whatever grid is still empty
        // (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/ParametricSampler.cpp:443-446,729-732`).
        let virtual_examples = if value.virtual_examples.len() == expected_grid_size {
            value.virtual_examples
        } else {
            let rebuilt = build_virtual_grid(
                &value.dimensions,
                value.example_count,
                &value.example_parameters,
                &value.pseudo_examples,
                &value.faces,
            )?;
            report_rebuilt_grid(&rebuilt);
            rebuilt.cells
        };

        validate_virtual_grid(
            &virtual_examples,
            1usize << dimension_count,
            value.example_count,
        )?;

        Ok(Self {
            dimensions: value
                .dimensions
                .into_iter()
                .collect::<ArrayVec<_, MAX_BLEND_SPACE_DIMENSIONS>>(),
            additional_extraction: value.additional_extraction.into_iter().collect(),
            example_count: value.example_count,
            example_parameters: value.example_parameters.into_boxed_slice(),
            pseudo_examples: value.pseudo_examples.into_boxed_slice(),
            faces: value.faces.into_boxed_slice(),
            virtual_examples: virtual_examples.into_boxed_slice(),
            threshold: value.threshold,
            idle_to_move: value.idle_to_move,
        })
    }
}

impl From<ParametricBlendSpace> for ParametricBlendSpaceDescription {
    fn from(value: ParametricBlendSpace) -> Self {
        Self {
            dimensions: value.dimensions.into_iter().collect(),
            additional_extraction: value.additional_extraction.into_iter().collect(),
            example_count: value.example_count,
            example_parameters: value.example_parameters.into_vec(),
            pseudo_examples: value.pseudo_examples.into_vec(),
            faces: value.faces.into_vec(),
            virtual_examples: value.virtual_examples.into_vec(),
            threshold: value.threshold,
            idle_to_move: value.idle_to_move,
        }
    }
}

impl Serialize for ParametricBlendSpace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ParametricBlendSpaceDescription {
            dimensions: self.dimensions.iter().copied().collect(),
            additional_extraction: self.additional_extraction.iter().copied().collect(),
            example_count: self.example_count,
            example_parameters: self.example_parameters.to_vec(),
            pseudo_examples: self.pseudo_examples.to_vec(),
            faces: self.faces.to_vec(),
            virtual_examples: self.virtual_examples.to_vec(),
            threshold: self.threshold,
            idle_to_move: self.idle_to_move,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ParametricBlendSpace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(ParametricBlendSpaceDescription::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidBlendSpace {
    #[error("blend space has {0} dimensions; expected 1..={MAX_BLEND_SPACE_DIMENSIONS}")]
    DimensionCount(usize),
    #[error("blend space has {0} examples; expected 1..={MAX_BLEND_SPACE_MOTIONS}")]
    ExampleCount(u8),
    #[error(
        "blend space has {0} extraction parameters; expected at most {MAX_BLEND_SPACE_EXTRACTION_PARAMETERS}"
    )]
    ExtractionParameterCount(usize),
    #[error("blend-space dimension {0} has a non-finite or empty range")]
    DimensionRange(usize),
    #[error("blend-space dimension {0} has fewer than two grid cells")]
    DimensionCells(usize),
    #[error("blend-space virtual-grid size overflowed usize")]
    GridSizeOverflow,
    #[error("blend-space threshold must be finite and greater than zero")]
    Threshold,
    #[error(
        "virtual-grid entry {grid_index} has {actual_indices} indices and {actual_weights} weights; expected {expected} of each"
    )]
    VirtualContributorCount {
        grid_index: usize,
        actual_indices: usize,
        actual_weights: usize,
        expected: usize,
    },
    #[error("virtual-grid entry {grid_index} references invalid example {example_index}")]
    VirtualExampleIndex {
        grid_index: usize,
        example_index: u8,
    },
    #[error("virtual-grid entry {0} contains a non-finite weight")]
    VirtualWeight(usize),
    #[error("blend space has {actual} example parameter sets; expected {expected}")]
    ExampleParameterCount { actual: usize, expected: usize },
    #[error(
        "computed virtual-grid entry {grid_index} needs more contributors than the grid provides"
    )]
    GridContributorOverflow { grid_index: usize },
    #[error("cannot rebuild a {0}-dimensional virtual grid; only 1D and 2D are ported")]
    GridRebuildUnsupported(usize),
}

/// Reusable fixed sampler output. The active prefix is the blend space's
/// example count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendWeights {
    weights: [f32; MAX_BLEND_SPACE_MOTIONS],
    len: u8,
}

impl Default for BlendWeights {
    fn default() -> Self {
        Self {
            weights: [0.0; MAX_BLEND_SPACE_MOTIONS],
            len: 0,
        }
    }
}

impl BlendWeights {
    pub fn set_direct(&mut self) {
        self.clear(1);
        self.weights[0] = 1.0;
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "`len` is the sampler's example count, which validation bounds to \
                  `MAX_BLEND_SPACE_MOTIONS` (40); a larger value would already have panicked \
                  on the slice above"
    )]
    fn clear(&mut self, len: usize) {
        self.weights[..len].fill(0.0);
        self.len = len as u8;
    }

    fn normalize(&mut self) {
        let active = &mut self.weights[..usize::from(self.len)];
        for weight in active.iter_mut() {
            if weight.abs() < WEIGHT_EPSILON {
                *weight = 0.0;
            }
        }
        let mut total = active.iter().sum::<f32>();
        if total == 0.0 {
            active[0] = 1.0;
            total = 1.0;
        }
        for weight in active {
            *weight /= total;
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.weights[..usize::from(self.len)]
    }

    pub fn active(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.as_slice()
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, weight)| *weight != 0.0)
    }
}

impl AsRef<[f32]> for BlendWeights {
    fn as_ref(&self) -> &[f32] {
        self.as_slice()
    }
}

/// Timing and root-path data for one motion segment.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionSegmentTiming {
    pub normalized_start: f32,
    pub normalized_end: f32,
    pub duration: f32,
    pub travel_distance: f32,
    pub mean_travel_speed: f32,
}

/// Fixed-capacity timing data for one direct or parametric motion example.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTiming {
    duration: f32,
    playback_scale: f32,
    sample_rate: f32,
    segment_count: u8,
    segments: [MotionSegmentTiming; MAX_MOTION_SEGMENTS],
}

impl MotionTiming {
    /// Builds the timing table for one motion from its authored segment
    /// boundaries, returning `None` when any input is out of range.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`segment_count` is checked to be within 1..=MAX_MOTION_SEGMENTS above, so it \
                  always fits in a `u8`"
    )]
    #[must_use]
    pub fn from_segments(
        duration: f32,
        playback_scale: f32,
        sample_rate: f32,
        normalized_boundaries: &[f32],
        travel_distances: &[f32],
        mean_travel_speeds: &[f32],
    ) -> Option<Self> {
        let segment_count = normalized_boundaries.len().checked_sub(1)?;
        if !(1..=MAX_MOTION_SEGMENTS).contains(&segment_count)
            || travel_distances.len() != segment_count
            || mean_travel_speeds.len() != segment_count
            || !duration.is_finite()
            || duration <= 0.0
            || !playback_scale.is_finite()
            || playback_scale < 0.0
            || !sample_rate.is_finite()
            || sample_rate <= 0.0
            || normalized_boundaries.first().copied() != Some(0.0)
            || normalized_boundaries.last().copied() != Some(1.0)
            || !normalized_boundaries
                .windows(2)
                .all(|pair| pair[0].is_finite() && pair[0] < pair[1])
            || travel_distances
                .iter()
                .any(|distance| !distance.is_finite() || *distance < 0.0)
            || mean_travel_speeds
                .iter()
                .any(|speed| !speed.is_finite() || *speed < 0.0)
        {
            return None;
        }

        let mut segments = [MotionSegmentTiming::default(); MAX_MOTION_SEGMENTS];
        for index in 0..segment_count {
            let normalized_start = normalized_boundaries[index];
            let normalized_end = normalized_boundaries[index + 1];
            let segment_duration = duration * (normalized_end - normalized_start);
            let travel_distance = travel_distances[index];
            segments[index] = MotionSegmentTiming {
                normalized_start,
                normalized_end,
                duration: segment_duration,
                travel_distance,
                mean_travel_speed: mean_travel_speeds[index],
            };
        }

        Some(Self {
            duration,
            playback_scale,
            sample_rate,
            segment_count: segment_count as u8,
            segments,
        })
    }

    #[must_use]
    pub fn single(
        duration: f32,
        playback_scale: f32,
        sample_rate: f32,
        travel_distance: f32,
    ) -> Option<Self> {
        Self::from_segments(
            duration,
            playback_scale,
            sample_rate,
            &[0.0, 1.0],
            &[travel_distance],
            &[travel_distance / duration],
        )
    }

    #[must_use]
    pub const fn duration(self) -> f32 {
        self.duration
    }

    #[must_use]
    pub const fn playback_scale(self) -> f32 {
        self.playback_scale
    }

    #[must_use]
    pub const fn sample_rate(self) -> f32 {
        self.sample_rate
    }

    #[must_use]
    pub const fn segment_count(self) -> usize {
        self.segment_count as usize
    }

    #[must_use]
    pub fn segment(self, index: usize) -> Option<MotionSegmentTiming> {
        (index < self.segment_count()).then(|| self.segments[index])
    }

    /// The `fSegTime` a playback clock divides by.
    ///
    /// `CryEngine` never divides by the raw segment duration: every site floors it
    /// at one sample period first — `max(GetSegmentDuration(seg), 1/GetSampleRate())`
    /// in `CSkeletonAnim::UpdateParameters` for both the direct-CAF and the
    /// parametric branch (`SkeletonAnim_BlendMan.cpp:294,326`) and again in
    /// `SParametricSamplerInternal::Parameterizer` (`ParametricSampler.cpp:212`).
    #[must_use]
    pub fn clock_segment_duration(self, index: usize) -> Option<f32> {
        Some(self.segment(index)?.duration.max(1.0 / self.sample_rate))
    }

    #[must_use]
    pub fn with_playback_scale(mut self, playback_scale: f32) -> Option<Self> {
        if !playback_scale.is_finite() || playback_scale < 0.0 {
            return None;
        }
        self.playback_scale = playback_scale;
        Some(self)
    }

    /// Maps a phase within `segment` onto the motion's normalized clock.
    #[expect(
        clippy::suboptimal_flops,
        reason = "the segment phase is interpolated as `start + phase * (end - start)`, the \
                  form the playback clock was ported from; fusing it changes the normalized \
                  time the sampler advances to"
    )]
    #[must_use]
    pub fn normalized_time(self, segment: usize, phase: f32) -> Option<f32> {
        let segment = self.segment(segment)?;
        Some(
            segment.normalized_start
                + phase.clamp(0.0, 1.0) * (segment.normalized_end - segment.normalized_start),
        )
    }
}

/// Computes the shared normalized-clock increment used by all examples in a
/// parametric motion.
#[expect(
    clippy::suboptimal_flops,
    reason = "the four weighted sums accumulate as `weight * value` added to the running \
              total, exactly as `CSkeletonAnim::UpdateParameters` does; fusing them changes \
              the clock increment every example plays at"
)]
#[must_use]
pub fn parameterized_normalized_delta(
    frame_delta_time: f32,
    weights: &BlendWeights,
    timings: &[MotionTiming],
    segment_indices: &[u8],
) -> Option<f32> {
    if timings.len() != weights.as_slice().len()
        || segment_indices.len() != timings.len()
        || frame_delta_time < 0.0
    {
        return None;
    }

    let mut duration = 0.0f32;
    let mut speed = 0.0f32;
    let mut distance = 0.0f32;
    let mut playback_scale = 0.0f32;
    for (index, weight) in weights.active() {
        let timing = timings[index];
        let segment_index = usize::from(segment_indices[index]);
        let segment = timing.segment(segment_index)?;
        duration += weight * timing.clock_segment_duration(segment_index)?;
        speed += weight * segment.mean_travel_speed;
        distance += weight * segment.travel_distance;
        playback_scale += weight * timing.playback_scale();
    }

    if duration <= 0.0 {
        return None;
    }
    Some(if distance < TRAVEL_DISTANCE_EPSILON {
        playback_scale * (frame_delta_time / duration)
    } else {
        playback_scale * frame_delta_time * (speed / distance)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn virtual_example(indices: &[u8], weights: &[f32]) -> VirtualExample {
        VirtualExample {
            indices: indices.iter().copied().collect(),
            weights: weights.iter().copied().collect(),
        }
    }

    fn face(indices: &[u8]) -> BlendSpaceFace {
        BlendSpaceFace {
            indices: indices.iter().copied().collect(),
        }
    }

    fn dimension(min: f32, max: f32, cells: u8) -> BlendSpaceDimension {
        BlendSpaceDimension {
            parameter: MotionParameterId::TravelSpeed,
            min,
            max,
            cells,
            locked: false,
        }
    }

    /// Builds a description whose authored grid is empty, forcing the rebuild.
    fn rebuilt(
        dimensions: Vec<BlendSpaceDimension>,
        example_parameters: Vec<ExampleParameters>,
        faces: Vec<BlendSpaceFace>,
    ) -> ParametricBlendSpace {
        let example_count = u8::try_from(example_parameters.len()).expect("example count fits u8");
        ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions,
            additional_extraction: Vec::new(),
            example_count,
            example_parameters,
            pseudo_examples: Vec::new(),
            faces,
            virtual_examples: Vec::new(),
            threshold: None,
            idle_to_move: false,
        })
        .expect("grid rebuilds from example parameters")
    }

    /// Three examples at 0, 1 and 2 on a five-cell grid over [0, 2]. Cell
    /// centres land at 0, 0.5, 1, 1.5, 2, so the interior cells split their
    /// bracketing line exactly in half.
    #[test]
    fn one_dimensional_grid_is_built_from_example_parameters() {
        let space = rebuilt(
            vec![dimension(0.0, 2.0, 5)],
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            vec![face(&[0, 1]), face(&[1, 2])],
        );

        let grid = space.virtual_examples();
        assert_eq!(grid.len(), 5);
        // x = 0.0 sits on example 0, so only one weight is non-zero and
        // CryEngine's `if (w)` packing leaves the second slot empty.
        assert_eq!(grid[0].indices.as_slice(), &[0, 0]);
        assert_eq!(grid[0].weights.as_slice(), &[1.0, 0.0]);
        // x = 0.5 is halfway along line (0, 1).
        assert_eq!(grid[1].indices.as_slice(), &[0, 1]);
        assert_eq!(grid[1].weights.as_slice(), &[0.5, 0.5]);
        // x = 1.0 is the shared endpoint; line (0, 1) matches first and puts
        // all of the weight on example 1.
        assert_eq!(grid[2].indices.as_slice(), &[1, 0]);
        assert_eq!(grid[2].weights.as_slice(), &[1.0, 0.0]);
        // x = 1.5 is halfway along line (1, 2).
        assert_eq!(grid[3].indices.as_slice(), &[1, 2]);
        assert_eq!(grid[3].weights.as_slice(), &[0.5, 0.5]);
        assert_eq!(grid[4].indices.as_slice(), &[2, 0]);
        assert_eq!(grid[4].weights.as_slice(), &[1.0, 0.0]);
    }

    /// A grid cell outside every line extrapolates onto the nearest line
    /// (`ParametricSampler.cpp:1406-1502`), producing weights that still sum to
    /// one but fall outside [0, 1].
    #[test]
    fn one_dimensional_grid_extrapolates_outside_the_annotations() {
        // Examples at 1 and 3; the grid spans [0, 4] with five cells, so cells
        // 0 and 4 lie outside the only line.
        let space = rebuilt(
            vec![dimension(0.0, 4.0, 5)],
            vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            vec![face(&[0, 1])],
        );

        let grid = space.virtual_examples();
        // x = 0: d = -1, distance = 2, so w0 = 1.5 and w1 = -0.5.
        assert_eq!(grid[0].indices.as_slice(), &[0, 1]);
        assert_eq!(grid[0].weights.as_slice(), &[1.5, -0.5]);
        // x = 4: d = 3, so w0 = -0.5 and w1 = 1.5.
        assert_eq!(grid[4].indices.as_slice(), &[0, 1]);
        assert_eq!(grid[4].weights.as_slice(), &[-0.5, 1.5]);
    }

    /// One triangle covering the unit square's lower-left half, on a 2x2 grid.
    /// Barycentric weights are hand computed from
    /// `ParametricSampler.cpp:1714-1722`.
    #[test]
    fn two_dimensional_grid_is_built_from_triangle_annotations() {
        let space = rebuilt(
            vec![dimension(0.0, 1.0, 2), dimension(0.0, 1.0, 2)],
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![face(&[0, 1, 2])],
        );

        let grid = space.virtual_examples();
        assert_eq!(grid.len(), 4);
        // (0,0) is vertex 0: u = 0, v = 0, w = 1 after normalisation, so only
        // the third barycentric slot (example 0, the triangle's v2) is
        // non-zero... but the triangle is (v0=0, v1=1, v2=2), so the corner
        // that carries all the weight at (0,0) is v0 = example 0.
        assert_eq!(grid[0].indices.as_slice(), &[0, 0, 0, 0]);
        assert_eq!(grid[0].weights.as_slice(), &[1.0, 0.0, 0.0, 0.0]);
        // (1,0) is vertex 1.
        assert_eq!(grid[1].indices.as_slice(), &[1, 0, 0, 0]);
        assert_eq!(grid[1].weights.as_slice(), &[1.0, 0.0, 0.0, 0.0]);
        // (0,1) is vertex 2.
        assert_eq!(grid[2].indices.as_slice(), &[2, 0, 0, 0]);
        assert_eq!(grid[2].weights.as_slice(), &[1.0, 0.0, 0.0, 0.0]);
        // (1,1) is outside the triangle: u = 1, v = 1, w = -1, summing to one.
        assert_eq!(grid[3].indices.as_slice(), &[0, 1, 2, 0]);
        assert_eq!(grid[3].weights.as_slice(), &[-1.0, 1.0, 1.0, 0.0]);
    }

    /// An authored grid of exactly the right size is `CryEngine`'s cache and
    /// must be used verbatim, even where it disagrees with what we would
    /// compute.
    /// `ReadVGrid` validates only the entry count
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/GlobalAnimationHeaderLMG.cpp:126-130`).
    #[test]
    fn correctly_sized_authored_grid_is_used_verbatim() {
        let authored = vec![
            virtual_example(&[1, 0], &[1.0, 0.0]),
            virtual_example(&[0, 0], &[1.0, 0.0]),
        ];
        let space = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: vec![dimension(0.0, 1.0, 2)],
            additional_extraction: Vec::new(),
            example_count: 2,
            example_parameters: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            pseudo_examples: Vec::new(),
            faces: vec![face(&[0, 1])],
            virtual_examples: authored.clone(),
            threshold: None,
            idle_to_move: false,
        })
        .expect("authored grid of the right size loads");

        // The authored grid deliberately swaps the two cells, so this only
        // passes if the cache wins over the computed grid.
        assert_eq!(space.virtual_examples(), authored.as_slice());
    }

    /// The same inputs with the authored grid one entry short must be rebuilt,
    /// and the rebuild must agree with what the correctly sized cache would say.
    #[test]
    fn short_authored_grid_is_rebuilt_and_matches_the_cache() {
        let dimensions = vec![dimension(0.0, 1.0, 3)];
        let example_parameters = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let faces = vec![face(&[0, 1])];

        let computed = rebuilt(
            dimensions.clone(),
            example_parameters.clone(),
            faces.clone(),
        );
        let cached = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions,
            additional_extraction: Vec::new(),
            example_count: 2,
            example_parameters,
            pseudo_examples: Vec::new(),
            faces,
            // One entry short, so `ReadVGrid` would reject it and the sampler
            // would rebuild.
            virtual_examples: vec![virtual_example(&[0, 0], &[1.0, 0.0])],
            threshold: None,
            idle_to_move: false,
        })
        .expect("short authored grid rebuilds");

        assert_eq!(computed.virtual_examples(), cached.virtual_examples());
        assert_eq!(
            computed.virtual_examples()[1].weights.as_slice(),
            &[0.5, 0.5]
        );
    }

    /// An authored grid whose cells sum to zero must still load: `CryEngine`
    /// validates only the entry count.
    #[test]
    fn zero_weight_authored_cells_are_accepted() {
        let space = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: vec![dimension(0.0, 1.0, 2)],
            additional_extraction: Vec::new(),
            example_count: 2,
            example_parameters: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            pseudo_examples: Vec::new(),
            faces: vec![face(&[0, 1])],
            virtual_examples: vec![
                virtual_example(&[0, 0], &[0.0, 0.0]),
                virtual_example(&[1, 0], &[1.0, 0.0]),
            ],
            threshold: None,
            idle_to_move: false,
        })
        .expect("all-zero authored cells load");

        assert_eq!(space.virtual_examples()[0].weights.as_slice(), &[0.0, 0.0]);
    }

    #[test]
    fn current_parameter_names_resolve() {
        assert_eq!(
            MotionParameterId::from_cry_name("MoveSpeed"),
            Some(MotionParameterId::TravelSpeed)
        );
        assert_eq!(
            MotionParameterId::from_cry_name("SlopeYaw"),
            Some(MotionParameterId::SlopeYaw)
        );
        assert_eq!(
            MotionParameterId::from_cry_name("SlopePitch"),
            Some(MotionParameterId::SlopePitch)
        );
        assert_eq!(
            MotionParameterId::from_cry_name("TravelDist"),
            Some(MotionParameterId::TravelDistance)
        );
        assert_eq!(MotionParameterId::from_cry_name("NotAParameter"), None);
    }

    #[test]
    fn current_motion_parameter_ids_are_stable() {
        assert_eq!(u8::from(MotionParameterId::TravelSpeed), 0);
        assert_eq!(u8::from(MotionParameterId::BlendWeight4), 10);
        assert_eq!(u8::from(MotionParameterId::DesiredFacing), 13);
        assert_eq!(u8::from(MotionParameterId::VelocityX), 14);
        assert_eq!(u8::from(MotionParameterId::VelocityY), 15);
        assert_eq!(u8::from(MotionParameterId::SlopeYaw), 16);
        assert_eq!(u8::from(MotionParameterId::SlopePitch), 17);
        assert_eq!(MOTION_PARAMETER_COUNT, 18);
        assert!(MotionParameterId::TravelAngle.is_cyclic());
        assert!(MotionParameterId::DesiredFacing.is_cyclic());
        assert!(!MotionParameterId::AimHorizontalNavigationAngle.is_cyclic());
        assert!(!MotionParameterId::SlopeYaw.is_cyclic());
    }

    #[test]
    fn desired_parameter_history_advances_on_accepted_and_rejected_requests() {
        let mut parameters = MotionParameters::default();

        assert!(parameters.record_desired(MotionParameterId::TravelSpeed, 0.25, true));
        assert_eq!(parameters.get(MotionParameterId::TravelSpeed), Some(0.25));
        assert_eq!(
            parameters.desired(MotionParameterId::TravelSpeed),
            Some(0.25)
        );
        assert_eq!(
            parameters.previous_desired(MotionParameterId::TravelSpeed),
            Some(0.0)
        );

        assert!(!parameters.record_desired(MotionParameterId::TravelSpeed, 0.75, false));
        assert_eq!(parameters.get(MotionParameterId::TravelSpeed), Some(0.25));
        assert_eq!(
            parameters.desired(MotionParameterId::TravelSpeed),
            Some(0.25)
        );
        assert_eq!(
            parameters.previous_desired(MotionParameterId::TravelSpeed),
            Some(0.25)
        );

        assert!(parameters.record_desired(MotionParameterId::TravelSpeed, 1.0, true));
        assert_eq!(parameters.get(MotionParameterId::TravelSpeed), Some(1.0));
        assert_eq!(
            parameters.desired(MotionParameterId::TravelSpeed),
            Some(1.0)
        );
        assert_eq!(
            parameters.previous_desired(MotionParameterId::TravelSpeed),
            Some(0.25)
        );
    }

    #[test]
    fn one_dimensional_grid_interpolates_adjacent_virtual_examples() {
        let space = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
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
            faces: vec![face(&[0, 1])],
            virtual_examples: vec![
                virtual_example(&[0, 0], &[1.0, 0.0]),
                virtual_example(&[1, 0], &[1.0, 0.0]),
            ],
            threshold: None,
            idle_to_move: false,
        })
        .unwrap();
        let mut parameters = MotionParameters::default();
        parameters.set(MotionParameterId::TravelSpeed, 0.25);
        let mut weights = BlendWeights::default();

        space.evaluate(&parameters, &mut weights);

        assert_eq!(weights.as_slice(), &[0.75, 0.25]);
    }

    #[test]
    fn three_dimensional_threshold_suppresses_the_third_parameter() {
        let dimensions = [
            MotionParameterId::TravelSpeed,
            MotionParameterId::TurnSpeed,
            MotionParameterId::TravelAngle,
        ]
        .map(|parameter| BlendSpaceDimension {
            parameter,
            min: -1.0,
            max: 1.0,
            cells: 2,
            locked: false,
        });
        let mut virtual_examples = Vec::new();
        for index in 0..8u8 {
            virtual_examples.push(virtual_example(
                &[index, 0, 0, 0, 0, 0, 0, 0],
                &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ));
        }
        let space = ParametricBlendSpace::try_from(ParametricBlendSpaceDescription {
            dimensions: dimensions.into_iter().collect(),
            additional_extraction: Vec::new(),
            example_count: 8,
            // The authored grid is exactly the right size, so it is used as-is
            // and never rebuilt; no annotations are needed.
            example_parameters: Vec::new(),
            pseudo_examples: Vec::new(),
            faces: Vec::new(),
            virtual_examples,
            threshold: Some(0.5),
            idle_to_move: false,
        })
        .unwrap();
        let mut parameters = MotionParameters::default();
        parameters
            .set(MotionParameterId::TravelSpeed, -1.0)
            .set(MotionParameterId::TurnSpeed, 1.0)
            .set(MotionParameterId::TravelAngle, 1.0);
        let mut weights = BlendWeights::default();

        space.evaluate(&parameters, &mut weights);

        assert!((weights.as_slice()[2] - 0.4995).abs() < 0.00001);
        assert!((weights.as_slice()[6] - 0.4995).abs() < 0.00001);
    }

    /// The unit tetrahedron with the sample at (0.25, 0.25, 0.25). Every
    /// signed volume is a quarter of the whole, so all four weights are 0.25.
    /// Hand-computed: `n = (t3-t0) % (t2-t0) = (-1, 0, 0)`, `m = n | (t1-t0) =
    /// -1`, and each face normal dotted with the sample offset is 0.75, so
    /// every weight is `0.75 / -1 + 1`.
    #[expect(
        clippy::float_cmp,
        reason = "the hand-computed barycentrics are exact quarters, so the port is only \
                  correct if it reproduces them bit for bit"
    )]
    #[test]
    fn tetrahedron_weights_are_the_hand_computed_barycentrics() {
        let weights = weight_tetrahedron(
            [0.25, 0.25, 0.25],
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
        );

        assert_eq!(weights, [0.25, 0.25, 0.25, 0.25]);
    }

    /// A square-based pyramid sampled at the centre of its base. Each of the
    /// four tetrahedra `CryEngine` decomposes it into contributes `(0.5, 0,
    /// 0.5, 0)`, so every base corner accumulates 1.0 and the apex 0.0; the sum
    /// of 4.0 then normalises them to a quarter each.
    #[expect(
        clippy::float_cmp,
        reason = "the hand-computed barycentrics are exact quarters, so the port is only \
                  correct if it reproduces them bit for bit"
    )]
    #[test]
    fn pyramid_weights_split_a_square_base_evenly() {
        let weights = weight_pyramid(
            [0.5, 0.5, 0.0],
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.5, 0.5, 1.0],
            ],
        );

        assert_eq!(weights, [0.25, 0.25, 0.25, 0.25, 0.0]);
    }

    /// A wedge whose quad base is the unit square at `z = 0` and whose ridge
    /// runs from (0,0,1) to (1,0,1). The sample sits at the base centre, which
    /// is behind the `t0,t1,t5` plane (signed distance -0.5) and in front of
    /// its reverse, so only the pyramid half contributes and the tetrahedron
    /// half is skipped. The pyramid returns quarters over the base, and the
    /// wedge's own sum of 1.0 leaves them untouched.
    #[expect(
        clippy::float_cmp,
        reason = "the hand-computed barycentrics are exact quarters, so the port is only \
                  correct if it reproduces them bit for bit"
    )]
    #[test]
    fn prism_weights_use_only_the_pyramid_half_at_the_base_centre() {
        let weights = weight_prism(
            [0.5, 0.5, 0.0],
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
        );

        assert_eq!(weights, [0.25, 0.25, 0.25, 0.25, 0.0, 0.0]);
    }

    /// End-to-end 3D rebuild over the same wedge. The grid is 3x2x2 and the
    /// second dimension starts at 0.5, which keeps every sample strictly in
    /// front of the `t1,t0,t5` plane so the wedge's second half - the
    /// tetrahedron `(t0, t1, t5, t4)`, which is degenerate for this geometry
    /// because all four points share `y = 0` - is never entered. The first cell
    /// of the first row lands on the base centre and must carry the four
    /// quarters computed above, packed into the eight contributor slots a 3D
    /// cell holds.
    #[test]
    fn three_dimensional_grid_is_built_from_prism_annotations() {
        let space = rebuilt(
            vec![
                dimension(0.0, 1.0, 3),
                dimension(0.5, 1.0, 2),
                dimension(0.0, 1.0, 2),
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
            vec![face(&[0, 1, 2, 3, 4, 5])],
        );

        assert_eq!(space.virtual_examples.len(), 12);
        // `cell = c2 * cells1 * cells0 + c1 * cells0 + c0` = 0*6 + 0*3 + 1,
        // which is the sample (0.5, 0.5, 0).
        assert_eq!(
            space.virtual_examples[1],
            virtual_example(
                &[0, 1, 2, 3, 0, 0, 0, 0],
                &[0.25, 0.25, 0.25, 0.25, 0.0, 0.0, 0.0, 0.0]
            )
        );
    }

    /// A 3D annotation whose vertices all sit on `z == 0` is not a volume, so
    /// `GetConvex8` refuses it, no face ever passes the inside-hull test, and
    /// every cell keeps whatever the last face produced - here all zeros.
    #[test]
    fn flat_three_dimensional_annotations_are_rejected_as_degenerate() {
        let space = rebuilt(
            vec![
                dimension(0.0, 1.0, 2),
                dimension(0.0, 1.0, 2),
                dimension(0.0, 1.0, 2),
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.5, 0.0],
                [1.0, 0.5, 0.0],
            ],
            vec![face(&[0, 1, 2, 3, 4, 5])],
        );

        assert_eq!(space.virtual_examples.len(), 8);
        for cell in &space.virtual_examples {
            assert_eq!(cell.weights.as_slice(), &[0.0; 8]);
        }
    }

    #[test]
    fn time_warp_uses_speed_over_distance_for_moving_examples() {
        let mut weights = BlendWeights::default();
        weights.clear(2);
        weights.weights[0] = 0.25;
        weights.weights[1] = 0.75;
        let timings = [
            MotionTiming::from_segments(1.0, 1.0, 30.0, &[0.0, 1.0], &[1.0], &[2.0]).unwrap(),
            MotionTiming::from_segments(2.0, 0.5, 30.0, &[0.0, 1.0], &[3.0], &[3.0]).unwrap(),
        ];

        let delta = parameterized_normalized_delta(0.1, &weights, &timings, &[0, 0]).unwrap();

        assert!((delta - 0.06875).abs() < 0.00001);
    }

    #[test]
    fn time_warp_uses_the_current_motion_segment() {
        let mut weights = BlendWeights::default();
        weights.set_direct();
        let timing = MotionTiming::from_segments(
            2.0,
            1.0,
            30.0,
            &[0.0, 0.25, 1.0],
            &[0.0, 3.0],
            &[0.0, 2.0],
        )
        .unwrap();

        let idle_delta = parameterized_normalized_delta(0.1, &weights, &[timing], &[0]).unwrap();
        let moving_delta = parameterized_normalized_delta(0.1, &weights, &[timing], &[1]).unwrap();

        assert!((idle_delta - 0.2).abs() < 0.00001);
        assert!((moving_delta - (0.1 / 1.5)).abs() < 0.00001);
    }

    /// `CryEngine` never divides by a raw segment duration shorter than one
    /// sample period (`ParametricSampler.cpp:212`), so an authored segment that
    /// lands between two keys still advances at the one-sample rate instead of
    /// exploding the clock.
    #[expect(
        clippy::float_cmp,
        reason = "the authored segment duration must survive the timing table unchanged, so \
                  the comparison against `0.01` is deliberately exact"
    )]
    #[expect(
        clippy::suboptimal_flops,
        reason = "the expected delta is spelled `0.1 * 30.0` to mirror the one-sample floor \
                  under test; fusing it into the subtraction changes the tolerance check"
    )]
    #[test]
    fn time_warp_floors_a_sub_sample_segment_at_one_sample_period() {
        let mut weights = BlendWeights::default();
        weights.set_direct();
        // Segment 0 is 1/300 s long at a 30 Hz sample rate, an order of
        // magnitude under one sample period.
        let timing = MotionTiming::from_segments(
            1.0,
            1.0,
            30.0,
            &[0.0, 0.01, 1.0],
            &[0.0, 1.0],
            &[0.0, 1.0],
        )
        .unwrap();

        assert_eq!(timing.segment(0).unwrap().duration, 0.01);
        assert!((timing.clock_segment_duration(0).unwrap() - 1.0 / 30.0).abs() < 1e-7);
        let idle_delta = parameterized_normalized_delta(0.1, &weights, &[timing], &[0]).unwrap();
        assert!((idle_delta - 0.1 * 30.0).abs() < 0.00001);
    }

    // -----------------------------------------------------------------------
    // Invariants every *computed* virtual grid has to satisfy.
    //
    // The grid is CryEngine's cache of `GetWeights1D/2D/3D`: cell `c` holds the
    // example weights the sampler would compute at that cell's motion-parameter
    // coordinate. Every bound below is taken from the engine rather than chosen
    // to fit.
    // -----------------------------------------------------------------------

    use std::collections::BTreeSet;

    /// `ParametricSampler.cpp:474-481` draws an "invalid sum" label over a
    /// packed 1D cell whose weights miss one by more than this, so it is
    /// `CryEngine`'s own statement of how close a virtual example has to be to
    /// a partition of unity.
    const PARTITION_TOLERANCE: f32 = 0.001;

    /// Absolute slack on the `[-d, 1 + d]` hull bound. The hull test runs on
    /// the barycentric weights; the packed weights are those weights routed
    /// through the pseudo-example decomposition, so they can round a little
    /// past the bound the test itself saw.
    const WEIGHT_BOUND_SLOP: f32 = 0.001;

    /// Relative slack on the reconstruction identity, scaled by the size of the
    /// weights and of the example cloud.
    const RECONSTRUCTION_TOLERANCE: f32 = 1e-4;

    /// Two annotations that share the boundary an adjacent-cell step crosses
    /// agree on that boundary, so crossing from one to the other costs at most
    /// what staying on the first would have cost up to the boundary plus what
    /// staying on the second would have cost from it - at most twice the larger
    /// of the two single-annotation costs.
    ///
    /// The argument needs the step to cross a *shared* boundary, which is only
    /// true while both cells are inside the annotation net. Once the hull
    /// tolerance widens, `GetWeights2D` and `GetWeights3D` return the first
    /// annotation that accepts the sample at the current tolerance
    /// (`ParametricSampler.cpp:1540-1552`, `:1597-1609`), and two adjacent cells
    /// outside the net can land on annotations that share no boundary at all -
    /// so the bound is stated only over pairs both annotations claimed at
    /// tolerance zero. The cells outside are counted and their jump reported.
    const CONTINUITY_SLACK: f32 = 2.0;

    /// Absolute floor under the continuity bound, so a pair of cells whose
    /// annotations are both flat across the step is not compared against zero.
    const CONTINUITY_FLOOR: f32 = 1e-3;

    fn l1_distance(left: &[f32], right: &[f32]) -> f32 {
        left.iter()
            .zip(right)
            .map(|(left, right)| (left - right).abs())
            .sum()
    }

    /// Every `.bspace.ron` under `directory`, recursively.
    fn collect_blend_space_sources(
        directory: &std::path::Path,
        found: &mut Vec<std::path::PathBuf>,
    ) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_blend_space_sources(&path, found);
            } else if path.to_string_lossy().ends_with(".bspace.ron") {
                found.push(path);
            }
        }
    }

    /// One packed cell as a dense per-example vector.
    fn dense_weights(cell: &VirtualExample, example_count: usize) -> Vec<f32> {
        let mut dense = vec![0.0f32; example_count];
        for (&example, &weight) in cell.indices.iter().zip(&cell.weights) {
            if weight != 0.0
                && let Some(slot) = dense.get_mut(usize::from(example))
            {
                *slot += weight;
            }
        }
        dense
    }

    /// A description whose authored grid is empty, so it is always rebuilt.
    fn description(
        dimensions: Vec<BlendSpaceDimension>,
        example_parameters: Vec<ExampleParameters>,
        pseudo_examples: Vec<PseudoExample>,
        faces: Vec<BlendSpaceFace>,
    ) -> ParametricBlendSpaceDescription {
        let example_count = u8::try_from(example_parameters.len()).expect("example count fits u8");
        ParametricBlendSpaceDescription {
            dimensions,
            additional_extraction: Vec::new(),
            example_count,
            example_parameters,
            pseudo_examples,
            faces,
            virtual_examples: Vec::new(),
            threshold: None,
            idle_to_move: false,
        }
    }

    /// What one grid check measured. The universal invariants assert inside
    /// [`GridUnderTest::check`]; callers state their own bounds over these
    /// measurements.
    #[derive(Debug, Clone, Copy, Default)]
    struct GridReport {
        cells: usize,
        empty_cells: usize,
        degenerate_cells: usize,
        unaccepted_cells: usize,
        extrapolated_cells: usize,
        widened_cells: usize,
        widest_tolerance: f32,
        worst_partition_error: f32,
        worst_reconstruction_error: f32,
        worst_continuity_ratio: f32,
        worst_outside_jump: f32,
    }

    impl GridReport {
        fn merge(&mut self, other: Self) {
            self.cells += other.cells;
            self.empty_cells += other.empty_cells;
            self.degenerate_cells += other.degenerate_cells;
            self.unaccepted_cells += other.unaccepted_cells;
            self.extrapolated_cells += other.extrapolated_cells;
            self.widened_cells += other.widened_cells;
            self.widest_tolerance = self.widest_tolerance.max(other.widest_tolerance);
            self.worst_partition_error =
                self.worst_partition_error.max(other.worst_partition_error);
            self.worst_reconstruction_error = self
                .worst_reconstruction_error
                .max(other.worst_reconstruction_error);
            self.worst_continuity_ratio = self
                .worst_continuity_ratio
                .max(other.worst_continuity_ratio);
            self.worst_outside_jump = self.worst_outside_jump.max(other.worst_outside_jump);
        }
    }

    /// The invariants that can be stated without knowing which annotation
    /// produced a cell.
    ///
    /// [`GridReport`] leans on the rebuild's provenance, which an authored
    /// `<VGrid>` does not carry, so this is the measurement that runs
    /// over both and lets the rebuilt grid be compared against the cache as a
    /// control rather than only against itself.
    #[derive(Debug, Clone, Copy)]
    struct ControlReport {
        cells: usize,
        empty_cells: usize,
        non_finite_cells: usize,
        out_of_range_cells: usize,
        worst_partition_error: f32,
        weight_low: f32,
        weight_high: f32,
        worst_reconstruction_error: f32,
        worst_jump: f32,
    }

    impl Default for ControlReport {
        fn default() -> Self {
            Self {
                cells: 0,
                empty_cells: 0,
                non_finite_cells: 0,
                out_of_range_cells: 0,
                worst_partition_error: 0.0,
                weight_low: f32::INFINITY,
                weight_high: f32::NEG_INFINITY,
                worst_reconstruction_error: 0.0,
                worst_jump: 0.0,
            }
        }
    }

    impl ControlReport {
        fn merge(&mut self, other: Self) {
            self.cells += other.cells;
            self.empty_cells += other.empty_cells;
            self.non_finite_cells += other.non_finite_cells;
            self.out_of_range_cells += other.out_of_range_cells;
            self.worst_partition_error =
                self.worst_partition_error.max(other.worst_partition_error);
            self.weight_low = self.weight_low.min(other.weight_low);
            self.weight_high = self.weight_high.max(other.weight_high);
            self.worst_reconstruction_error = self
                .worst_reconstruction_error
                .max(other.worst_reconstruction_error);
            self.worst_jump = self.worst_jump.max(other.worst_jump);
        }
    }

    /// A rebuilt grid together with everything the invariants are stated over.
    struct GridUnderTest {
        dimensions: Vec<BlendSpaceDimension>,
        example_count: usize,
        points: Vec<ParameterPoint>,
        faces: Vec<BlendSpaceFace>,
        coordinates: Vec<ExampleParameters>,
        cells: Vec<VirtualExample>,
        sources: Vec<GridCellSource>,
    }

    fn grid_under_test(description: &ParametricBlendSpaceDescription) -> GridUnderTest {
        let rebuilt = build_virtual_grid(
            &description.dimensions,
            description.example_count,
            &description.example_parameters,
            &description.pseudo_examples,
            &description.faces,
        )
        .expect("the grid rebuilds from the example parameters");
        GridUnderTest {
            dimensions: description.dimensions.clone(),
            example_count: usize::from(description.example_count),
            points: parameter_points(
                &description.example_parameters,
                &description.pseudo_examples,
            ),
            faces: description.faces.clone(),
            coordinates: grid_coordinates(&description.dimensions),
            cells: rebuilt.cells,
            sources: rebuilt.sources,
        }
    }

    impl GridUnderTest {
        fn axes(&self) -> usize {
            self.dimensions.len()
        }

        /// The cell's packed weights as a dense per-example vector.
        fn cell_weights(&self, cell: usize) -> Vec<f32> {
            dense_weights(&self.cells[cell], self.example_count)
        }

        /// One annotation's own weight map at `coordinate`, with no hull test:
        /// what the cell would have held had the sampler stayed on that
        /// annotation.
        fn face_weights(&self, face: usize, coordinate: ExampleParameters) -> Vec<f32> {
            let mut weights = vec![0.0f32; self.example_count];
            let face = &self.faces[face];
            match self.axes() {
                1 => {
                    if let Some((i0, i1, x0, x1)) = line_segment(face, &self.points) {
                        let distance = x1 - x0;
                        let offset = coordinate[0] - x0;
                        accumulate_point(&self.points, i0, 1.0 - offset / distance, &mut weights);
                        accumulate_point(&self.points, i1, offset / distance, &mut weights);
                    }
                }
                2 => {
                    convex_4(
                        face,
                        [coordinate[0], coordinate[1]],
                        &self.points,
                        &mut weights,
                    );
                }
                _ => {
                    convex_8(face, coordinate, &self.points, &mut weights);
                }
            }
            weights
        }

        /// The next cell one step along `axis`, if the grid has one.
        fn neighbour(&self, cell: usize, axis: usize) -> Option<usize> {
            let mut stride = 1usize;
            for dimension in self.dimensions.iter().take(axis) {
                stride *= usize::from(dimension.cells);
            }
            let cells = usize::from(self.dimensions[axis].cells);
            ((cell / stride) % cells + 1 < cells).then_some(cell + stride)
        }

        /// The real examples one annotation may put weight on: every corner is
        /// a real example, or a pseudo example that decomposes into two real
        /// ones (`GlobalAnimationHeaderLMG.cpp:700-703`).
        fn face_examples(&self, face: usize) -> BTreeSet<usize> {
            let mut examples = BTreeSet::new();
            for &corner in &self.faces[face].indices {
                let Some(point) = self.points.get(usize::from(corner)) else {
                    continue;
                };
                if point.w0 != 0.0 {
                    examples.insert(usize::from(point.i0));
                }
                if point.w1 != 0.0 {
                    examples.insert(usize::from(point.i1));
                }
            }
            examples
        }

        /// How many of the annotation's corners can put weight on `example`.
        /// The packed weight is the sum over those corners, so the per-corner
        /// hull bound scales with this.
        fn corner_multiplicity(&self, face: usize, example: usize) -> f32 {
            let mut corners = 0u8;
            for &corner in &self.faces[face].indices {
                let Some(point) = self.points.get(usize::from(corner)) else {
                    continue;
                };
                if (point.w0 != 0.0 && usize::from(point.i0) == example)
                    || (point.w1 != 0.0 && usize::from(point.i1) == example)
                {
                    corners = corners.saturating_add(1);
                }
            }
            f32::from(corners).max(1.0)
        }

        /// Whether `GetConvex4` would leave this annotation's weights
        /// unnormalised.
        ///
        /// The three-corner branch divides the barycentrics by the triangle's
        /// determinant only when `fabsf(det) > FLT_EPSILON`, and otherwise
        /// keeps the raw cross products (`ParametricSampler.cpp:1714-1722`).
        /// A zero-area triangle - two corners on the same motion parameter, or
        /// three collinear ones - therefore yields weights that sum to the
        /// determinant rather than to one, all of them tiny, so the hull test
        /// waves them through. The cell is not a partition of unity and does
        /// not reconstruct its own coordinate.
        ///
        /// Only the three-corner branch can do this. `ComputeWeightExtrapolate4`
        /// always divides by the accumulated sum, and the 3D branches guard
        /// their own division on a non-zero sum.
        #[expect(
            clippy::suboptimal_flops,
            reason = "the determinant is `ParametricSampler.cpp:1719` transcribed, and the test \
                      has to see the value the sampler branched on"
        )]
        fn face_is_degenerate(&self, face: usize) -> bool {
            let face = &self.faces[face];
            if self.axes() != 2 || face.indices.len() != 3 {
                return false;
            }
            let corner = |slot: usize| -> [f32; 2] {
                face.indices
                    .get(slot)
                    .and_then(|&index| self.points.get(usize::from(index)))
                    .map_or([0.0; 2], |point| [point.parameters[0], point.parameters[1]])
            };
            let (v0, v1, v2) = (corner(0), corner(1), corner(2));
            let z0 = [v0[0] - v2[0], v0[1] - v2[1]];
            let z1 = [v1[0] - v2[0], v1[1] - v2[1]];
            (z0[0] * z1[1] - z1[0] * z0[1]).abs() <= f32::EPSILON
        }

        /// The largest example-coordinate magnitude, floored at one: the scale
        /// the reconstruction error is measured against.
        fn parameter_scale(&self) -> f32 {
            self.points
                .iter()
                .flat_map(|point| point.parameters.iter())
                .fold(1.0f32, |scale, value| scale.max(value.abs()))
        }

        /// Measures the provenance-free invariants over any grid laid out on
        /// this space's coordinates: either the rebuild or an authored cache.
        fn measure_any(&self, cells: &[VirtualExample]) -> ControlReport {
            let mut report = ControlReport {
                cells: cells.len(),
                ..ControlReport::default()
            };
            let scale_base = self.parameter_scale();
            for (index, cell) in cells.iter().enumerate() {
                let mut contributors = Vec::new();
                let mut non_finite = false;
                let mut out_of_range = false;
                for (&example, &weight) in cell.indices.iter().zip(&cell.weights) {
                    if !weight.is_finite() {
                        non_finite = true;
                    } else if usize::from(example) >= self.example_count {
                        out_of_range = true;
                    } else if weight != 0.0 {
                        contributors.push((usize::from(example), weight));
                    }
                }
                report.non_finite_cells += usize::from(non_finite);
                report.out_of_range_cells += usize::from(out_of_range);
                if non_finite || contributors.is_empty() {
                    report.empty_cells += usize::from(!non_finite);
                    continue;
                }

                let sum: f32 = contributors.iter().map(|&(_, weight)| weight).sum();
                report.worst_partition_error = report.worst_partition_error.max((sum - 1.0).abs());
                for &(_, weight) in &contributors {
                    report.weight_low = report.weight_low.min(weight);
                    report.weight_high = report.weight_high.max(weight);
                }

                let magnitude: f32 = contributors
                    .iter()
                    .map(|&(_, weight)| weight.abs())
                    .sum::<f32>();
                let scale = scale_base * (1.0 + magnitude);
                for axis in 0..self.axes() {
                    let blended: f32 = contributors
                        .iter()
                        .map(|&(example, weight)| weight * self.points[example].parameters[axis])
                        .sum();
                    let relative = (blended - self.coordinates[index][axis]).abs() / scale;
                    if relative.is_finite() {
                        report.worst_reconstruction_error =
                            report.worst_reconstruction_error.max(relative);
                    }
                }
            }

            for index in 0..cells.len() {
                let here = dense_weights(&cells[index], self.example_count);
                for axis in 0..self.axes() {
                    let Some(next) = self.neighbour(index, axis) else {
                        continue;
                    };
                    let jump = l1_distance(&here, &dense_weights(&cells[next], self.example_count));
                    if jump.is_finite() {
                        report.worst_jump = report.worst_jump.max(jump);
                    }
                }
            }
            report
        }

        fn check(&self) -> GridReport {
            assert_eq!(
                self.coordinates.len(),
                self.cells.len(),
                "one cell per grid coordinate"
            );
            assert_eq!(
                self.sources.len(),
                self.cells.len(),
                "one provenance per cell"
            );
            let mut report = GridReport {
                cells: self.cells.len(),
                ..GridReport::default()
            };
            for index in 0..self.cells.len() {
                self.check_cell(index, &mut report);
                self.measure_continuity(index, &mut report);
            }
            report
        }

        fn check_cell(&self, index: usize, report: &mut GridReport) {
            let cell = &self.cells[index];
            let source = self.sources[index];
            let expected = 1usize << self.axes();
            assert_eq!(
                cell.indices.len(),
                expected,
                "cell {index} holds {} contributor slots",
                cell.indices.len()
            );
            assert_eq!(
                cell.weights.len(),
                expected,
                "cell {index} holds {} weight slots",
                cell.weights.len()
            );
            for &example in &cell.indices {
                assert!(
                    usize::from(example) < self.example_count,
                    "cell {index} names example {example}, but the space has {} examples",
                    self.example_count
                );
            }
            for &weight in &cell.weights {
                assert!(
                    weight.is_finite(),
                    "cell {index} holds the non-finite weight {weight}"
                );
            }

            match source {
                GridCellSource::Accepted { tolerance, .. } => {
                    if tolerance > 0.0 {
                        report.widened_cells += 1;
                    }
                    report.widest_tolerance = report.widest_tolerance.max(tolerance);
                }
                GridCellSource::Extrapolated { .. } => report.extrapolated_cells += 1,
                GridCellSource::Unaccepted => report.unaccepted_cells += 1,
            }

            // A cell no annotation claimed holds whatever the last annotation
            // left behind (`ParametricSampler.cpp:1562`), which the engine
            // itself does not bound; there is nothing to state about it beyond
            // its count.
            let Some(face) = source.face() else {
                return;
            };

            let contributors: Vec<(usize, f32)> = cell
                .indices
                .iter()
                .zip(&cell.weights)
                .filter(|&(_, &weight)| weight != 0.0)
                .map(|(&example, &weight)| (usize::from(example), weight))
                .collect();
            if contributors.is_empty() {
                // `WeightPyramid` and `WeightPrism` leave every weight at zero
                // when no sub-tetrahedron claims the sample
                // (`ParametricSampler.cpp:1874-1877`, `:1937-1940`), and zero
                // weights pass the hull test, so an accepted cell can still be
                // empty. CryEngine ships such holes.
                report.empty_cells += 1;
                return;
            }

            // Contributor validity: the weights may only name examples the
            // selected annotation actually touches.
            let allowed = self.face_examples(face);
            for &(example, weight) in &contributors {
                assert!(
                    allowed.contains(&example),
                    "cell {index} puts {weight} on example {example}, which annotation {face} \
                     does not touch"
                );
            }

            // Everything below is a statement about barycentric weights, and a
            // degenerate annotation never produces any.
            if self.face_is_degenerate(face) {
                report.degenerate_cells += 1;
                return;
            }

            // Partition of unity.
            let sum: f32 = contributors.iter().map(|&(_, weight)| weight).sum();
            report.worst_partition_error = report.worst_partition_error.max((sum - 1.0).abs());
            assert!(
                (sum - 1.0).abs() <= PARTITION_TOLERANCE,
                "cell {index} weights sum to {sum}, not to one"
            );

            // Weight bounds: the hull test accepted this annotation only
            // because every barycentric weight was inside `[-d, 1 + d]`, and a
            // packed weight is the sum over the corners that decompose onto
            // that example.
            #[expect(
                clippy::suboptimal_flops,
                reason = "the bound is the engine's `[-d, 1 + d]` scaled by the corner \
                          multiplicity and then slackened; fusing the pair moves the bound the \
                          assertion is stated over"
            )]
            if let GridCellSource::Accepted { tolerance, .. } = source {
                for &(example, weight) in &contributors {
                    let multiplicity = self.corner_multiplicity(face, example);
                    let low = -tolerance * multiplicity - WEIGHT_BOUND_SLOP;
                    let high = (1.0 + tolerance) * multiplicity + WEIGHT_BOUND_SLOP;
                    assert!(
                        weight >= low && weight <= high,
                        "cell {index} puts {weight} on example {example}, outside \
                         [{low}, {high}] for hull tolerance {tolerance}"
                    );
                }
            }

            self.check_linear_precision(index, &contributors, report);
        }

        /// Linear precision. Every branch of the sampler produces barycentric
        /// weights over the annotation's corners, and a pseudo corner is itself
        /// a convex blend of two real examples (`ParametricSampler.cpp:437`), so
        /// blending the example coordinates with the packed weights has to give
        /// the cell's own coordinate back. This is the statement that the
        /// contributors really do bracket the sample rather than merely being
        /// nearby.
        fn check_linear_precision(
            &self,
            index: usize,
            contributors: &[(usize, f32)],
            report: &mut GridReport,
        ) {
            let magnitude: f32 = contributors
                .iter()
                .map(|&(_, weight)| weight.abs())
                .sum::<f32>();
            let scale = self.parameter_scale() * (1.0 + magnitude);
            for axis in 0..self.axes() {
                let blended: f32 = contributors
                    .iter()
                    .map(|&(example, weight)| weight * self.points[example].parameters[axis])
                    .sum();
                let relative = (blended - self.coordinates[index][axis]).abs() / scale;
                report.worst_reconstruction_error = report.worst_reconstruction_error.max(relative);
                assert!(
                    relative <= RECONSTRUCTION_TOLERANCE,
                    "cell {index} blends to {blended} on axis {axis} instead of its own \
                     coordinate {}",
                    self.coordinates[index][axis]
                );
            }
        }

        /// Records how far the blend jumps between adjacent cells, relative to
        /// what either of the two annotations involved would itself have
        /// produced over the same step.
        ///
        /// The ratio is only meaningful where both cells sat inside the
        /// annotation net; a pair that needed a widened hull tolerance is
        /// measured as a raw jump instead. See [`CONTINUITY_SLACK`].
        fn measure_continuity(&self, index: usize, report: &mut GridReport) {
            let Some(here_face) = self.sources[index].face() else {
                return;
            };
            let here_coordinate = self.coordinates[index];
            let here_weights = self.cell_weights(index);
            let inside = |source: GridCellSource| matches!(source, GridCellSource::Accepted { tolerance, .. } if tolerance == 0.0);
            for axis in 0..self.axes() {
                let Some(next) = self.neighbour(index, axis) else {
                    continue;
                };
                let Some(there_face) = self.sources[next].face() else {
                    continue;
                };
                let there_coordinate = self.coordinates[next];
                let jump = l1_distance(&here_weights, &self.cell_weights(next));
                if !jump.is_finite() {
                    continue;
                }
                if !inside(self.sources[index]) || !inside(self.sources[next]) {
                    report.worst_outside_jump = report.worst_outside_jump.max(jump);
                    continue;
                }
                let own = l1_distance(
                    &self.face_weights(here_face, here_coordinate),
                    &self.face_weights(here_face, there_coordinate),
                );
                let neighbour = l1_distance(
                    &self.face_weights(there_face, here_coordinate),
                    &self.face_weights(there_face, there_coordinate),
                );
                let allowed = own
                    .max(neighbour)
                    .mul_add(CONTINUITY_SLACK, CONTINUITY_FLOOR);
                if !allowed.is_finite() {
                    continue;
                }
                report.worst_continuity_ratio = report.worst_continuity_ratio.max(jump / allowed);
            }
        }
    }

    /// The engine's own bound on how far a virtual-example weight may leave
    /// `[0, 1]`. `GetWeights2D` and `GetWeights3D` accept an annotation only
    /// when every barycentric weight lies in `[-d, 1 + d]`, widening `d` from
    /// zero in `0.05` steps while `d < 2.35` (`ParametricSampler.cpp:1522`,
    /// `:1540-1547`, `:1579`, `:1597-1604`).
    ///
    /// Accumulating `0.05f` drifts below the exact multiples, so the ladder
    /// runs 48 times and its last tolerance is 2.3499990, not the 2.30 exact
    /// arithmetic would stop at. The widest weight the engine can therefore
    /// accept is 3.3499990 and the most negative is -2.3499990.
    #[expect(
        clippy::while_float,
        reason = "the ladder is the engine's own float accumulation; counting it with an integer \
                  would measure a different ladder than the one under test"
    )]
    #[test]
    fn hull_tolerance_ladder_matches_the_shipped_binary() {
        let mut tolerance = 0.0f32;
        let mut steps = 0usize;
        let mut last = 0.0f32;
        while tolerance < HULL_TOLERANCE_LIMIT {
            last = tolerance;
            steps += 1;
            tolerance += HULL_TOLERANCE_STEP;
        }

        assert_eq!(steps, 48);
        assert!(
            last > 2.349_998 && last < HULL_TOLERANCE_LIMIT,
            "last {last}"
        );
        assert_eq!(HULL_TOLERANCE_LIMIT.to_bits(), 0x4016_6666);
        assert_eq!(HULL_TOLERANCE_STEP.to_bits(), 0x3d4c_cccd);
    }

    /// Three examples on a line, five cells: the ordinary 1D case, where every
    /// cell is bracketed by an annotation.
    #[test]
    fn one_dimensional_chain_holds_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![dimension(0.0, 2.0, 5)],
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            Vec::new(),
            vec![face(&[0, 1]), face(&[1, 2])],
        ))
        .check();

        assert_eq!(report.cells, 5);
        assert_eq!(report.unaccepted_cells, 0);
        assert_eq!(report.empty_cells, 0);
        assert_eq!(report.extrapolated_cells, 0);
        assert!(report.worst_continuity_ratio <= 1.0);
    }

    /// The same space with the grid wider than the annotation, so the outer
    /// cells extrapolate. Their weights leave `[0, 1]` but still sum to one and
    /// still reproduce their own coordinate.
    #[test]
    fn one_dimensional_extrapolation_holds_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![dimension(0.0, 4.0, 9)],
            vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            Vec::new(),
            vec![face(&[0, 1])],
        ))
        .check();

        assert_eq!(report.cells, 9);
        assert_eq!(report.extrapolated_cells, 4);
        assert_eq!(report.unaccepted_cells, 0);
        assert!(report.worst_continuity_ratio <= 1.0);
    }

    /// A pseudo example splits the packed weight over two real examples, so the
    /// contributor set and the reconstruction both have to follow the
    /// decomposition rather than the corner index.
    #[test]
    fn one_dimensional_pseudo_examples_hold_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![dimension(0.0, 2.0, 9)],
            vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            // A point a quarter of the way along, standing in for a third
            // example the space does not have.
            vec![PseudoExample {
                i0: 0,
                w0: 0.75,
                i1: 1,
                w1: 0.25,
            }],
            vec![face(&[0, 2]), face(&[2, 1])],
        ))
        .check();

        assert_eq!(report.cells, 9);
        assert_eq!(report.unaccepted_cells, 0);
        assert_eq!(report.empty_cells, 0);
        assert!(report.worst_partition_error <= PARTITION_TOLERANCE);
        assert!(report.worst_continuity_ratio <= 1.0);
    }

    /// Two examples that extract to the same motion parameter make a
    /// zero-length annotation, and interpolating along it is `0 / 0`.
    ///
    /// `CryEngine`'s checked build warns "parameters in 1D-Blend-Space are too
    /// close" and returns `-1` with every weight left at zero
    /// (`ParametricSampler.cpp:1338-1354`), and `CRY_ASSERT(fDistance)` states
    /// the same precondition on both division sites (`:1370`, `:1419`).
    /// [`line_segment`] drops the line, which reproduces the checked build's
    /// zero weights instead of a grid full of `NaN` - and a `NaN` here is fatal,
    /// because `ParametricBlendSpace::try_from` rejects a non-finite grid
    /// weight outright.
    #[test]
    fn a_zero_length_annotation_yields_empty_cells_rather_than_nan() {
        let space = description(
            vec![dimension(0.0, 1.57, 9)],
            vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
            Vec::new(),
            vec![face(&[0, 1])],
        );
        let grid = grid_under_test(&space);
        let report = grid.check();

        assert_eq!(report.cells, 9);
        assert_eq!(report.unaccepted_cells, 9, "no cell may claim the line");
        for cell in &grid.cells {
            for &weight in &cell.weights {
                // Compared on the bit pattern with the sign bit masked off, so
                // this is an exact test for either zero and not a float
                // comparison with a tolerance hiding inside it.
                assert_eq!(
                    weight.to_bits() & 0x7fff_ffff,
                    0,
                    "a dropped annotation leaves no weight behind, but the cell holds {weight}"
                );
            }
        }
        ParametricBlendSpace::try_from(space)
            .expect("a grid of zeros loads where a grid of NaN could not");
    }

    /// An annotation corner naming an example the space does not have is
    /// clamped into range, not rejected.
    ///
    /// `ReadFaces` runs every `p0`..`p7` attribute through
    /// `EnsureValidFaceExampleIndex`, which warns "has been clamped. Fix it in
    /// order to work properly" and clamps
    /// (Lumberyard reference: `dev/Gems/CryLegacy/Code/Source/CryAnimation/GlobalAnimationHeaderLMG.cpp:730-779,1816-1823`).
    #[test]
    fn an_out_of_range_annotation_corner_is_clamped() {
        let space = ParametricBlendSpace::try_from(description(
            vec![dimension(0.0, 1.0, 5), dimension(0.0, 1.0, 5)],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            Vec::new(),
            vec![face(&[0, 1, 2, 3]), face(&[1, 9, 2])],
        ))
        .expect("an out-of-range corner is clamped, not rejected");

        // The bound is the last parameter, so `9` becomes `3`.
        assert_eq!(
            space.faces()[1].indices.as_slice(),
            &[1, 3, 2],
            "the corner must be clamped to the last parameter"
        );
        assert_eq!(space.virtual_examples().len(), 25);
    }

    /// A jittered lattice of examples, triangulated, swept over many jitter
    /// patterns. The jitter stays well under half a lattice step so every quad
    /// remains convex and every annotation conforming, which is the regime the
    /// continuity bound is derived for.
    #[test]
    fn jittered_two_dimensional_lattices_hold_the_grid_invariants() {
        let mut worst = GridReport::default();
        for seed in 0..64u32 {
            let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                f32::from(u16::try_from(state >> 16).unwrap_or(0)) / f32::from(u16::MAX) - 0.5
            };
            let mut examples = Vec::new();
            for row in 0..4u8 {
                for column in 0..4u8 {
                    let jitter_x = next() * 0.5;
                    let jitter_y = next() * 0.5;
                    examples.push([f32::from(column) + jitter_x, f32::from(row) + jitter_y, 0.0]);
                }
            }
            let mut faces = Vec::new();
            for row in 0..3u8 {
                for column in 0..3u8 {
                    let base = row * 4 + column;
                    // The quad's corners must run round the perimeter: the
                    // extrapolating quad weights split it on the `1-3` and
                    // `0-2` diagonals (`ParametricSampler.cpp:1648-1655`).
                    faces.push(face(&[base, base + 1, base + 5, base + 4]));
                }
            }
            let report = grid_under_test(&description(
                vec![dimension(0.0, 3.0, 7), dimension(0.0, 3.0, 7)],
                examples,
                Vec::new(),
                faces,
            ))
            .check();
            assert_eq!(report.cells, 49);
            assert_eq!(report.unaccepted_cells, 0, "seed {seed}");
            assert_eq!(report.empty_cells, 0, "seed {seed}");
            worst.merge(report);
        }

        assert!(
            worst.worst_partition_error <= PARTITION_TOLERANCE,
            "partition error {}",
            worst.worst_partition_error
        );
        assert!(
            worst.worst_reconstruction_error <= RECONSTRUCTION_TOLERANCE,
            "reconstruction error {}",
            worst.worst_reconstruction_error
        );
        assert!(
            worst.worst_continuity_ratio <= 1.0,
            "continuity ratio {}",
            worst.worst_continuity_ratio
        );
    }

    /// The same lattice triangulated instead of quadded, which takes the
    /// three-corner branch of `GetConvex4`.
    #[test]
    fn triangulated_two_dimensional_lattices_hold_the_grid_invariants() {
        let mut examples = Vec::new();
        for row in 0..4u8 {
            for column in 0..4u8 {
                examples.push([f32::from(column), f32::from(row), 0.0]);
            }
        }
        let mut faces = Vec::new();
        for row in 0..3u8 {
            for column in 0..3u8 {
                let base = row * 4 + column;
                faces.push(face(&[base, base + 1, base + 5]));
                faces.push(face(&[base, base + 5, base + 4]));
            }
        }

        let report = grid_under_test(&description(
            vec![dimension(0.0, 3.0, 10), dimension(0.0, 3.0, 10)],
            examples,
            Vec::new(),
            faces,
        ))
        .check();

        assert_eq!(report.cells, 100);
        assert_eq!(report.unaccepted_cells, 0);
        assert_eq!(report.empty_cells, 0);
        assert!(report.worst_continuity_ratio <= 1.0);
    }

    /// A grid wider than the example cloud, so the border cells only pass the
    /// hull test after the tolerance widens. Their weights must still sum to
    /// one and still stay inside `[-d, 1 + d]` for the tolerance that accepted
    /// them.
    #[test]
    fn two_dimensional_cells_outside_the_hull_hold_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![dimension(-1.0, 2.0, 7), dimension(-1.0, 2.0, 7)],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            Vec::new(),
            vec![face(&[0, 1, 2, 3])],
        ))
        .check();

        assert_eq!(report.cells, 49);
        assert_eq!(report.unaccepted_cells, 0);
        assert!(report.widened_cells > 0, "the border cells must widen");
        assert!(
            report.widest_tolerance < HULL_TOLERANCE_LIMIT,
            "widest tolerance {}",
            report.widest_tolerance
        );
    }

    /// The unit tetrahedron over a 5x5x5 grid: the four-corner branch of
    /// `GetConvex8`.
    #[test]
    fn three_dimensional_tetrahedron_holds_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![
                dimension(0.0, 1.0, 5),
                dimension(0.0, 1.0, 5),
                dimension(0.0, 1.0, 5),
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            Vec::new(),
            vec![face(&[0, 1, 2, 3])],
        ))
        .check();

        assert_eq!(report.cells, 125);
        assert_eq!(report.unaccepted_cells, 0);
        assert_eq!(report.empty_cells, 0);
    }

    /// The square pyramid of `pyramid_weights_split_a_square_base_evenly` over
    /// a 5x5x5 grid: the five-corner branch of `GetConvex8`.
    #[test]
    fn three_dimensional_pyramid_holds_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![
                dimension(0.0, 1.0, 5),
                dimension(0.0, 1.0, 5),
                dimension(0.0, 1.0, 5),
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.5, 0.5, 1.0],
            ],
            Vec::new(),
            vec![face(&[0, 1, 2, 3, 4])],
        ))
        .check();

        assert_eq!(report.cells, 125);
        assert_eq!(report.unaccepted_cells, 0);
    }

    /// The wedge of `prism_weights_use_only_the_pyramid_half_at_the_base_centre`
    /// over a 5x3x3 grid: the six-corner branch of `GetConvex8`.
    #[test]
    fn three_dimensional_prism_holds_the_grid_invariants() {
        let report = grid_under_test(&description(
            vec![
                dimension(0.0, 1.0, 5),
                dimension(0.5, 1.0, 3),
                dimension(0.0, 1.0, 3),
            ],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
            Vec::new(),
            vec![face(&[0, 1, 2, 3, 4, 5])],
        ))
        .check();

        assert_eq!(report.cells, 45);
        assert_eq!(report.unaccepted_cells, 0);
    }

    /// Continuity, stated without reference to the annotation geometry:
    /// halving the grid step has to halve the largest jump between adjacent
    /// cells. A genuine discontinuity - a cell that switches to an annotation
    /// it does not share a boundary with - would not shrink with the step.
    ///
    /// Stated over a triangulated net, where `GetConvex4`'s three-corner branch
    /// is an exact barycentric map (`ParametricSampler.cpp:1714-1722`): two
    /// triangles sharing an edge agree along it, so the blend is continuous and
    /// its gradient bounded, and the largest step across one cell is
    /// proportional to the cell size. Quads do not qualify - see
    /// [`quad_weights_step_across_their_own_diagonal`].
    #[test]
    fn refining_the_grid_shrinks_the_largest_adjacent_cell_jump() {
        let mut examples = Vec::new();
        for row in 0..3u8 {
            for column in 0..3u8 {
                examples.push([f32::from(column), f32::from(row), 0.0]);
            }
        }
        let mut faces = Vec::new();
        for row in 0..2u8 {
            for column in 0..2u8 {
                let base = row * 3 + column;
                faces.push(face(&[base, base + 1, base + 4]));
                faces.push(face(&[base, base + 4, base + 3]));
            }
        }
        let jump = |cells: u8| {
            let grid = grid_under_test(&description(
                vec![dimension(0.0, 2.0, cells), dimension(0.0, 2.0, cells)],
                examples.clone(),
                Vec::new(),
                faces.clone(),
            ));
            let mut worst = 0.0f32;
            for index in 0..grid.cells.len() {
                let here = grid.cell_weights(index);
                for axis in 0..grid.axes() {
                    if let Some(next) = grid.neighbour(index, axis) {
                        worst = worst.max(l1_distance(&here, &grid.cell_weights(next)));
                    }
                }
            }
            worst
        };

        let coarse = jump(9);
        let fine = jump(17);
        let finer = jump(33);

        assert!(coarse > 0.0, "the coarse grid must move at all");
        assert!(
            fine <= coarse * 0.55,
            "halving the step left the jump at {fine} against {coarse}"
        );
        assert!(
            finer <= fine * 0.55,
            "halving the step again left the jump at {finer} against {fine}"
        );
    }

    /// The one place the rebuilt blend is genuinely discontinuous, and it is
    /// `CryEngine`'s own.
    ///
    /// `ComputeWeightExtrapolate4` keeps a triangle whenever the sample is on
    /// or behind its edge plane - `if ((plane | p) <= 0)`,
    /// `ParametricSampler.cpp:1636` - and the four triangles are split on the
    /// quad's two diagonals. A sample sitting exactly on a diagonal therefore
    /// satisfies three of the four tests instead of two, and the trailing
    /// normalisation by the sum of the accumulated weights
    /// (`ParametricSampler.cpp:1657`) divides by three rather than two. The
    /// result is a step, not a kink: neither side's limit equals the value on
    /// the diagonal, and refining the grid moves the step rather than shrinking
    /// it.
    ///
    /// This is documented rather than repaired because changing the plane test
    /// would diverge from `ComputeWeightExtrapolate4`.
    #[test]
    fn quad_weights_step_across_their_own_diagonal() {
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        // On the `y == x` diagonal three triangles contribute and the sum is
        // three, giving `((3 - 4x) / 3, x / 3, 2x / 3, x / 3)`.
        let on = weight_extrapolate_4([0.25, 0.25], corners);
        // A hair below it only two contribute and the sum is two, giving
        // `(1 - x - y / 2, x - y / 2, y / 2, y / 2)`.
        let below = weight_extrapolate_4([0.25, 0.25 - 1.0e-5], corners);

        for (label, weights) in [("on", on), ("below", below)] {
            let sum: f32 = weights.iter().sum();
            assert!(
                (sum - 1.0).abs() <= 1.0e-6,
                "{label} weights sum to {sum}, not to one"
            );
        }
        for (slot, expected) in [
            (0, 2.0 / 3.0),
            (1, 1.0 / 12.0),
            (2, 1.0 / 6.0),
            (3, 1.0 / 12.0),
        ] {
            assert!(
                (on[slot] - expected).abs() <= 1.0e-5,
                "on-diagonal slot {slot} is {}, not {expected}",
                on[slot]
            );
        }
        for (slot, expected) in [(0, 0.625), (1, 0.125), (2, 0.125), (3, 0.125)] {
            assert!(
                (below[slot] - expected).abs() <= 1.0e-4,
                "below-diagonal slot {slot} is {}, not {expected}",
                below[slot]
            );
        }
        let step: f32 = l1_distance(&on, &below);
        assert!(
            (step - 1.0 / 6.0).abs() <= 1.0e-4,
            "the diagonal step is {step}, not one sixth"
        );
    }

    /// Sweeps every `.bspace.ron` under the authoring root named by
    /// `AZ_ANIMATION_BLEND_SPACE_CORPUS`, rebuilds each grid from the example
    /// parameters, checks the invariants above over every cell, and - where the
    /// authored `<VGrid>` is present and correctly sized, so `ReadVGrid` would
    /// have kept it (`GlobalAnimationHeaderLMG.cpp:126-130`) - measures how far
    /// the rebuilt grid drifts from that cache.
    ///
    /// The corpus is not part of this repository, so the sweep is a no-op
    /// unless the variable is set. Run it with `--nocapture` to see the
    /// measurements.
    #[expect(
        clippy::too_many_lines,
        reason = "one sweep over one corpus: splitting the accumulators from the report they \
                  print would hide which measurement each number came from"
    )]
    #[test]
    fn authoring_corpus_computed_grids_hold_the_grid_invariants() {
        use crate::blend_space_asset::{
            BlendSpaceSource, read_blend_space_asset, transform_blend_space_product_at_root,
        };
        use std::path::PathBuf;

        let Some(root) = std::env::var_os("AZ_ANIMATION_BLEND_SPACE_CORPUS") else {
            return;
        };
        let root = PathBuf::from(root);

        let mut sources = Vec::new();
        collect_blend_space_sources(&root, &mut sources);
        sources.sort();
        assert!(
            !sources.is_empty(),
            "no .bspace.ron under {}",
            root.display()
        );

        let mut checked = 0usize;
        let mut skipped = 0usize;
        let mut cached_files = 0usize;
        let mut cached_cells = 0usize;
        let mut differing_cells = 0usize;
        let mut differing_sets = 0usize;
        let mut visible_cells = 0usize;
        let mut max_deviation = 0.0f32;
        let mut total_deviation = 0.0f64;
        let mut deviation_samples = 0usize;
        let mut worst = GridReport::default();
        let mut worst_file = String::new();
        let mut computed_control = ControlReport::default();
        let mut authored_control = ControlReport::default();

        for source_path in &sources {
            let relative = source_path
                .strip_prefix(&root)
                .unwrap_or(source_path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(bytes) = std::fs::read(source_path) else {
                skipped += 1;
                continue;
            };
            let Ok(build) = transform_blend_space_product_at_root(&root, &relative, &bytes) else {
                skipped += 1;
                continue;
            };
            let Ok(asset) = read_blend_space_asset(&build.product.bytes) else {
                skipped += 1;
                continue;
            };
            let Ok(source) = BlendSpaceSource::from_ron_bytes(&bytes) else {
                skipped += 1;
                continue;
            };

            let compiled = ParametricBlendSpaceDescription::from(asset.sampler);
            let expected_cells = compiled.dimensions.iter().fold(1usize, |size, dimension| {
                size * usize::from(dimension.cells)
            });
            let grid = grid_under_test(&compiled);
            let report = grid.check();
            if report.worst_continuity_ratio > worst.worst_continuity_ratio {
                worst_file = relative.clone();
            }
            worst.merge(report);
            computed_control.merge(grid.measure_any(&grid.cells));
            checked += 1;

            // `ReadVGrid` keeps the authored grid only when its entry count
            // matches the product of the cell counts, so only those files carry
            // a cache to compare the rebuild against.
            if source.blend_space.virtual_examples.len() != expected_cells {
                continue;
            }
            cached_files += 1;
            // The control column measures the same invariants over the authored
            // grid at the same coordinates.
            authored_control.merge(grid.measure_any(&compiled.virtual_examples));
            for (index, authored) in compiled.virtual_examples.iter().enumerate() {
                let mut cached = vec![0.0f32; grid.example_count];
                for (&example, &weight) in authored.indices.iter().zip(&authored.weights) {
                    if weight != 0.0 {
                        cached[usize::from(example)] += weight;
                    }
                }
                let computed = grid.cell_weights(index);
                let mut cell_max = 0.0f32;
                for (cached, computed) in cached.iter().zip(&computed) {
                    let deviation = (cached - computed).abs();
                    cell_max = cell_max.max(deviation);
                    total_deviation += f64::from(deviation);
                    deviation_samples += 1;
                }
                cached_cells += 1;
                max_deviation = max_deviation.max(cell_max);
                if cell_max > 0.0 {
                    differing_cells += 1;
                }
                // "Visible" is a two-percent weight shift on some example: a
                // blend that far off would read as a different pose.
                if cell_max > 0.02 {
                    visible_cells += 1;
                }
                let set = |weights: &[f32]| -> BTreeSet<usize> {
                    weights
                        .iter()
                        .enumerate()
                        .filter(|(_, weight)| weight.abs() > 1e-6)
                        .map(|(index, _)| index)
                        .collect()
                };
                if set(&cached) != set(&computed) {
                    differing_sets += 1;
                }
            }
        }

        println!("--- blend-space corpus sweep ---");
        println!(
            "files {} checked {checked} skipped {skipped}",
            sources.len()
        );
        println!(
            "cells {} empty {} degenerate {} unaccepted {} extrapolated {} widened {} \
             widest-tolerance {}",
            worst.cells,
            worst.empty_cells,
            worst.degenerate_cells,
            worst.unaccepted_cells,
            worst.extrapolated_cells,
            worst.widened_cells,
            worst.widest_tolerance
        );
        println!(
            "worst partition error {} reconstruction error {} continuity ratio {} \
             outside-hull jump {} ({worst_file})",
            worst.worst_partition_error,
            worst.worst_reconstruction_error,
            worst.worst_continuity_ratio,
            worst.worst_outside_jump
        );
        for (label, control) in [
            ("computed", computed_control),
            ("authored", authored_control),
        ] {
            println!(
                "{label} control: cells {} empty {} non-finite {} out-of-range {} \
                 partition {} weight [{}, {}] reconstruction {} jump {}",
                control.cells,
                control.empty_cells,
                control.non_finite_cells,
                control.out_of_range_cells,
                control.worst_partition_error,
                control.weight_low,
                control.weight_high,
                control.worst_reconstruction_error,
                control.worst_jump
            );
        }
        println!(
            "authored-cache files {cached_files} cells {cached_cells} differing {differing_cells} \
             differing-example-set {differing_sets} visible(>0.02) {visible_cells}"
        );
        let samples = f64::from(u32::try_from(deviation_samples).unwrap_or(u32::MAX)).max(1.0);
        println!(
            "weight deviation max {max_deviation} mean {}",
            total_deviation / samples
        );

        // A non-finite or out-of-range grid weight is fatal: it is what
        // `ParametricBlendSpace::try_from` refuses to load, so any file the
        // rebuild has to serve would fail to build.
        assert_eq!(
            computed_control.non_finite_cells, 0,
            "the rebuilt grids hold non-finite weights"
        );
        assert_eq!(
            computed_control.out_of_range_cells, 0,
            "the rebuilt grids name examples the spaces do not have"
        );

        // The per-cell invariants assert inside `check`. What is left to state
        // is that the rebuild is no further from a partition of unity, and no
        // further from reproducing each cell's own coordinate than the authored
        // cache is over the same inputs.
        assert!(
            computed_control.worst_partition_error
                <= authored_control.worst_partition_error + 1.0e-6,
            "rebuilt partition error {} exceeds the authored grids' {}",
            computed_control.worst_partition_error,
            authored_control.worst_partition_error
        );
        assert!(
            computed_control.worst_reconstruction_error
                <= authored_control.worst_reconstruction_error + 1.0e-6,
            "rebuilt reconstruction error {} exceeds the authored grids' {}",
            computed_control.worst_reconstruction_error,
            authored_control.worst_reconstruction_error
        );
    }
}
