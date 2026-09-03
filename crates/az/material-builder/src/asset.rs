//! Decoded runtime shapes for compiled material products.
//!
//! Property values reuse the typed structs from `az_material::value`; naming
//! follows ADR 0020 (`MaterialTypeAsset` / `MaterialAsset`).

use az_material::{
    BlendMode, CullMode, MaterialDomain, MaterialPropertyBinding, MaterialPropertyGroup,
    ShadingModel,
};

/// Compiled material-type property table (`azoth.material.type`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::asset::Asset, bevy::reflect::TypePath))]
pub struct MaterialTypeAsset {
    pub name: String,
    pub description: String,
    pub domain: MaterialDomain,
    pub blend_mode: BlendMode,
    pub cull_mode: CullMode,
    pub shading_model: ShadingModel,
    /// Authored shader-graph source path. The graph/shader compilation path
    /// arrives later via the graph-builder backend; v1 carries the reference.
    pub shader_graph: String,
    pub property_groups: Vec<MaterialPropertyGroup>,
}

/// Compiled material instance (`azoth.material.material`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::asset::Asset, bevy::reflect::TypePath))]
pub struct MaterialAsset {
    pub name: String,
    /// Catalog product path of the compiled material type
    /// (e.g. `materials/types/standard.azmaterialtype`).
    pub material_type: String,
    /// Catalog product path of the compiled parent material, when set.
    pub parent: Option<String>,
    /// Resolved per-instance property bindings.
    pub property_values: Vec<MaterialPropertyBinding>,
}
