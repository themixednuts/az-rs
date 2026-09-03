//! Rock'n'Roll shape products and their deterministic runtime codec.

use std::io::{Read, Write};

use az_core::{AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo};
use bevy::{
    asset::{Asset, io::Reader},
    math::{Vec3, Vec4},
    reflect::TypePath,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::{Uuid, uuid};

use crate::shape_codec;

pub const SHAPE_ASSET_VERSION: u32 = 1;
pub const SHAPE_ASSET_TYPE: AssetType =
    AssetType::new(uuid!("8b3c2d11-4a08-49e7-9c5f-22f1d6b3a4e0"));
pub const SHAPE_ASSET_STABLE_NAME: &str = "azoth.rock-n-roll.shape";

/// Extensions claimed by the Bevy product loader.
pub const SHAPE_ASSET_EXTENSIONS: &[&str] = &["rnr"];

pub type ShapeTransform = [Vec4; 3];

#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeAsset {
    pub version: u32,
    pub objects: Box<[ShapeObject]>,
    pub material_filter: MaterialFilter,
    pub shapes: Box<[PhysicalShape]>,
}

impl ShapeAsset {
    #[must_use]
    pub const fn new(
        objects: Box<[ShapeObject]>,
        material_filter: MaterialFilter,
        shapes: Box<[PhysicalShape]>,
    ) -> Self {
        Self {
            version: SHAPE_ASSET_VERSION,
            objects,
            material_filter,
            shapes,
        }
    }
}

impl AzTypeInfo for ShapeAsset {
    const NAME: &'static str = "RockNRoll::ShapeAsset";
    const TYPE_ID: Uuid = uuid!("8b3c2d11-4a08-49e7-9c5f-22f1d6b3a4e0");
}

impl AzRtti for ShapeAsset {}

impl AssetData for ShapeAsset {
    const STABLE_NAME: &'static str = SHAPE_ASSET_STABLE_NAME;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeObject {
    pub name: String,
    pub material_indices: Box<[u16]>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialFilter {
    pub enabled: bool,
    pub secondary_geometry: bool,
    pub indices: Box<[u16]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicalShape {
    pub data: ShapeData,
    pub extra: Box<[u8]>,
}

impl PhysicalShape {
    #[must_use]
    pub const fn new(data: ShapeData, extra: Box<[u8]>) -> Self {
        Self { data, extra }
    }

    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.data.kind()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum ShapeKind {
    Box = 1,
    Sphere = 2,
    ConvexHull = 3,
    Cylinder = 4,
    CylinderUnaligned = 5,
    Capsule = 6,
    CapsuleUnaligned = 7,
    Triangle = 8,
    Mesh = 10,
    Compound = 11,
    Transform = 12,
    SoftBody = 13,
    Plane = 17,
    ScaleConvexPolytope = 18,
    ScaleMesh = 19,
    HeightField = 20,
}

impl TryFrom<u32> for ShapeKind {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Box),
            2 => Ok(Self::Sphere),
            3 => Ok(Self::ConvexHull),
            4 => Ok(Self::Cylinder),
            5 => Ok(Self::CylinderUnaligned),
            6 => Ok(Self::Capsule),
            7 => Ok(Self::CapsuleUnaligned),
            8 => Ok(Self::Triangle),
            10 => Ok(Self::Mesh),
            11 => Ok(Self::Compound),
            12 => Ok(Self::Transform),
            13 => Ok(Self::SoftBody),
            17 => Ok(Self::Plane),
            18 => Ok(Self::ScaleConvexPolytope),
            19 => Ok(Self::ScaleMesh),
            20 => Ok(Self::HeightField),
            _ => Err(value),
        }
    }
}

impl From<ShapeKind> for u32 {
    fn from(value: ShapeKind) -> Self {
        value as Self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShapeData {
    Box(BoxShape),
    Sphere(SphereShape),
    ConvexHull(ConvexHullShape),
    Cylinder(CylinderShape),
    CylinderUnaligned(CylinderUnalignedShape),
    Capsule(CapsuleShape),
    CapsuleUnaligned(CapsuleUnalignedShape),
    Triangle(TriangleShape),
    Mesh(MeshShape),
    Compound(CompoundShape),
    Transform(TransformShape),
    SoftBody(SoftBodyShape),
    Plane(PlaneShape),
    ScaleConvexPolytope(ScaledShape),
    ScaleMesh(ScaledShape),
    HeightField(HeightFieldShape),
}

impl ShapeData {
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        match self {
            Self::Box(_) => ShapeKind::Box,
            Self::Sphere(_) => ShapeKind::Sphere,
            Self::ConvexHull(_) => ShapeKind::ConvexHull,
            Self::Cylinder(_) => ShapeKind::Cylinder,
            Self::CylinderUnaligned(_) => ShapeKind::CylinderUnaligned,
            Self::Capsule(_) => ShapeKind::Capsule,
            Self::CapsuleUnaligned(_) => ShapeKind::CapsuleUnaligned,
            Self::Triangle(_) => ShapeKind::Triangle,
            Self::Mesh(_) => ShapeKind::Mesh,
            Self::Compound(_) => ShapeKind::Compound,
            Self::Transform(_) => ShapeKind::Transform,
            Self::SoftBody(_) => ShapeKind::SoftBody,
            Self::Plane(_) => ShapeKind::Plane,
            Self::ScaleConvexPolytope(_) => ShapeKind::ScaleConvexPolytope,
            Self::ScaleMesh(_) => ShapeKind::ScaleMesh,
            Self::HeightField(_) => ShapeKind::HeightField,
        }
    }
}

/// Kind 13 carries no shape payload. Deformable vertices, faces, and links live
/// on the linked soft body instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftBodyShape;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BoxShape {
    pub half_extents: Vec3,
    pub convex_radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SphereShape {
    pub radius: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConvexHullShape {
    pub vertices: Box<[Vec3]>,
    pub planes: Box<[Vec4]>,
    pub convex_radius: f32,
    pub extra: Option<ConvexHullExtra>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvexHullExtra {
    pub data_a: Box<[u16]>,
    pub data_b: Box<[u16]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CylinderShape {
    pub half_height: f32,
    pub radius: f32,
    pub convex_radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CylinderUnalignedShape {
    pub endpoint_a: Vec3,
    pub endpoint_b: Vec3,
    pub radius: f32,
    pub convex_radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShape {
    pub half_height: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CapsuleUnalignedShape {
    pub endpoint_a: Vec3,
    pub endpoint_b: Vec3,
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TriangleShape {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub convex_radius: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshShape {
    pub stream_header: u32,
    pub vertices: Box<[Vec3]>,
    pub indices: Box<[u16]>,
    pub adjacent_triangles: Option<Box<[u16]>>,
    pub bvh: BvhTree,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompoundShape {
    pub children: Box<[CompoundChild]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundChild {
    pub transform: ShapeTransform,
    pub shape: Box<PhysicalShape>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformShape {
    pub transform: ShapeTransform,
    pub shape: Box<PhysicalShape>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaneShape {
    pub plane: Vec4,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaledShape {
    pub stream_header: u32,
    pub scale: Vec3,
    pub shape: Box<PhysicalShape>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeightFieldShape {
    pub layout: u32,
    pub data: Option<HeightFieldData>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeightFieldData {
    pub version: u32,
    pub width: u32,
    pub length: u32,
    pub height_scale: f32,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    pub samples: Box<[u8]>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BvhTree {
    pub version: u32,
    pub parts: BvhTreeParts,
    pub payload: Box<[u8]>,
}

impl BvhTree {
    #[must_use]
    pub fn quantized_nodes(&self) -> &[u8] {
        slice_part(
            &self.payload,
            self.parts.quantized_nodes_offset,
            self.parts.quantized_node_count as usize,
            16,
        )
    }

    #[must_use]
    pub fn subtree_infos(&self) -> &[u8] {
        slice_part(
            &self.payload,
            self.parts.subtree_infos_offset,
            self.parts.subtree_info_count as usize,
            32,
        )
    }

    #[must_use]
    pub fn triangle_index_map(&self) -> &[u8] {
        let stride = if self.parts.flags & 2 == 0 { 4 } else { 2 };
        slice_part(
            &self.payload,
            self.parts.triangle_index_map_offset,
            self.parts.triangle_index_count as usize,
            stride,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BvhTreeParts {
    pub quantized_nodes_offset: u32,
    pub subtree_infos_offset: u32,
    pub triangle_index_map_offset: u32,
    pub quantized_node_count: u32,
    pub subtree_info_count: u16,
    pub triangle_index_count: u32,
    pub flags: u16,
}

/// Write a `RockNRoll` shape asset in its binary product format.
///
/// # Errors
///
/// Returns [`ShapeAssetFormatError::TooManyItems`] if a shape, vertex, index or
/// material list is longer than `u32::MAX`, or
/// [`ShapeAssetFormatError::Io`] if `writer` rejects a write.
pub fn write_shape_asset(
    asset: &ShapeAsset,
    writer: impl Write,
) -> Result<(), ShapeAssetFormatError> {
    shape_codec::write_shape_asset(asset, writer)
}

/// Read a `RockNRoll` shape asset from an in-memory product buffer.
///
/// # Errors
///
/// Returns [`ShapeAssetFormatError::BadMagic`] or
/// [`ShapeAssetFormatError::UnsupportedVersion`] if the header does not match
/// this format, [`ShapeAssetFormatError::UnknownShapeKind`] for an unrecognized
/// shape discriminant, [`ShapeAssetFormatError::InvalidData`] if a decoded
/// record is internally inconsistent, and [`ShapeAssetFormatError::Io`] if
/// `bytes` ends mid-record.
pub fn read_shape_asset(bytes: &[u8]) -> Result<ShapeAsset, ShapeAssetFormatError> {
    shape_codec::read_shape_asset(bytes)
}

/// Read a `RockNRoll` shape asset from a stream.
///
/// # Errors
///
/// Returns any error [`read_shape_asset`] returns.
pub fn read_shape_asset_from_reader(
    reader: impl Read,
) -> Result<ShapeAsset, ShapeAssetFormatError> {
    shape_codec::read_shape_asset_from_reader(reader)
}

#[derive(Debug, Error)]
pub enum ShapeAssetFormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad RockNRoll shape asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported RockNRoll shape asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid RockNRoll shape asset data: {0}")]
    InvalidData(&'static str),
    #[error("unknown RockNRoll shape kind {kind}")]
    UnknownShapeKind { kind: u32 },
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

fn slice_part(payload: &[u8], offset: u32, count: usize, stride: usize) -> &[u8] {
    let Some(start) = usize::try_from(offset).ok() else {
        return &[];
    };
    let Some(len) = count.checked_mul(stride) else {
        return &[];
    };
    payload.get(start..start + len).unwrap_or(&[])
}

/// Asynchronously loads a cooked Rock'n'Roll product into Bevy.
#[derive(Default, TypePath)]
pub struct ShapeAssetLoader;

impl bevy::asset::AssetLoader for ShapeAssetLoader {
    type Asset = ShapeAsset;
    type Settings = ();
    type Error = ShapeAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut bevy::asset::LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        read_shape_asset(&bytes)
    }

    fn extensions(&self) -> &[&str] {
        SHAPE_ASSET_EXTENSIONS
    }
}

/// The asset types this module owns, for the gem's contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<ShapeAsset>().with_owner("az-gem-rock-n-roll")]
}
