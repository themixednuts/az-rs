use az_core::AssetPathBuf;
use glam::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Material render domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialDomain {
    Surface,
    Decal,
    PostProcess,
}

/// Material transparency and composition mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    Opaque,
    Masked,
    Translucent,
    Additive,
}

/// Triangle face culling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CullMode {
    Back,
    Front,
    None,
}

/// Lighting model used by the material type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadingModel {
    Standard,
    Unlit,
}

/// RGBA material color value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaterialColor {
    pub rgba: Vec4,
}

/// Texture asset value used by material properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialTexture {
    pub asset: AssetPathBuf,
}

/// Closed set of material property value kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MaterialPropertyValue {
    Bool(bool),
    Int(i32),
    UInt(u32),
    Float(f32),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Color(MaterialColor),
    Texture(MaterialTexture),
    String(String),
}

/// One material-instance override of a property declared by its material type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialPropertyBinding {
    pub property: String,

    pub value: MaterialPropertyValue,
}

/// One property declared by a material type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialPropertyDefinition {
    pub id: String,

    pub display_name: String,

    pub description: String,

    pub default_value: MaterialPropertyValue,

    pub connection: Option<String>,

    pub enum_values: Vec<String>,
}

/// Display group for material type properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialPropertyGroup {
    pub id: String,

    pub display_name: String,

    pub description: String,

    pub properties: Vec<MaterialPropertyDefinition>,
}
