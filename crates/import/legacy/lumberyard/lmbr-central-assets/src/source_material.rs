//! Editable source schemas for legacy Cry `.mtl` materials.

use az_gem_lmbr_central::{
    MaterialAsset, MaterialDefinition, MaterialPublicParam, MaterialTextureFilter,
    MaterialTextureMap, MaterialTextureReference, MaterialTextureType,
};
use bevy::color::{LinearRgba, Srgba};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

/// Faithful editable source emitted from legacy Cry `.mtl` files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialSource {
    pub source_path: String,
    pub root: MaterialDefinitionSource,
    pub sub_materials: Vec<MaterialDefinitionSource>,
}

impl MaterialSource {
    #[must_use]
    pub fn from_asset(asset: &MaterialAsset) -> Self {
        Self {
            source_path: asset.source_path.clone(),
            root: MaterialDefinitionSource::from(&asset.root),
            sub_materials: asset
                .sub_materials
                .iter()
                .map(MaterialDefinitionSource::from)
                .collect(),
        }
    }

    #[must_use]
    pub fn to_asset(&self) -> MaterialAsset {
        MaterialAsset {
            source_path: self.source_path.clone(),
            root: MaterialDefinition::from(&self.root),
            sub_materials: self
                .sub_materials
                .iter()
                .map(MaterialDefinition::from)
                .collect(),
        }
    }

    /// Serializes the material source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

/// One material or sub-material source block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialDefinitionSource {
    pub name: Option<String>,
    pub shader: Option<String>,
    pub surface_type: Option<String>,
    pub diffuse: Option<MaterialColorSource>,
    pub specular: Option<MaterialColorSource>,
    pub emissive: Option<MaterialColorSource>,
    pub emittance: Option<MaterialLinearColorSource>,
    pub opacity: f32,
    pub shininess: f32,
    pub alpha_test: Option<f32>,
    pub gen_mask: Option<String>,
    pub string_gen_mask: Option<String>,
    pub material_flags: Option<u64>,
    pub cloak_amount: Option<f32>,
    pub textures: Vec<MaterialTextureReferenceSource>,
    pub public_params: Vec<MaterialPublicParamSource>,
    pub extra_attributes: Vec<MaterialPublicParamSource>,
}

impl From<&MaterialDefinition> for MaterialDefinitionSource {
    fn from(value: &MaterialDefinition) -> Self {
        Self {
            name: value.name.clone(),
            shader: value.shader.clone(),
            surface_type: value.surface_type.clone(),
            diffuse: value.diffuse.map(MaterialColorSource::from),
            specular: value.specular.map(MaterialColorSource::from),
            emissive: value.emissive.map(MaterialColorSource::from),
            emittance: value.emittance.map(MaterialLinearColorSource::from),
            opacity: value.opacity,
            shininess: value.shininess,
            alpha_test: value.alpha_test,
            gen_mask: value.gen_mask.clone(),
            string_gen_mask: value.string_gen_mask.clone(),
            material_flags: value.material_flags,
            cloak_amount: value.cloak_amount,
            textures: value
                .textures
                .iter()
                .map(MaterialTextureReferenceSource::from)
                .collect(),
            public_params: value
                .public_params
                .iter()
                .map(MaterialPublicParamSource::from)
                .collect(),
            extra_attributes: value
                .extra_attributes
                .iter()
                .map(MaterialPublicParamSource::from)
                .collect(),
        }
    }
}

impl From<&MaterialDefinitionSource> for MaterialDefinition {
    fn from(value: &MaterialDefinitionSource) -> Self {
        Self {
            name: value.name.clone(),
            shader: value.shader.clone(),
            surface_type: value.surface_type.clone(),
            diffuse: value.diffuse.map(Srgba::from),
            specular: value.specular.map(Srgba::from),
            emissive: value.emissive.map(Srgba::from),
            emittance: value.emittance.map(LinearRgba::from),
            opacity: value.opacity,
            shininess: value.shininess,
            alpha_test: value.alpha_test,
            gen_mask: value.gen_mask.clone(),
            string_gen_mask: value.string_gen_mask.clone(),
            material_flags: value.material_flags,
            cloak_amount: value.cloak_amount,
            textures: value
                .textures
                .iter()
                .map(MaterialTextureReference::from)
                .collect(),
            public_params: value
                .public_params
                .iter()
                .map(MaterialPublicParam::from)
                .collect(),
            extra_attributes: value
                .extra_attributes
                .iter()
                .map(MaterialPublicParam::from)
                .collect(),
        }
    }
}

/// sRGB material color preserved as editable scalar channels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaterialColorSource {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl From<Srgba> for MaterialColorSource {
    fn from(value: Srgba) -> Self {
        Self {
            red: value.red,
            green: value.green,
            blue: value.blue,
            alpha: value.alpha,
        }
    }
}

impl From<MaterialColorSource> for Srgba {
    fn from(value: MaterialColorSource) -> Self {
        Self::new(value.red, value.green, value.blue, value.alpha)
    }
}

/// Linear material color preserved as editable scalar channels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaterialLinearColorSource {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl From<LinearRgba> for MaterialLinearColorSource {
    fn from(value: LinearRgba) -> Self {
        Self {
            red: value.red,
            green: value.green,
            blue: value.blue,
            alpha: value.alpha,
        }
    }
}

impl From<MaterialLinearColorSource> for LinearRgba {
    fn from(value: MaterialLinearColorSource) -> Self {
        Self::new(value.red, value.green, value.blue, value.alpha)
    }
}

/// Texture reference inside a source material block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialTextureReferenceSource {
    pub map: String,
    pub image_asset_path: Option<String>,
    pub asset_id: Option<String>,
    pub filter: Option<i32>,
    pub is_tile_u: bool,
    pub is_tile_v: bool,
    pub texture_type: Option<i32>,
    pub texture_modifier: Vec<MaterialPublicParamSource>,
}

impl From<&MaterialTextureReference> for MaterialTextureReferenceSource {
    fn from(value: &MaterialTextureReference) -> Self {
        Self {
            map: value.map.native_name().into_owned(),
            image_asset_path: value.image_asset_path.clone(),
            asset_id: value.asset_id.clone(),
            filter: value.filter.map(MaterialTextureFilter::native_value),
            is_tile_u: value.is_tile_u,
            is_tile_v: value.is_tile_v,
            texture_type: value.texture_type.map(MaterialTextureType::native_value),
            texture_modifier: value
                .texture_modifier
                .iter()
                .map(MaterialPublicParamSource::from)
                .collect(),
        }
    }
}

impl From<&MaterialTextureReferenceSource> for MaterialTextureReference {
    fn from(value: &MaterialTextureReferenceSource) -> Self {
        Self {
            map: MaterialTextureMap::from_native_name(&value.map),
            image_asset_path: value.image_asset_path.clone(),
            asset_id: value.asset_id.clone(),
            filter: value
                .filter
                .and_then(MaterialTextureFilter::from_native_value),
            is_tile_u: value.is_tile_u,
            is_tile_v: value.is_tile_v,
            texture_type: value
                .texture_type
                .and_then(MaterialTextureType::from_native_value),
            texture_modifier: value
                .texture_modifier
                .iter()
                .map(MaterialPublicParam::from)
                .collect(),
        }
    }
}

/// Name/value parameter preserved from material XML attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialPublicParamSource {
    pub name: String,
    pub value: String,
}

impl From<&MaterialPublicParam> for MaterialPublicParamSource {
    fn from(value: &MaterialPublicParam) -> Self {
        Self {
            name: value.name.clone(),
            value: value.value.clone(),
        }
    }
}

impl From<&MaterialPublicParamSource> for MaterialPublicParam {
    fn from(value: &MaterialPublicParamSource) -> Self {
        Self {
            name: value.name.clone(),
            value: value.value.clone(),
        }
    }
}
