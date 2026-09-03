//! Typed character-definition authoring and runtime products.

use std::sync::Arc;

use az_asset_builder::{ProductFormat, SourceFormat};
use az_core::{AssetData, AssetId, AssetPathBuf, AzRtti, AzTypeInfo};
use bevy_asset::Asset;
use bevy_math::{Quat, Vec2, Vec3, Vec4};
use bevy_reflect::TypePath;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use uuid::{Uuid, uuid};

pub const CHARACTER_DEFINITION_SOURCE_SCHEMA_NAME: &str =
    "azoth.animation.CharacterDefinitionSource";
pub const CHARACTER_DEFINITION_PRODUCT_SUB_ID: u32 = 1;
pub const CHARACTER_DEFINITION_PRODUCT_EXTENSION: &str = "azcharacter";
pub const INLINE_ATTACHMENT_COLLISION_PROXIES: usize = 10;

pub type CharacterDefinitionSource = CharacterDefinition<AssetPathBuf>;
pub type CharacterDefinitionProduct = CharacterDefinition<AssetId>;

/// The product class loaded by character instantiation systems.
#[derive(Asset, TypePath, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterDefinitionAsset {
    definition: CharacterDefinitionProduct,
}

impl CharacterDefinitionAsset {
    #[must_use]
    pub const fn new(definition: CharacterDefinitionProduct) -> Self {
        Self { definition }
    }

    #[must_use]
    pub const fn definition(&self) -> &CharacterDefinitionProduct {
        &self.definition
    }

    #[must_use]
    pub fn into_definition(self) -> CharacterDefinitionProduct {
        self.definition
    }
}

impl AsRef<CharacterDefinitionProduct> for CharacterDefinitionAsset {
    fn as_ref(&self) -> &CharacterDefinitionProduct {
        self.definition()
    }
}

impl AzTypeInfo for CharacterDefinitionAsset {
    const NAME: &'static str = "Azoth::Animation::CharacterDefinitionAsset";
    const TYPE_ID: Uuid = uuid!("70c383cf-95b7-4c70-9699-d53b6064dccb");
}

impl AzRtti for CharacterDefinitionAsset {}

impl AssetData for CharacterDefinitionAsset {
    const STABLE_NAME: &'static str = "azoth.animation.character-definition";
}

#[derive(SourceFormat)]
#[source(
    schema = "azoth.animation.CharacterDefinitionSource",
    ext = "character.ron"
)]
pub struct CharacterDefinitionSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.animation.character-definition",
    version = 1,
    asset = CharacterDefinitionAsset
)]
pub struct CharacterDefinitionProductFormat;

/// Reference families used while lowering paths into product identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharacterAssetKind {
    CharacterDefinition,
    CharacterParameters,
    Skeleton,
    StaticMesh,
    SkinnedMesh,
    Cloth,
    Material,
}

impl CharacterAssetKind {
    /// Primary product sub-id for each canonical authoring source family.
    #[must_use]
    pub const fn product_sub_id(self) -> u32 {
        match self {
            Self::CharacterDefinition
            | Self::CharacterParameters
            | Self::Skeleton
            | Self::StaticMesh
            | Self::SkinnedMesh
            | Self::Cloth
            | Self::Material => 1,
        }
    }
}

/// A character definition parameterized by authoring paths or cooked asset ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterDefinition<R> {
    pub model: R,
    pub parameters: Option<R>,
    pub material: Option<R>,
    pub keep_models_in_memory: bool,
    pub mirroring: CharacterMirroring,
    pub attachments: Vec<CharacterAttachment<R>>,
}

impl<R> CharacterDefinition<R> {
    /// Visits every asset reference without allocating an intermediate list.
    pub fn visit_assets(&self, mut visit: impl FnMut(CharacterAssetKind, &R)) {
        visit(CharacterAssetKind::Skeleton, &self.model);
        if let Some(parameters) = &self.parameters {
            visit(CharacterAssetKind::CharacterParameters, parameters);
        }
        if let Some(material) = &self.material {
            visit(CharacterAssetKind::Material, material);
        }
        for attachment in &self.attachments {
            attachment.visit_assets(&mut visit);
        }
    }

    /// Consumes the definition and lowers every reference in one pass.
    ///
    /// # Errors
    ///
    /// Returns the first `E` produced by `map`, which is called once per
    /// skeleton, character-parameter, material, and attachment reference.
    pub fn try_map_assets<S, E>(
        self,
        mut map: impl FnMut(CharacterAssetKind, R) -> Result<S, E>,
    ) -> Result<CharacterDefinition<S>, E> {
        let model = map(CharacterAssetKind::Skeleton, self.model)?;
        let parameters = match self.parameters {
            Some(value) => Some(map(CharacterAssetKind::CharacterParameters, value)?),
            None => None,
        };
        let material = match self.material {
            Some(value) => Some(map(CharacterAssetKind::Material, value)?),
            None => None,
        };
        let attachments = self
            .attachments
            .into_iter()
            .map(|attachment| attachment.try_map_assets(&mut map))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CharacterDefinition {
            model,
            parameters,
            material,
            keep_models_in_memory: self.keep_models_in_memory,
            mirroring: self.mirroring,
            attachments,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterMirroring {
    pub axis: Option<MirroringAxis>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirroringAxis {
    X,
    Y,
    Z,
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct AttachmentFlags: u32 {
        const HIDDEN = 0x01;
        const PHYSICALIZED_RAYS = 0x02;
        const PHYSICALIZED_COLLISIONS = 0x04;
        const SOFTWARE_SKINNING = 0x08;
        const RENDER_ONLY_EXISTING_LOD = 0x10;
        const LINEAR_SKINNING = 0x20;
        const MATRIX_SKINNING = 0x40;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterAttachment<R> {
    pub name: Option<Arc<str>>,
    pub flags: AttachmentFlags,
    pub absolute: AttachmentTransform,
    pub relative: RelativeAttachmentTransform,
    pub binding: Option<AttachmentBinding<R>>,
    pub materials: AttachmentMaterials<R>,
    pub kind: CharacterAttachmentKind,
}

impl<R> CharacterAttachment<R> {
    fn visit_assets(&self, visit: &mut impl FnMut(CharacterAssetKind, &R)) {
        if let Some(binding) = &self.binding {
            binding.visit_asset(visit);
        }
        self.materials.visit_assets(visit);
    }

    fn try_map_assets<S, E>(
        self,
        map: &mut impl FnMut(CharacterAssetKind, R) -> Result<S, E>,
    ) -> Result<CharacterAttachment<S>, E> {
        Ok(CharacterAttachment {
            name: self.name,
            flags: self.flags,
            absolute: self.absolute,
            relative: self.relative,
            binding: match self.binding {
                Some(binding) => Some(binding.try_map_asset(map)?),
                None => None,
            },
            materials: self.materials.try_map_assets(map)?,
            kind: self.kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttachmentTransform {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl Default for AttachmentTransform {
    fn default() -> Self {
        Self {
            rotation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RelativeAttachmentTransform {
    pub rotation: Option<Quat>,
    pub translation: Option<Vec3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentBinding<R> {
    Character(R),
    StaticMesh(R),
    SkinnedMesh(R),
    Cloth(R),
}

impl<R> AttachmentBinding<R> {
    #[must_use]
    pub const fn asset(&self) -> &R {
        match self {
            Self::Character(asset)
            | Self::StaticMesh(asset)
            | Self::SkinnedMesh(asset)
            | Self::Cloth(asset) => asset,
        }
    }

    #[must_use]
    pub const fn asset_kind(&self) -> CharacterAssetKind {
        match self {
            Self::Character(_) => CharacterAssetKind::CharacterDefinition,
            Self::StaticMesh(_) => CharacterAssetKind::StaticMesh,
            Self::SkinnedMesh(_) => CharacterAssetKind::SkinnedMesh,
            Self::Cloth(_) => CharacterAssetKind::Cloth,
        }
    }

    fn visit_asset(&self, visit: &mut impl FnMut(CharacterAssetKind, &R)) {
        visit(self.asset_kind(), self.asset());
    }

    fn try_map_asset<S, E>(
        self,
        map: &mut impl FnMut(CharacterAssetKind, R) -> Result<S, E>,
    ) -> Result<AttachmentBinding<S>, E> {
        Ok(match self {
            Self::Character(asset) => {
                AttachmentBinding::Character(map(CharacterAssetKind::CharacterDefinition, asset)?)
            }
            Self::StaticMesh(asset) => {
                AttachmentBinding::StaticMesh(map(CharacterAssetKind::StaticMesh, asset)?)
            }
            Self::SkinnedMesh(asset) => {
                AttachmentBinding::SkinnedMesh(map(CharacterAssetKind::SkinnedMesh, asset)?)
            }
            Self::Cloth(asset) => AttachmentBinding::Cloth(map(CharacterAssetKind::Cloth, asset)?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMaterials<R> {
    pub shared: Option<R>,
    pub lods: [Option<R>; 6],
}

impl<R> Default for AttachmentMaterials<R> {
    fn default() -> Self {
        Self {
            shared: None,
            lods: std::array::from_fn(|_| None),
        }
    }
}

impl<R> AttachmentMaterials<R> {
    fn visit_assets(&self, visit: &mut impl FnMut(CharacterAssetKind, &R)) {
        if let Some(material) = &self.shared {
            visit(CharacterAssetKind::Material, material);
        }
        for material in self.lods.iter().flatten() {
            visit(CharacterAssetKind::Material, material);
        }
    }

    fn try_map_assets<S, E>(
        self,
        map: &mut impl FnMut(CharacterAssetKind, R) -> Result<S, E>,
    ) -> Result<AttachmentMaterials<S>, E> {
        let shared = match self.shared {
            Some(material) => Some(map(CharacterAssetKind::Material, material)?),
            None => None,
        };
        let mut mapped = Vec::with_capacity(self.lods.len());
        for material in self.lods {
            mapped.push(match material {
                Some(material) => Some(map(CharacterAssetKind::Material, material)?),
                None => None,
            });
        }
        let lods = mapped
            .try_into()
            .unwrap_or_else(|_| unreachable!("six input material LODs produce six output LODs"));
        Ok(AttachmentMaterials { shared, lods })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CharacterAttachmentKind {
    Bone(BoneAttachment),
    Face(FaceAttachment),
    Skin(SkinAttachment),
    Proxy(ProxyAttachment),
    PendulumRow(PendulumRowAttachment),
    Cloth(ClothAttachment),
    ClothCollision(ClothCollisionAttachment),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BoneAttachment {
    pub joint: Option<Arc<str>>,
    pub simulation: Option<SocketSimulation>,
    pub procedural_function: Option<Arc<str>>,
    pub physics_lods: [Option<JointPhysics>; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceAttachment;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinAttachment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAttachment {
    pub joint: Option<Arc<str>>,
    pub parameters: Vec4,
    pub purpose: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendulumRowAttachment {
    pub row_joint: Option<Arc<str>>,
    pub simulation: Option<RowSimulation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClothAttachment {
    pub hidden: bool,
    pub collision_layer_mask: u32,
    pub max_simulation_distance: f32,
    pub local_wind: Vec3,
}

impl Default for ClothAttachment {
    fn default() -> Self {
        Self {
            hidden: false,
            collision_layer_mask: 1,
            max_simulation_distance: 3.0,
            local_wind: Vec3::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothCollisionAttachment {
    pub joint: Option<Arc<str>>,
    pub parameters: Vec4,
    pub collision_layer: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketSimulation {
    pub constraint: SocketConstraint,
    pub redirect: bool,
    pub frames_per_second: u8,
    pub max_angle_degrees: f32,
    pub radius: f32,
    pub sphere_scale: Vec2,
    pub disk_rotation_degrees: Vec2,
    pub mass: f32,
    pub gravity: f32,
    pub damping: f32,
    pub stiffness: f32,
    pub pivot_offset: Vec3,
    pub simulation_axis: Vec3,
    pub stiffness_target: Vec3,
    pub capsule: Vec2,
    pub projection_type: i32,
    pub directional_translation_joint: Option<Arc<str>>,
    pub collision_proxies: SmallVec<[Arc<str>; INLINE_ATTACHMENT_COLLISION_PROXIES]>,
}

impl Default for SocketSimulation {
    fn default() -> Self {
        Self {
            constraint: SocketConstraint::Disabled,
            redirect: false,
            frames_per_second: 10,
            max_angle_degrees: 45.0,
            radius: 0.5,
            sphere_scale: Vec2::ONE,
            disk_rotation_degrees: Vec2::ZERO,
            mass: 1.0,
            gravity: 9.81,
            damping: 1.0,
            stiffness: 0.0,
            pivot_offset: Vec3::ZERO,
            simulation_axis: Vec3::new(0.0, 0.5, 0.0),
            stiffness_target: Vec3::ZERO,
            capsule: Vec2::ZERO,
            projection_type: 0,
            directional_translation_joint: None,
            collision_proxies: SmallVec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SocketConstraint {
    #[default]
    Disabled = 0,
    PendulumCone = 1,
    PendulumHingePlane = 2,
    PendulumHalfCone = 3,
    SpringEllipsoid = 4,
    TranslationalProjection = 5,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowSimulation {
    pub constraint: RowConstraint,
    pub frames_per_second: u8,
    pub cone_angle_degrees: f32,
    pub cone_rotation_degrees: Vec3,
    pub mass: f32,
    pub gravity: f32,
    pub damping: f32,
    pub joint_spring: f32,
    pub rod_length: f32,
    pub stiffness_target: Vec2,
    pub turbulence: Vec2,
    pub max_velocity: f32,
    pub world_space_damping: Vec3,
    pub cycle: bool,
    pub stretch: f32,
    pub relaxation_loops: u32,
    pub capsule: Vec2,
    pub projection_type: i32,
    pub collision_proxies: SmallVec<[Arc<str>; INLINE_ATTACHMENT_COLLISION_PROXIES]>,
}

impl Default for RowSimulation {
    fn default() -> Self {
        Self {
            constraint: RowConstraint::PendulumCone,
            frames_per_second: 10,
            cone_angle_degrees: 45.0,
            cone_rotation_degrees: Vec3::ZERO,
            mass: 1.0,
            gravity: 9.81,
            damping: 1.0,
            joint_spring: 0.0,
            rod_length: 0.0,
            stiffness_target: Vec2::ZERO,
            turbulence: Vec2::new(0.5, 0.0),
            max_velocity: 8.0,
            world_space_damping: Vec3::ZERO,
            cycle: false,
            stretch: 0.1,
            relaxation_loops: 0,
            capsule: Vec2::ZERO,
            projection_type: 0,
            collision_proxies: SmallVec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RowConstraint {
    #[default]
    PendulumCone = 0,
    PendulumHingePlane = 1,
    PendulumHalfCone = 2,
    TranslationalProjection = 3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JointPhysics {
    Rope(RopeJointPhysics),
    Cloth(ClothJointPhysics),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool mirrors a distinct authored rope-physics toggle in the CDF schema"
)]
pub struct RopeJointPhysics {
    pub gravity: f32,
    pub joint_limit_degrees: f32,
    pub joint_limit_increase: f32,
    pub max_timestep: f32,
    pub stiffness_degrees: f32,
    pub stiffness_decay_degrees: f32,
    pub damping_degrees: f32,
    pub friction: f32,
    pub simple_blending: bool,
    pub mass: f32,
    pub thickness: f32,
    pub hinge_y: bool,
    pub hinge_z: bool,
    pub stiffness_control_bone: f32,
    pub environment_collisions: bool,
    pub body_collisions: bool,
}

impl Default for RopeJointPhysics {
    fn default() -> Self {
        Self {
            gravity: 0.0,
            joint_limit_degrees: 0.0,
            joint_limit_increase: 0.0,
            max_timestep: 0.02,
            stiffness_degrees: 0.001,
            stiffness_decay_degrees: 0.0,
            damping_degrees: 0.0,
            friction: 0.0,
            simple_blending: true,
            mass: 0.0,
            thickness: 0.0,
            hinge_y: false,
            hinge_z: false,
            stiffness_control_bone: 0.0,
            environment_collisions: true,
            body_collisions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothJointPhysics {
    pub max_timestep: f32,
    pub max_stretch: f32,
    pub stiffness_degrees: f32,
    pub thickness: f32,
    pub friction: f32,
    pub normal_stiffness_degrees: f32,
    pub tangential_stiffness_degrees: f32,
    pub damping: f32,
    pub air_resistance: f32,
    pub animation_stiffness: f32,
    pub animation_stiffness_decay: f32,
    pub animation_damping: f32,
    pub max_iterations: u32,
    pub max_animation_distance: f32,
    pub character_space: f32,
    pub environment_collisions: bool,
    pub body_collisions: bool,
}

impl Default for ClothJointPhysics {
    fn default() -> Self {
        Self {
            max_timestep: 0.0,
            max_stretch: 0.0,
            stiffness_degrees: 0.0,
            thickness: 0.0,
            friction: 0.0,
            normal_stiffness_degrees: 0.0,
            tangential_stiffness_degrees: 0.0,
            damping: 0.0,
            air_resistance: 0.0,
            animation_stiffness: 0.0,
            animation_stiffness_decay: 0.0,
            animation_damping: 0.0,
            max_iterations: 0,
            max_animation_distance: 0.0,
            character_space: 0.0,
            environment_collisions: true,
            body_collisions: true,
        }
    }
}

/// Encodes a cooked character definition and writes it to `writer`.
///
/// # Errors
///
/// Returns [`CharacterDefinitionCodecError::Codec`] when postcard fails to
/// serialize `asset`, and [`CharacterDefinitionCodecError::Write`] when the
/// writer rejects the encoded bytes.
pub fn write_character_definition(
    asset: &CharacterDefinitionAsset,
    writer: &mut impl std::io::Write,
) -> Result<(), CharacterDefinitionCodecError> {
    writer.write_all(&postcard::to_allocvec(asset)?)?;
    Ok(())
}

/// Decodes a cooked character definition from its postcard representation.
///
/// # Errors
///
/// Returns [`CharacterDefinitionCodecError::Codec`] when `bytes` is not a valid
/// postcard encoding of a [`CharacterDefinitionAsset`].
pub fn read_character_definition(
    bytes: &[u8],
) -> Result<CharacterDefinitionAsset, CharacterDefinitionCodecError> {
    Ok(postcard::from_bytes(bytes)?)
}

#[derive(Debug, thiserror::Error)]
pub enum CharacterDefinitionCodecError {
    #[error("encode or decode character definition: {0}")]
    Codec(#[from] postcard::Error),
    #[error("write character definition: {0}")]
    Write(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> AssetPathBuf {
        AssetPathBuf::new(value).unwrap()
    }

    #[test]
    fn maps_authoring_paths_to_runtime_ids_without_copying_shape_metadata() {
        let source = CharacterDefinition {
            model: path("characters/hero.skinnedmesh.glb"),
            parameters: Some(path("characters/hero.character-parameters.ron")),
            material: None,
            keep_models_in_memory: false,
            mirroring: CharacterMirroring::default(),
            attachments: vec![CharacterAttachment {
                name: Some(Arc::from("weapon")),
                flags: AttachmentFlags::empty(),
                absolute: AttachmentTransform::default(),
                relative: RelativeAttachmentTransform::default(),
                binding: Some(AttachmentBinding::StaticMesh(path(
                    "weapons/sword.staticmesh.glb",
                ))),
                materials: AttachmentMaterials::default(),
                kind: CharacterAttachmentKind::Bone(BoneAttachment::default()),
            }],
        };
        let mut sub_ids = Vec::new();
        let product = source
            .try_map_assets(|kind, _| {
                sub_ids.push(kind.product_sub_id());
                Ok::<_, std::convert::Infallible>(AssetId::new(Uuid::from_u128(7), 1))
            })
            .unwrap();

        assert_eq!(sub_ids, [1, 1, 1]);
        assert!(matches!(
            product.attachments[0].binding,
            Some(AttachmentBinding::StaticMesh(_))
        ));
    }

    #[test]
    fn product_codec_round_trips_typed_attachment_data() {
        let asset = CharacterDefinitionAsset::new(CharacterDefinition {
            model: AssetId::new(Uuid::from_u128(1), 1),
            parameters: None,
            material: None,
            keep_models_in_memory: true,
            mirroring: CharacterMirroring {
                axis: Some(MirroringAxis::X),
                enabled: true,
            },
            attachments: vec![CharacterAttachment {
                name: Some(Arc::from("cloth")),
                flags: AttachmentFlags::HIDDEN,
                absolute: AttachmentTransform::default(),
                relative: RelativeAttachmentTransform::default(),
                binding: Some(AttachmentBinding::Cloth(AssetId::new(
                    Uuid::from_u128(2),
                    1,
                ))),
                materials: AttachmentMaterials::default(),
                kind: CharacterAttachmentKind::Cloth(ClothAttachment::default()),
            }],
        });
        let mut bytes = Vec::new();

        write_character_definition(&asset, &mut bytes).unwrap();

        assert_eq!(read_character_definition(&bytes).unwrap(), asset);
    }
}
