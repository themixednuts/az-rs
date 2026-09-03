//! Typed cloth authoring and runtime products.

mod builder;

use std::{borrow::Borrow, io};

use az_asset_builder::{ProductFormat, SourceFormat};
use az_core::{AssetData, AssetId, AssetPathBuf, AssetType, AzRtti, AzTypeInfo};
use bevy_asset::Asset;
use bevy_math::{Quat, Vec3};
use bevy_reflect::{Reflect, TypePath};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub use builder::{
    cloth_fabric_build_rule, cloth_fabric_product_path, cloth_material_build_rule,
    cloth_material_product_path,
};

pub const SOURCE_VERSION: u32 = 1;
pub const CLOTH_FABRIC_PRODUCT_SUB_ID: u32 = 1;
pub const CLOTH_MATERIAL_PRODUCT_SUB_ID: u32 = 1;
pub const CLOTH_FABRIC_PRODUCT_EXTENSION: &str = "azcloth";
pub const CLOTH_MATERIAL_PRODUCT_EXTENSION: &str = "azclothmaterial";
pub const CLOTH_FABRIC_ASSET_TYPE_ID: Uuid = uuid!("9A3CBAC2-EC59-4F6F-9FD7-6C6A323B6364");
pub const CLOTH_MATERIAL_ASSET_TYPE_ID: Uuid = uuid!("6ED8009C-E9D9-48D2-9AEF-CD6B90B796E5");

pub type ClothFabricSource = VersionedClothFabric<AssetPathBuf>;
pub type ClothFabricProduct = ClothFabric<AssetId>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionedClothFabric<R> {
    pub version: u32,
    pub fabric: ClothFabric<R>,
}

impl<R> VersionedClothFabric<R> {
    #[must_use]
    pub const fn new(fabric: ClothFabric<R>) -> Self {
        Self {
            version: SOURCE_VERSION,
            fabric,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothFabric<R> {
    pub render_model: R,
    pub material: ClothMaterialBinding<R>,
    pub mesh: ClothMesh,
    pub cooked: FabricCookedData,
}

impl<R> ClothFabric<R> {
    pub fn visit_assets(&self, mut visit: impl FnMut(ClothAssetKind, &R)) {
        visit(ClothAssetKind::SkinnedMesh, &self.render_model);
        if let ClothMaterialBinding::Asset(material) = &self.material {
            visit(ClothAssetKind::ClothMaterial, material);
        }
    }

    /// Rebuilds the fabric with every asset reference replaced by `map`'s
    /// result, keeping the mesh and cooked data untouched.
    ///
    /// # Errors
    ///
    /// Returns the first `E` produced by `map`, for either the skinned-mesh
    /// reference or the `.clothmaterial` reference. An embedded material has
    /// no reference to map and therefore never fails here.
    pub fn try_map_assets<S, E>(
        self,
        mut map: impl FnMut(ClothAssetKind, R) -> Result<S, E>,
    ) -> Result<ClothFabric<S>, E> {
        Ok(ClothFabric {
            render_model: map(ClothAssetKind::SkinnedMesh, self.render_model)?,
            material: match self.material {
                ClothMaterialBinding::Embedded(material) => {
                    ClothMaterialBinding::Embedded(material)
                }
                ClothMaterialBinding::Asset(asset) => {
                    ClothMaterialBinding::Asset(map(ClothAssetKind::ClothMaterial, asset)?)
                }
            },
            mesh: self.mesh,
            cooked: self.cooked,
        })
    }

    /// Validates structural invariants required by both cooking and simulation.
    ///
    /// # Errors
    ///
    /// Propagates the [`ClothValidationError`] reported by an embedded
    /// [`ClothMaterial`], by the mesh, or by the cooked fabric data, and
    /// returns [`ClothValidationError::TriangleTopologyMismatch`] when the
    /// mesh indices and the cooked triangle list disagree.
    pub fn validate(&self) -> Result<(), ClothValidationError> {
        if let ClothMaterialBinding::Embedded(material) = &self.material {
            material.validate()?;
        }
        self.mesh.validate()?;
        self.cooked.validate(self.mesh.vertices.len())?;
        if self.mesh.indices != self.cooked.triangles {
            return Err(ClothValidationError::TriangleTopologyMismatch);
        }
        Ok(())
    }
}

/// Selects exactly one material source for a cloth fabric.
///
/// Native `.cloth` files always contain a fixed-size material block, but when
/// they also name a `.clothmaterial` asset that block is inactive placeholder
/// storage. Representing both at once makes it possible to validate or consume
/// the wrong branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClothMaterialBinding<R> {
    Embedded(ClothMaterial),
    Asset(R),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClothAssetKind {
    SkinnedMesh,
    ClothMaterial,
}

impl ClothAssetKind {
    #[must_use]
    pub const fn product_sub_id(self) -> u32 {
        match self {
            Self::SkinnedMesh | Self::ClothMaterial => 1,
        }
    }
}

#[derive(Asset, TypePath, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothFabricAsset {
    fabric: ClothFabricProduct,
}

impl ClothFabricAsset {
    #[must_use]
    pub const fn new(fabric: ClothFabricProduct) -> Self {
        Self { fabric }
    }

    #[must_use]
    pub const fn fabric(&self) -> &ClothFabricProduct {
        &self.fabric
    }

    #[must_use]
    pub fn into_fabric(self) -> ClothFabricProduct {
        self.fabric
    }
}

impl AsRef<ClothFabricProduct> for ClothFabricAsset {
    fn as_ref(&self) -> &ClothFabricProduct {
        self.fabric()
    }
}

impl Borrow<ClothFabricProduct> for ClothFabricAsset {
    fn borrow(&self) -> &ClothFabricProduct {
        self.fabric()
    }
}

impl AzTypeInfo for ClothFabricAsset {
    const NAME: &'static str = "NvCloth::ClothAsset";
    const TYPE_ID: Uuid = CLOTH_FABRIC_ASSET_TYPE_ID;
}

impl AzRtti for ClothFabricAsset {}

impl AssetData for ClothFabricAsset {
    const STABLE_NAME: &'static str = "azoth.nvcloth.cloth-fabric";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothMaterialSource {
    pub version: u32,
    pub material: ClothMaterial,
}

impl ClothMaterialSource {
    #[must_use]
    pub const fn new(material: ClothMaterial) -> Self {
        Self {
            version: SOURCE_VERSION,
            material,
        }
    }
}

#[derive(Asset, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ClothMaterialAsset {
    material: ClothMaterial,
}

impl ClothMaterialAsset {
    #[must_use]
    pub const fn new(material: ClothMaterial) -> Self {
        Self { material }
    }

    #[must_use]
    pub const fn material(&self) -> &ClothMaterial {
        &self.material
    }

    #[must_use]
    pub const fn into_material(self) -> ClothMaterial {
        self.material
    }
}

impl AsRef<ClothMaterial> for ClothMaterialAsset {
    fn as_ref(&self) -> &ClothMaterial {
        self.material()
    }
}

impl Borrow<ClothMaterial> for ClothMaterialAsset {
    fn borrow(&self) -> &ClothMaterial {
        self.material()
    }
}

impl AzTypeInfo for ClothMaterialAsset {
    const NAME: &'static str = "NvCloth::ClothMaterialAsset";
    const TYPE_ID: Uuid = CLOTH_MATERIAL_ASSET_TYPE_ID;
}

impl AzRtti for ClothMaterialAsset {}

impl AssetData for ClothMaterialAsset {
    const STABLE_NAME: &'static str = "azoth.nvcloth.cloth-material";
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ClothMaterial {
    pub phase_configs: FabricPhaseConfigs,
    pub stiffness_frequency: f32,
    pub motion_constraints: MotionConstraintConfig,
    pub self_collision: SelfCollisionConfig,
    pub tether_constraints: TetherConstraintConfig,
    pub solver_frequency: f32,
    pub acceleration_filter_width: f32,
    pub continuous_collision: bool,
    pub damping: Vec3,
    pub linear_drag: Vec3,
    pub angular_drag: Vec3,
    pub linear_inertia: Vec3,
    pub angular_inertia: Vec3,
    pub centrifugal_inertia: Vec3,
}

impl ClothMaterial {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        phase_configs: FabricPhaseConfigs,
        stiffness_frequency: f32,
        motion_constraints: MotionConstraintConfig,
        self_collision: SelfCollisionConfig,
        tether_constraints: TetherConstraintConfig,
        solver_frequency: f32,
        acceleration_filter_width: f32,
        continuous_collision: bool,
        damping: Vec3,
        linear_drag: Vec3,
        angular_drag: Vec3,
        linear_inertia: Vec3,
        angular_inertia: Vec3,
        centrifugal_inertia: Vec3,
    ) -> Self {
        Self {
            phase_configs,
            stiffness_frequency,
            motion_constraints,
            self_collision,
            tether_constraints,
            solver_frequency,
            acceleration_filter_width,
            continuous_collision,
            damping,
            linear_drag,
            angular_drag,
            linear_inertia,
            angular_inertia,
            centrifugal_inertia,
        }
    }

    /// Validates that every material coefficient is finite and that the
    /// frequency-style scalars stay in their supported range.
    ///
    /// # Errors
    ///
    /// Returns [`ClothValidationError::NonFiniteMaterial`] if any scalar,
    /// vector or phase-config value is not finite, and
    /// [`ClothValidationError::InvalidMaterialFrequency`] if the stiffness
    /// frequency or acceleration filter width is negative, or the solver
    /// frequency is not strictly positive.
    pub fn validate(self) -> Result<(), ClothValidationError> {
        let scalars = [
            self.stiffness_frequency,
            self.motion_constraints.max_distance,
            self.motion_constraints.scale,
            self.motion_constraints.bias,
            self.motion_constraints.stiffness,
            self.self_collision.distance,
            self.self_collision.stiffness,
            self.tether_constraints.stiffness,
            self.tether_constraints.scale,
            self.solver_frequency,
            self.acceleration_filter_width,
        ];
        if scalars.into_iter().any(|value| !value.is_finite())
            || [
                self.damping,
                self.linear_drag,
                self.angular_drag,
                self.linear_inertia,
                self.angular_inertia,
                self.centrifugal_inertia,
            ]
            .into_iter()
            .any(|value| !value.is_finite())
            || self
                .phase_configs
                .iter()
                .flat_map(PhaseConfig::values)
                .any(|value| !value.is_finite())
        {
            return Err(ClothValidationError::NonFiniteMaterial);
        }
        if self.stiffness_frequency < 0.0
            || self.solver_frequency <= 0.0
            || self.acceleration_filter_width < 0.0
        {
            return Err(ClothValidationError::InvalidMaterialFrequency);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct FabricPhaseConfigs {
    pub horizontal: PhaseConfig,
    pub vertical: PhaseConfig,
    pub bending: PhaseConfig,
    pub shearing: PhaseConfig,
}

impl FabricPhaseConfigs {
    #[must_use]
    pub const fn new(
        horizontal: PhaseConfig,
        vertical: PhaseConfig,
        bending: PhaseConfig,
        shearing: PhaseConfig,
    ) -> Self {
        Self {
            horizontal,
            vertical,
            bending,
            shearing,
        }
    }

    pub fn iter(self) -> impl Iterator<Item = PhaseConfig> {
        [self.horizontal, self.vertical, self.bending, self.shearing].into_iter()
    }

    #[must_use]
    pub const fn for_type(self, phase_type: FabricPhaseType) -> PhaseConfig {
        match phase_type {
            FabricPhaseType::Horizontal => self.horizontal,
            FabricPhaseType::Vertical => self.vertical,
            FabricPhaseType::Bending => self.bending,
            FabricPhaseType::Shearing => self.shearing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct PhaseConfig {
    pub stiffness: f32,
    pub stiffness_multiplier: f32,
    pub compression_limit: f32,
    pub stretch_limit: f32,
}

impl PhaseConfig {
    #[must_use]
    pub const fn new(
        stiffness: f32,
        stiffness_multiplier: f32,
        compression_limit: f32,
        stretch_limit: f32,
    ) -> Self {
        Self {
            stiffness,
            stiffness_multiplier,
            compression_limit,
            stretch_limit,
        }
    }

    fn values(self) -> impl Iterator<Item = f32> {
        [
            self.stiffness,
            self.stiffness_multiplier,
            self.compression_limit,
            self.stretch_limit,
        ]
        .into_iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct MotionConstraintConfig {
    pub max_distance: f32,
    pub scale: f32,
    pub bias: f32,
    pub stiffness: f32,
}

impl MotionConstraintConfig {
    #[must_use]
    pub const fn new(max_distance: f32, scale: f32, bias: f32, stiffness: f32) -> Self {
        Self {
            max_distance,
            scale,
            bias,
            stiffness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct SelfCollisionConfig {
    pub distance: f32,
    pub stiffness: f32,
}

impl SelfCollisionConfig {
    #[must_use]
    pub const fn new(distance: f32, stiffness: f32) -> Self {
        Self {
            distance,
            stiffness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct TetherConstraintConfig {
    pub stiffness: f32,
    pub scale: f32,
}

impl TetherConstraintConfig {
    #[must_use]
    pub const fn new(stiffness: f32, scale: f32) -> Self {
        Self { stiffness, scale }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothMesh {
    pub vertices: Vec<ClothSimulationVertex>,
    pub indices: Vec<u32>,
    pub render_mapping: ClothRenderMapping,
    pub paint: ClothPaintMaps,
}

impl ClothMesh {
    fn validate(&self) -> Result<(), ClothValidationError> {
        if self.vertices.is_empty() {
            return Err(ClothValidationError::EmptyMesh);
        }
        validate_triangles(&self.indices, self.vertices.len())?;
        for (index, vertex) in self.vertices.iter().enumerate() {
            if !vertex.position.is_finite() || !vertex.tangent_frame.is_finite() {
                return Err(ClothValidationError::NonFiniteVertex { vertex: index });
            }
            let weight_sum = vertex
                .joint_weights
                .iter()
                .copied()
                .map(u16::from)
                .sum::<u16>();
            if weight_sum != u16::from(u8::MAX) {
                return Err(ClothValidationError::InvalidSkinWeights {
                    vertex: index,
                    sum: weight_sum,
                });
            }
        }
        self.render_mapping
            .validate(self.vertices.len(), self.indices.len() / 3)?;
        self.paint.validate(self.vertices.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClothSimulationVertex {
    pub position: Vec3,
    pub tangent_frame: Quat,
    pub joint_indices: [u16; 4],
    pub joint_weights: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClothRenderMapping {
    Direct {
        particle_indices: Vec<u32>,
    },
    Barycentric {
        entries: Vec<ClothSkinMapEntry>,
        ranges: Vec<ClothSkinMapRange>,
    },
}

impl ClothRenderMapping {
    fn validate(
        &self,
        particle_count: usize,
        triangle_count: usize,
    ) -> Result<(), ClothValidationError> {
        match self {
            Self::Direct { particle_indices } => {
                if particle_indices.len() != particle_count {
                    return Err(ClothValidationError::DirectMapLength {
                        expected: particle_count,
                        actual: particle_indices.len(),
                    });
                }
                if let Some((vertex, particle)) = particle_indices
                    .iter()
                    .copied()
                    .enumerate()
                    .find(|(_, particle)| *particle as usize >= particle_count)
                {
                    return Err(ClothValidationError::DirectMapParticle {
                        vertex,
                        particle,
                        particle_count,
                    });
                }
            }
            Self::Barycentric { entries, ranges } => {
                for (vertex, entry) in entries.iter().enumerate() {
                    if !entry.barycentric.is_finite() || !entry.height.is_finite() {
                        return Err(ClothValidationError::NonFiniteSkinMap { vertex });
                    }
                    let sum = entry.barycentric.element_sum();
                    if (sum - 1.0).abs() > 1.0e-3 {
                        return Err(ClothValidationError::InvalidBarycentricSum { vertex, sum });
                    }
                    if entry.triangle as usize >= triangle_count {
                        return Err(ClothValidationError::SkinMapTriangle {
                            vertex,
                            triangle: entry.triangle,
                            triangle_count,
                        });
                    }
                }
                let mut next = 0_usize;
                for range in ranges {
                    if range.first_vertex as usize != next {
                        return Err(ClothValidationError::NonContiguousSkinMapRanges);
                    }
                    next = next
                        .checked_add(range.vertex_count as usize)
                        .ok_or(ClothValidationError::NonContiguousSkinMapRanges)?;
                }
                if next != entries.len() {
                    return Err(ClothValidationError::SkinMapRangeCoverage {
                        entries: entries.len(),
                        covered: next,
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClothSkinMapEntry {
    pub barycentric: Vec3,
    pub height: f32,
    pub triangle: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClothSkinMapRange {
    pub first_vertex: u32,
    pub vertex_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothPaintMaps {
    pub motion_constraint_max_distances: Vec<f32>,
    pub backstop_offsets: Option<Vec<f32>>,
    pub backstop_radii: Option<Vec<f32>>,
}

impl ClothPaintMaps {
    fn validate(&self, particles: usize) -> Result<(), ClothValidationError> {
        validate_paint_map(
            "motion constraint max distance",
            &self.motion_constraint_max_distances,
            particles,
        )?;
        if let Some(values) = &self.backstop_offsets {
            validate_paint_map("backstop offset", values, particles)?;
        }
        if let Some(values) = &self.backstop_radii {
            validate_paint_map("backstop radius", values, particles)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabricCookedData {
    pub phase_indices: Vec<u32>,
    pub phase_types: Vec<FabricPhaseType>,
    pub sets: Vec<u32>,
    pub rest_values: Vec<f32>,
    pub stiffness_values: Vec<f32>,
    pub constraint_indices: Vec<u32>,
    pub anchors: Vec<u32>,
    pub tether_lengths: Vec<f32>,
    pub triangles: Vec<u32>,
}

impl FabricCookedData {
    fn validate(&self, particles: usize) -> Result<(), ClothValidationError> {
        if self.phase_indices.len() != self.phase_types.len() {
            return Err(ClothValidationError::PhaseLengthMismatch {
                indices: self.phase_indices.len(),
                types: self.phase_types.len(),
            });
        }
        if self.constraint_indices.len() != self.rest_values.len().saturating_mul(2) {
            return Err(ClothValidationError::ConstraintLengthMismatch {
                rest_values: self.rest_values.len(),
                indices: self.constraint_indices.len(),
            });
        }
        if !self.stiffness_values.is_empty()
            && self.stiffness_values.len() != self.rest_values.len()
        {
            return Err(ClothValidationError::StiffnessLengthMismatch {
                rest_values: self.rest_values.len(),
                stiffness_values: self.stiffness_values.len(),
            });
        }
        if self.anchors.len() != self.tether_lengths.len() {
            return Err(ClothValidationError::TetherLengthMismatch {
                anchors: self.anchors.len(),
                lengths: self.tether_lengths.len(),
            });
        }
        if self
            .constraint_indices
            .iter()
            .chain(&self.anchors)
            .any(|index| *index as usize >= particles)
        {
            return Err(ClothValidationError::FabricParticleIndex { particles });
        }
        if self
            .phase_indices
            .iter()
            .any(|phase| *phase as usize >= self.sets.len())
        {
            return Err(ClothValidationError::FabricPhaseIndex {
                sets: self.sets.len(),
            });
        }
        if self.sets.windows(2).any(|window| window[0] > window[1])
            || self
                .sets
                .last()
                .is_some_and(|last| *last as usize > self.rest_values.len())
        {
            return Err(ClothValidationError::InvalidFabricSets);
        }
        if self
            .rest_values
            .iter()
            .chain(&self.stiffness_values)
            .chain(&self.tether_lengths)
            .any(|value| !value.is_finite())
        {
            return Err(ClothValidationError::NonFiniteFabric);
        }
        validate_triangles(&self.triangles, particles)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum FabricPhaseType {
    Horizontal = 1,
    Vertical = 2,
    Bending = 3,
    Shearing = 4,
}

impl TryFrom<i32> for FabricPhaseType {
    type Error = ClothValidationError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Horizontal),
            2 => Ok(Self::Vertical),
            3 => Ok(Self::Bending),
            4 => Ok(Self::Shearing),
            value => Err(ClothValidationError::UnknownPhaseType { value }),
        }
    }
}

impl From<FabricPhaseType> for i32 {
    fn from(value: FabricPhaseType) -> Self {
        value as Self
    }
}

#[derive(SourceFormat)]
#[source(schema = "azoth.nvcloth.ClothFabricSource", ext = "cloth.ron")]
pub struct ClothFabricSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.nvcloth.cloth-fabric",
    version = 1,
    asset = ClothFabricAsset
)]
pub struct ClothFabricProductFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.nvcloth.ClothMaterialSource",
    ext = "clothmaterial.ron"
)]
pub struct ClothMaterialSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.nvcloth.cloth-material",
    version = 1,
    asset = ClothMaterialAsset
)]
pub struct ClothMaterialProductFormat;

#[derive(Debug, Error)]
pub enum ClothCodecError {
    #[error("encode cloth product: {0}")]
    Encode(#[from] postcard::Error),
    #[error("decode cloth product: {0}")]
    Decode(postcard::Error),
    #[error("write cloth product: {0}")]
    Write(#[from] io::Error),
}

/// Serialises a cloth fabric product to `writer` in the postcard product
/// encoding.
///
/// # Errors
///
/// Returns [`ClothCodecError::Encode`] if postcard cannot serialise `asset`,
/// and [`ClothCodecError::Write`] if `writer` fails while the encoded bytes
/// are written.
pub fn write_cloth_fabric(
    asset: &ClothFabricAsset,
    writer: impl io::Write,
) -> Result<(), ClothCodecError> {
    let mut writer = writer;
    writer.write_all(&postcard::to_allocvec(asset)?)?;
    Ok(())
}

/// Deserialises a cloth fabric product from postcard-encoded `bytes`.
///
/// # Errors
///
/// Returns [`ClothCodecError::Decode`] if `bytes` is not a valid postcard
/// encoding of a [`ClothFabricAsset`].
pub fn read_cloth_fabric(bytes: &[u8]) -> Result<ClothFabricAsset, ClothCodecError> {
    postcard::from_bytes(bytes).map_err(ClothCodecError::Decode)
}

/// Serialises a cloth material product to `writer` in the postcard product
/// encoding.
///
/// # Errors
///
/// Returns [`ClothCodecError::Encode`] if postcard cannot serialise `asset`,
/// and [`ClothCodecError::Write`] if `writer` fails while the encoded bytes
/// are written.
pub fn write_cloth_material(
    asset: &ClothMaterialAsset,
    writer: impl io::Write,
) -> Result<(), ClothCodecError> {
    let mut writer = writer;
    writer.write_all(&postcard::to_allocvec(asset)?)?;
    Ok(())
}

/// Deserialises a cloth material product from postcard-encoded `bytes`.
///
/// # Errors
///
/// Returns [`ClothCodecError::Decode`] if `bytes` is not a valid postcard
/// encoding of a [`ClothMaterialAsset`].
pub fn read_cloth_material(bytes: &[u8]) -> Result<ClothMaterialAsset, ClothCodecError> {
    postcard::from_bytes(bytes).map_err(ClothCodecError::Decode)
}

pub mod ids {
    use super::{AssetData, AssetType, ClothFabricAsset, ClothMaterialAsset};

    pub const CLOTH_FABRIC: AssetType = ClothFabricAsset::ASSET_TYPE;
    pub const CLOTH_MATERIAL: AssetType = ClothMaterialAsset::ASSET_TYPE;
}

pub mod source_schemas {
    use az_asset_builder::{SourceSchemaType, source_schema_type};

    use super::{ClothFabricSourceFormat, ClothMaterialSourceFormat};

    pub const CLOTH_FABRIC: SourceSchemaType = source_schema_type::<ClothFabricSourceFormat>();
    pub const CLOTH_MATERIAL: SourceSchemaType = source_schema_type::<ClothMaterialSourceFormat>();
}

fn validate_triangles(indices: &[u32], vertices: usize) -> Result<(), ClothValidationError> {
    if !indices.len().is_multiple_of(3) {
        return Err(ClothValidationError::TriangleIndexCount {
            indices: indices.len(),
        });
    }
    if let Some((slot, index)) = indices
        .iter()
        .copied()
        .enumerate()
        .find(|(_, index)| *index as usize >= vertices)
    {
        return Err(ClothValidationError::TriangleVertex {
            slot,
            index,
            vertices,
        });
    }
    Ok(())
}

fn validate_paint_map(
    name: &'static str,
    values: &[f32],
    particles: usize,
) -> Result<(), ClothValidationError> {
    if values.len() != particles {
        return Err(ClothValidationError::PaintMapLength {
            name,
            expected: particles,
            actual: values.len(),
        });
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(ClothValidationError::NonFinitePaintMap { name });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ClothValidationError {
    #[error("cloth mesh has no simulation vertices")]
    EmptyMesh,
    #[error("cloth material contains a non-finite value")]
    NonFiniteMaterial,
    #[error("cloth material frequencies must be finite and positive where required")]
    InvalidMaterialFrequency,
    #[error("cloth vertex {vertex} contains a non-finite position or tangent frame")]
    NonFiniteVertex { vertex: usize },
    #[error("cloth vertex {vertex} skin weights sum to {sum}, expected 255")]
    InvalidSkinWeights { vertex: usize, sum: u16 },
    #[error("triangle index count {indices} is not divisible by three")]
    TriangleIndexCount { indices: usize },
    #[error("triangle index slot {slot} references vertex {index}, but only {vertices} exist")]
    TriangleVertex {
        slot: usize,
        index: u32,
        vertices: usize,
    },
    #[error("the mesh and cooked-fabric triangle topology differ")]
    TriangleTopologyMismatch,
    #[error("direct render map has {actual} entries, expected {expected}")]
    DirectMapLength { expected: usize, actual: usize },
    #[error(
        "direct render vertex {vertex} references particle {particle}, but only {particle_count} exist"
    )]
    DirectMapParticle {
        vertex: usize,
        particle: u32,
        particle_count: usize,
    },
    #[error("barycentric render vertex {vertex} contains a non-finite value")]
    NonFiniteSkinMap { vertex: usize },
    #[error("barycentric render vertex {vertex} weights sum to {sum}, expected one")]
    InvalidBarycentricSum { vertex: usize, sum: f32 },
    #[error(
        "barycentric render vertex {vertex} references triangle {triangle}, but only {triangle_count} exist"
    )]
    SkinMapTriangle {
        vertex: usize,
        triangle: u32,
        triangle_count: usize,
    },
    #[error("barycentric render ranges are not contiguous")]
    NonContiguousSkinMapRanges,
    #[error("barycentric render ranges cover {covered} of {entries} entries")]
    SkinMapRangeCoverage { entries: usize, covered: usize },
    #[error("{name} paint map has {actual} values, expected {expected}")]
    PaintMapLength {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{name} paint map contains a non-finite value")]
    NonFinitePaintMap { name: &'static str },
    #[error("fabric has {indices} phase indices and {types} phase types")]
    PhaseLengthMismatch { indices: usize, types: usize },
    #[error("fabric has {rest_values} rest values but {indices} constraint indices")]
    ConstraintLengthMismatch { rest_values: usize, indices: usize },
    #[error("fabric has {rest_values} rest values but {stiffness_values} stiffness values")]
    StiffnessLengthMismatch {
        rest_values: usize,
        stiffness_values: usize,
    },
    #[error("fabric has {anchors} tether anchors but {lengths} tether lengths")]
    TetherLengthMismatch { anchors: usize, lengths: usize },
    #[error("fabric references a particle outside its {particles} particles")]
    FabricParticleIndex { particles: usize },
    #[error("fabric phase index references a missing set among {sets} sets")]
    FabricPhaseIndex { sets: usize },
    #[error("fabric constraint sets are not monotonic or exceed the constraint count")]
    InvalidFabricSets,
    #[error("fabric contains a non-finite constraint value")]
    NonFiniteFabric,
    #[error("fabric uses unknown phase type {value}")]
    UnknownPhaseType { value: i32 },
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The asset types this crate owns, for a composing host to register.
#[must_use]
pub const fn asset_types() -> [az_core::AssetTypeRegistration; 2] {
    [
        az_core::AssetTypeRegistration::for_asset::<ClothFabricAsset>().with_owner("az-nv-cloth"),
        az_core::AssetTypeRegistration::for_asset::<ClothMaterialAsset>().with_owner("az-nv-cloth"),
    ]
}

/// The product formats this crate owns, for a composing host to register.
#[must_use]
pub const fn product_formats() -> [az_asset_builder::ProductFormatRegistration; 2] {
    [
        az_asset_builder::ProductFormatRegistration::for_format::<ClothFabricProductFormat>(),
        az_asset_builder::ProductFormatRegistration::for_format::<ClothMaterialProductFormat>(),
    ]
}

/// The source schemas this crate owns, for a composing host to register.
#[must_use]
pub const fn source_schemas() -> [az_asset_builder::SourceSchemaRegistration; 2] {
    [
        az_asset_builder::SourceSchemaRegistration::for_source::<ClothFabricSourceFormat>()
            .with_category("Cloth")
            .with_editable_file("characters", &["cloth.ron"]),
        az_asset_builder::SourceSchemaRegistration::for_source::<ClothMaterialSourceFormat>()
            .with_category("Cloth")
            .with_editable_file("characters", &["clothmaterial.ron"]),
    ]
}

/// The build rules this crate owns, for a composing host to register.
#[must_use]
pub fn build_rules() -> [az_asset_builder::BuildRuleRegistration; 2] {
    [
        az_asset_builder::BuildRuleRegistration::new(
            builder::FABRIC_NAME,
            builder::FABRIC_BUILDER_ID,
            cloth_fabric_build_rule,
        ),
        az_asset_builder::BuildRuleRegistration::new(
            builder::MATERIAL_NAME,
            builder::MATERIAL_BUILDER_ID,
            cloth_material_build_rule,
        ),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<az_asset_builder::ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<az_asset_builder::SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<az_asset_builder::BuildRuleRegistration>()
        .register_many(build_rules());
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{
        BuildRuleRegistry, JobContext, composed_product_formats, composed_source_schemas,
    };
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    declare_caps!(ClothCaps:);

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.nv-cloth-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    /// This crate contributed the way a host's glue contributes it.
    struct Cloth;

    impl Contribution for Cloth {
        type Caps = ClothCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, ClothCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Cloth, ProductActivation::default())
            .expect("cloth registrations require no host capability");
        composer
    }

    #[test]
    fn current_asset_type_ids_are_owned_by_canonical_assets() {
        assert_eq!(
            ClothFabricAsset::TYPE_ID,
            uuid!("9A3CBAC2-EC59-4F6F-9FD7-6C6A323B6364")
        );
        assert_eq!(
            ClothMaterialAsset::TYPE_ID,
            uuid!("6ED8009C-E9D9-48D2-9AEF-CD6B90B796E5")
        );
    }

    #[test]
    fn every_family_reaches_the_composed_host() {
        let composer = composed();
        let registries = composer.registries();

        let types = az_core::composed_asset_types(registries)
            .into_iter()
            .map(|registration| registration.asset_type())
            .collect::<Vec<_>>();
        assert!(types.contains(&ClothFabricAsset::ASSET_TYPE));
        assert!(types.contains(&ClothMaterialAsset::ASSET_TYPE));

        let formats = composed_product_formats(registries)
            .into_iter()
            .map(|registration| registration.entry.id())
            .collect::<Vec<_>>();
        assert!(formats.contains(&<ClothFabricProductFormat as ProductFormat>::ID));
        assert!(formats.contains(&<ClothMaterialProductFormat as ProductFormat>::ID));

        let schemas = composed_source_schemas(registries)
            .into_iter()
            .map(|registration| registration.entry.schema_type())
            .collect::<Vec<_>>();
        for format in [
            <ClothFabricSourceFormat as SourceFormat>::SCHEMA,
            <ClothMaterialSourceFormat as SourceFormat>::SCHEMA,
        ] {
            let schema = format.expect("cloth source formats declare a schema");
            assert!(schemas.contains(&schema), "{schema} is not composed");
        }

        let rules = BuildRuleRegistry::compose(&JobContext::new(registries));
        let ids = rules.iter().map(|rule| rule.id).collect::<Vec<_>>();
        assert!(ids.contains(&builder::FABRIC_BUILDER_ID));
        assert!(ids.contains(&builder::MATERIAL_BUILDER_ID));
    }

    #[test]
    fn registration_identity_matches_the_rule_it_resolves() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }
}
