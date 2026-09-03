//! Typed source lowering for material compiles.
//!
//! Material property sources decode directly from RON into
//! `MaterialTypeSource` / `MaterialSource`, then lower to compiled
//! property-table products.

use az_material::{
    MATERIAL_EXTENSION, MATERIAL_SCHEMA_NAME, MATERIAL_TYPE_EXTENSION, MATERIAL_TYPE_SCHEMA_NAME,
    MaterialSource, MaterialTypeSource,
};
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::codec::{MaterialAssetCodecError, encode_material_asset, encode_material_type_asset};
use crate::{
    MATERIAL_PRODUCT_EXTENSION, MATERIAL_TYPE_PRODUCT_EXTENSION, MaterialAsset, MaterialTypeAsset,
};

#[derive(Debug, Error)]
pub enum MaterialCompileError {
    #[error("failed to decode typed RON source as `{schema}`: {source}")]
    DecodeSource {
        schema: &'static str,
        #[source]
        source: ron::error::SpannedError,
    },

    #[error("material reference `{path}` is not a `{expected}` source")]
    InvalidReference {
        path: String,
        expected: &'static str,
    },

    #[error("failed to encode material product: {0}")]
    Encode(#[from] MaterialAssetCodecError),
}

/// Compiled material-type product plus the dependency facts the builder reports.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMaterialType {
    pub product_path: String,
    pub asset: MaterialTypeAsset,
    pub bytes: Vec<u8>,
    /// Authored shader-graph source path.
    pub shader_graph_source: String,
}

/// Compiled material product plus the dependency facts the builder reports.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMaterial {
    pub product_path: String,
    pub asset: MaterialAsset,
    pub bytes: Vec<u8>,
    /// Material-type authored source path this material selects.
    pub material_type_source: String,
    /// Parent material authored source path, when set.
    pub parent_source: Option<String>,
}

/// Catalog product path for the compiled material type from `source_path`.
#[must_use]
pub fn material_type_product_path(source_path: &str) -> String {
    product_path(
        source_path,
        MATERIAL_TYPE_EXTENSION,
        MATERIAL_TYPE_PRODUCT_EXTENSION,
    )
}

/// Catalog product path for the compiled material from `source_path`.
#[must_use]
pub fn material_product_path(source_path: &str) -> String {
    product_path(source_path, MATERIAL_EXTENSION, MATERIAL_PRODUCT_EXTENSION)
}

fn product_path(source_path: &str, source_extension: &str, product_extension: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = normalized
        .strip_suffix(&format!(".{source_extension}"))
        .unwrap_or(normalized.as_str());
    if stem == "materials" || stem.starts_with("materials/") {
        format!("{stem}.{product_extension}")
    } else {
        format!("materials/{stem}.{product_extension}")
    }
}

/// Extract the shader-graph source dependency for `create_jobs`.
///
/// # Errors
///
/// Returns an error if the source bytes are not a valid material-type authored
/// document.
pub fn material_type_source_dependencies(
    source_bytes: &[u8],
) -> Result<Vec<String>, MaterialCompileError> {
    let source = decode_material_type_source(source_bytes)?;
    Ok(vec![source.shader_graph.into_string()])
}

/// Extract material-type and parent source dependencies for `create_jobs`.
///
/// # Errors
///
/// Returns an error if the source bytes are not a valid material authored
/// document.
pub fn material_source_dependencies(
    source_bytes: &[u8],
) -> Result<Vec<String>, MaterialCompileError> {
    let source = decode_material_source(source_bytes)?;
    let mut dependencies = vec![source.material_type.into_string()];
    if let Some(parent) = source.parent {
        dependencies.push(parent.into_string());
    }
    Ok(dependencies)
}

/// Compile one material-type authored document into a compiled material-type
/// product.
///
/// # Errors
///
/// Returns an error when the source is not a valid `azoth.material.MaterialType`
/// authored document or cannot be encoded.
pub fn compile_material_type_document(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<CompiledMaterialType, MaterialCompileError> {
    let source = decode_material_type_source(source_bytes)?;

    let asset = MaterialTypeAsset {
        name: source.name,
        description: source.description,
        domain: source.domain,
        blend_mode: source.blend_mode,
        cull_mode: source.cull_mode,
        shading_model: source.shading_model,
        shader_graph: source.shader_graph.as_str().to_string(),
        property_groups: source.property_groups,
    };
    let bytes = encode_material_type_asset(&asset)?;

    Ok(CompiledMaterialType {
        product_path: material_type_product_path(source_path),
        shader_graph_source: source.shader_graph.into_string(),
        asset,
        bytes,
    })
}

/// Compile one material authored document into a compiled material product.
///
/// # Errors
///
/// Returns an error when the source is not a valid `azoth.material.Material`
/// authored document, its references do not point at material sources, or the
/// product cannot be encoded.
pub fn compile_material_document(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<CompiledMaterial, MaterialCompileError> {
    let source = decode_material_source(source_bytes)?;

    let material_type_source = source.material_type.into_string();
    require_extension(&material_type_source, MATERIAL_TYPE_EXTENSION)?;
    let parent_source = source.parent.map(az_core::AssetPathBuf::into_string);
    if let Some(parent_source) = &parent_source {
        require_extension(parent_source, MATERIAL_EXTENSION)?;
    }

    let asset = MaterialAsset {
        name: source.name,
        material_type: material_type_product_path(&material_type_source),
        parent: parent_source.as_deref().map(material_product_path),
        property_values: source.property_values,
    };
    let bytes = encode_material_asset(&asset)?;

    Ok(CompiledMaterial {
        product_path: material_product_path(source_path),
        asset,
        bytes,
        material_type_source,
        parent_source,
    })
}

fn require_extension(path: &str, extension: &'static str) -> Result<(), MaterialCompileError> {
    if path.ends_with(&format!(".{extension}")) {
        Ok(())
    } else {
        Err(MaterialCompileError::InvalidReference {
            path: path.to_string(),
            expected: extension,
        })
    }
}

fn decode_material_type_source(
    source_bytes: &[u8],
) -> Result<MaterialTypeSource, MaterialCompileError> {
    decode_source(source_bytes, MATERIAL_TYPE_SCHEMA_NAME)
}

fn decode_material_source(source_bytes: &[u8]) -> Result<MaterialSource, MaterialCompileError> {
    decode_source(source_bytes, MATERIAL_SCHEMA_NAME)
}

fn decode_source<T: DeserializeOwned>(
    source_bytes: &[u8],
    schema: &'static str,
) -> Result<T, MaterialCompileError> {
    ron::de::from_bytes(source_bytes)
        .map_err(|source| MaterialCompileError::DecodeSource { schema, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_material_asset, decode_material_type_asset};
    use az_material::{
        BlendMode, CullMode, MaterialColor, MaterialDomain, MaterialPropertyBinding,
        MaterialPropertyDefinition, MaterialPropertyGroup, MaterialPropertyValue, ShadingModel,
    };
    use glam::Vec4;
    use ron::ser::PrettyConfig;

    fn source_ron(value: &impl serde::Serialize) -> String {
        ron::ser::to_string_pretty(value, PrettyConfig::new()).unwrap()
    }

    fn sample_material_type_source() -> MaterialTypeSource {
        MaterialTypeSource {
            name: "Standard".to_string(),
            description: "Opaque surface".to_string(),
            domain: MaterialDomain::Surface,
            blend_mode: BlendMode::Opaque,
            cull_mode: CullMode::Back,
            shading_model: ShadingModel::Standard,
            shader_graph: "materials/graphs/standard.azmat.ron".parse().unwrap(),
            property_groups: vec![MaterialPropertyGroup {
                id: "base".to_string(),
                display_name: "Base".to_string(),
                description: String::new(),
                properties: vec![MaterialPropertyDefinition {
                    id: "base.color".to_string(),
                    display_name: "Color".to_string(),
                    description: String::new(),
                    default_value: MaterialPropertyValue::Color(MaterialColor { rgba: Vec4::ONE }),
                    connection: Some("base_color".to_string()),
                    enum_values: Vec::new(),
                }],
            }],
        }
    }

    fn sample_material_source() -> MaterialSource {
        MaterialSource {
            name: "wood".to_string(),
            material_type: "materials/types/standard.azmaterialtype.ron"
                .parse()
                .unwrap(),
            parent: Some("materials/base.azmaterial.ron".parse().unwrap()),
            property_values: vec![MaterialPropertyBinding {
                property: "base.color".to_string(),
                value: MaterialPropertyValue::Float(0.5),
            }],
        }
    }

    #[test]
    fn compiles_authored_material_type_ron_into_decodable_product() {
        let ron = source_ron(&sample_material_type_source());

        let compiled = compile_material_type_document(
            "materials/types/standard.azmaterialtype.ron",
            ron.as_bytes(),
        )
        .unwrap();

        assert_eq!(
            compiled.product_path,
            "materials/types/standard.azmaterialtype"
        );
        assert_eq!(
            compiled.shader_graph_source,
            "materials/graphs/standard.azmat.ron"
        );
        let decoded = decode_material_type_asset(&compiled.bytes).unwrap();
        assert_eq!(decoded, compiled.asset);
        assert_eq!(decoded.name, "Standard");
        assert_eq!(decoded.property_groups.len(), 1);
        assert_eq!(decoded.property_groups[0].properties[0].id, "base.color");
    }

    #[test]
    fn compiles_authored_material_ron_into_decodable_product() {
        let ron = source_ron(&sample_material_source());

        let compiled =
            compile_material_document("materials/wood.azmaterial.ron", ron.as_bytes()).unwrap();

        assert_eq!(compiled.product_path, "materials/wood.azmaterial");
        assert_eq!(
            compiled.material_type_source,
            "materials/types/standard.azmaterialtype.ron"
        );
        assert_eq!(
            compiled.parent_source,
            Some("materials/base.azmaterial.ron".to_string())
        );
        let decoded = decode_material_asset(&compiled.bytes).unwrap();
        assert_eq!(decoded, compiled.asset);
        assert_eq!(
            decoded.material_type,
            "materials/types/standard.azmaterialtype"
        );
        assert_eq!(
            decoded.parent,
            Some("materials/base.azmaterial".to_string())
        );
        assert_eq!(decoded.property_values.len(), 1);
    }

    #[test]
    fn material_dependencies_cover_type_and_parent() {
        let ron = source_ron(&sample_material_source());

        assert_eq!(
            material_source_dependencies(ron.as_bytes()).unwrap(),
            vec![
                "materials/types/standard.azmaterialtype.ron".to_string(),
                "materials/base.azmaterial.ron".to_string(),
            ]
        );
    }

    #[test]
    fn material_type_dependencies_cover_shader_graph() {
        let ron = source_ron(&sample_material_type_source());

        assert_eq!(
            material_type_source_dependencies(ron.as_bytes()).unwrap(),
            vec!["materials/graphs/standard.azmat.ron".to_string()]
        );
    }

    #[test]
    fn incomplete_material_document_fails_compile() {
        let ron = "(name: \"incomplete\")";

        let error =
            compile_material_document("materials/bad.azmaterial.ron", ron.as_bytes()).unwrap_err();

        assert!(matches!(error, MaterialCompileError::DecodeSource { .. }));
    }

    #[test]
    fn material_rejects_non_material_type_reference() {
        let mut source = sample_material_source();
        source.material_type = "textures/not-a-type.png".parse().unwrap();
        let ron = source_ron(&source);

        let error =
            compile_material_document("materials/wood.azmaterial.ron", ron.as_bytes()).unwrap_err();

        assert!(matches!(
            error,
            MaterialCompileError::InvalidReference { .. }
        ));
    }

    #[test]
    fn product_paths_are_deterministic_and_prefixed() {
        assert_eq!(
            material_type_product_path("materials/types/standard.azmaterialtype.ron"),
            "materials/types/standard.azmaterialtype"
        );
        assert_eq!(
            material_product_path("materials/wood.azmaterial.ron"),
            "materials/wood.azmaterial"
        );
        assert_eq!(
            material_product_path("props\\crate.azmaterial.ron"),
            "materials/props/crate.azmaterial"
        );
    }
}
