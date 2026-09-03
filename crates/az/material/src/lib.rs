//! Native Azoth material authored-source schemas.
//!
//! These types describe editable source documents and material property
//! interfaces. Runtime material products and shader compilation are built from
//! these sources by asset-builder crates; this crate stays schema-only so
//! project-host can expose material editing without linking render/runtime code.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod graph;
mod material_type;
mod source;
mod value;

pub use graph::{
    MaterialGraphCommentSource, MaterialGraphConnectionRouteSource, MaterialGraphConnectionSource,
    MaterialGraphInputValueSource, MaterialGraphNodeLayoutSource, MaterialGraphNodeSource,
    MaterialGraphPointSource, MaterialGraphPortRefSource, MaterialGraphRouteAnchorKindSource,
    MaterialGraphRouteAnchorSource, MaterialGraphRouteStyleSource, MaterialGraphSource,
    material_graph_extension,
};
pub use material_type::MaterialTypeSource;
pub use source::MaterialSource;
pub use value::{
    BlendMode, CullMode, MaterialColor, MaterialDomain, MaterialPropertyBinding,
    MaterialPropertyDefinition, MaterialPropertyGroup, MaterialPropertyValue, MaterialTexture,
    ShadingModel,
};

pub const MATERIAL_SCHEMA_NAME: &str = "azoth.material.Material";
pub const MATERIAL_TYPE_SCHEMA_NAME: &str = "azoth.material.MaterialType";
pub const MATERIAL_GRAPH_NODE_SCHEMA_NAME: &str = "azoth.material.graph.Node";
pub const MATERIAL_GRAPH_INPUT_VALUE_SCHEMA_NAME: &str = "azoth.material.graph.InputValue";
pub const MATERIAL_GRAPH_NODE_LAYOUT_SCHEMA_NAME: &str = "azoth.material.graph.NodeLayout";
pub const MATERIAL_GRAPH_CONNECTION_SCHEMA_NAME: &str = "azoth.material.graph.Connection";
pub const MATERIAL_GRAPH_PORT_REF_SCHEMA_NAME: &str = "azoth.material.graph.PortRef";
pub const MATERIAL_GRAPH_CONNECTION_ROUTE_SCHEMA_NAME: &str =
    "azoth.material.graph.ConnectionRoute";
pub const MATERIAL_GRAPH_ROUTE_STYLE_SCHEMA_NAME: &str = "azoth.material.graph.RouteStyle";
pub const MATERIAL_GRAPH_ROUTE_ANCHOR_SCHEMA_NAME: &str = "azoth.material.graph.RouteAnchor";
pub const MATERIAL_GRAPH_ROUTE_ANCHOR_KIND_SCHEMA_NAME: &str =
    "azoth.material.graph.RouteAnchorKind";
pub const MATERIAL_GRAPH_POINT_SCHEMA_NAME: &str = "azoth.material.graph.Point";
pub const MATERIAL_GRAPH_COMMENT_SCHEMA_NAME: &str = "azoth.material.graph.Comment";
pub const PROPERTY_VALUE_SCHEMA_NAME: &str = "azoth.material.PropertyValue";
pub const PROPERTY_BINDING_SCHEMA_NAME: &str = "azoth.material.PropertyBinding";
pub const PROPERTY_DEFINITION_SCHEMA_NAME: &str = "azoth.material.PropertyDefinition";
pub const PROPERTY_GROUP_SCHEMA_NAME: &str = "azoth.material.PropertyGroup";
pub const MATERIAL_COLOR_SCHEMA_NAME: &str = "azoth.material.Color";
pub const MATERIAL_TEXTURE_SCHEMA_NAME: &str = "azoth.material.Texture";
pub const MATERIAL_DOMAIN_SCHEMA_NAME: &str = "azoth.material.Domain";
pub const BLEND_MODE_SCHEMA_NAME: &str = "azoth.material.BlendMode";
pub const CULL_MODE_SCHEMA_NAME: &str = "azoth.material.CullMode";
pub const SHADING_MODEL_SCHEMA_NAME: &str = "azoth.material.ShadingModel";

pub const MATERIAL_SOURCE_ROOT: &str = "project:source-root";
pub const MATERIAL_PATH_PREFIX: &str = "materials";
pub const MATERIAL_EXTENSION: &str = "azmaterial.ron";
pub const MATERIAL_TYPE_PATH_PREFIX: &str = "materials/types";
pub const MATERIAL_TYPE_EXTENSION: &str = "azmaterialtype.ron";

pub const MATERIAL_ASSET_TYPE_HINT: &str = "material";
pub const MATERIAL_TYPE_ASSET_TYPE_HINT: &str = "material-type";
pub const MATERIAL_GRAPH_ASSET_TYPE_HINT: &str = "azoth.material.graph";
pub const MATERIAL_GRAPH_SOURCE_SCHEMA_NAME: &str = "azoth.material.graph.source";
pub const MATERIAL_GRAPH_EXTENSION: &str = "azmat.ron";
pub const TEXTURE_ASSET_TYPE_HINT: &str = "texture";

pub const UNRELEASED_MATERIAL_SCHEMA_VERSION_ERROR: &str = "Azoth is not released; material authored schemas must stay at version 1. Do not bump material schema versions until the first release defines migrations.";

pub const MATERIAL_SCHEMA_VERSION: u32 = assert_unreleased_material_schema_v1(1);
pub const MATERIAL_TYPE_SCHEMA_VERSION: u32 = assert_unreleased_material_schema_v1(1);
pub const PROPERTY_SCHEMA_VERSION: u32 = assert_unreleased_material_schema_v1(1);
pub const MATERIAL_ENUM_SCHEMA_VERSION: u32 = assert_unreleased_material_schema_v1(1);

/// # Panics
///
/// Panics if `version` is anything but 1. Being a `const fn`, a bump in a
/// `const` initializer fails the build rather than the test run.
#[must_use]
pub const fn assert_unreleased_material_schema_v1(version: u32) -> u32 {
    assert!(
        version == 1,
        "Azoth is not released; material authored schemas must stay at version 1. Do not bump material schema versions until the first release defines migrations."
    );
    version
}
