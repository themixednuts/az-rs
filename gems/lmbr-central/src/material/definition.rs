//! Material asset and layer definitions.

use bevy::color::{Alpha, LinearRgba, Srgba};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::color::material_color_from_native;
use super::texture::{MaterialPublicParam, MaterialTextureMap, MaterialTextureReference};

/// Engine material asset.
#[derive(Asset, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub struct MaterialAsset {
    pub source_path: String,
    pub root: MaterialDefinition,
    pub sub_materials: Vec<MaterialDefinition>,
}

impl MaterialAsset {
    #[must_use]
    pub fn primary_material(&self) -> &MaterialDefinition {
        self.sub_materials.first().unwrap_or(&self.root)
    }

    #[must_use]
    pub fn material_for_slot(&self, slot: Option<u32>) -> &MaterialDefinition {
        slot.map_or_else(
            || self.primary_material(),
            |slot| self.sub_materials.get(slot as usize).unwrap_or(&self.root),
        )
    }

    #[must_use]
    pub fn standard_material(&self) -> StandardMaterial {
        self.primary_material().standard_material()
    }

    #[must_use]
    pub fn standard_material_for_slot(&self, slot: Option<u32>) -> StandardMaterial {
        self.material_for_slot(slot).standard_material()
    }

    #[must_use]
    pub fn standard_material_with_images(
        &self,
        image: impl FnMut(&str) -> Handle<Image>,
    ) -> StandardMaterial {
        self.primary_material().standard_material_with_images(image)
    }

    #[must_use]
    pub fn standard_material_for_slot_with_images(
        &self,
        slot: Option<u32>,
        image: impl FnMut(&str) -> Handle<Image>,
    ) -> StandardMaterial {
        self.material_for_slot(slot)
            .standard_material_with_images(image)
    }
}

/// Material parameter override asset.
#[derive(Asset, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideAsset {
    pub source_path: String,
    pub max_trigger_distance: Option<String>,
    pub materials: Vec<MaterialOverrideTarget>,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

impl MaterialOverrideAsset {
    #[must_use]
    pub fn primary_sub_material(&self) -> Option<&MaterialOverrideSubTarget> {
        self.materials
            .first()
            .and_then(|target| target.sub_materials.first())
    }

    #[must_use]
    pub fn standard_material(&self) -> StandardMaterial {
        let mut material = override_standard_material();
        let Some(sub_material) = self.primary_sub_material() else {
            return material;
        };

        apply_override_shader_params(&mut material, sub_material);
        material
    }

    #[must_use]
    pub fn standard_material_with_images(
        &self,
        mut image: impl FnMut(&str) -> Handle<Image>,
    ) -> StandardMaterial {
        let mut material = override_standard_material();
        let Some(sub_material) = self.primary_sub_material() else {
            return material;
        };

        apply_override_textures(&mut material, sub_material, &mut image);
        apply_override_shader_params(&mut material, sub_material);
        material
    }
}

/// Material matched by a material override asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideTarget {
    pub name: Option<String>,
    pub exclude: Option<String>,
    pub sub_materials: Vec<MaterialOverrideSubTarget>,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

/// Sub-material matched by a material override asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideSubTarget {
    pub name: Option<String>,
    pub shader_generation_params: Vec<MaterialOverrideSwitch>,
    pub texture_maps: Vec<MaterialOverrideParamBlock>,
    pub shader_params: Vec<MaterialOverrideParamBlock>,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

/// Shader-generation toggle in a material override.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideSwitch {
    pub name: String,
    pub enabled: bool,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

/// Named parameter block in a material override.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideParamBlock {
    pub name: String,
    pub params: Vec<MaterialOverrideParam>,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

/// One typed parameter value in a material override.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct MaterialOverrideParam {
    pub name: String,
    pub value_kind: MaterialOverrideValueKind,
    pub value: String,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

/// Native parameter value kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterialOverrideValueKind {
    #[default]
    String,
    Float,
    Color,
    Bool,
    Int,
    Vector,
    Unknown(String),
}

impl MaterialOverrideValueKind {
    #[must_use]
    pub fn from_native_name(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            // An absent kind is the native default, which is `String`.
            "string" | "" => Self::String,
            "float" => Self::Float,
            "color" => Self::Color,
            "bool" => Self::Bool,
            "int" => Self::Int,
            "vector" | "vec3" | "vec4" => Self::Vector,
            _ => Self::Unknown(value.trim().to_string()),
        }
    }
}

/// One material layer.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
pub struct MaterialDefinition {
    pub name: Option<String>,
    pub shader: Option<String>,
    pub surface_type: Option<String>,
    pub diffuse: Option<Srgba>,
    pub specular: Option<Srgba>,
    pub emissive: Option<Srgba>,
    /// Native `Emittance` mapped to Bevy emissive radiance.
    ///
    /// Lumberyard reference: `dev/Code/Framework/GFxFramework/GFxFramework/MaterialIO/Material.cpp`.
    pub emittance: Option<LinearRgba>,
    pub opacity: f32,
    pub shininess: f32,
    pub alpha_test: Option<f32>,
    pub gen_mask: Option<String>,
    pub string_gen_mask: Option<String>,
    pub material_flags: Option<u64>,
    pub cloak_amount: Option<f32>,
    pub textures: Vec<MaterialTextureReference>,
    pub public_params: Vec<MaterialPublicParam>,
    pub extra_attributes: Vec<MaterialPublicParam>,
}

impl Default for MaterialDefinition {
    fn default() -> Self {
        Self {
            name: None,
            shader: None,
            surface_type: None,
            diffuse: None,
            specular: None,
            emissive: None,
            emittance: None,
            opacity: 1.0,
            shininess: 0.0,
            alpha_test: None,
            gen_mask: None,
            string_gen_mask: None,
            material_flags: None,
            cloak_amount: None,
            textures: Vec::new(),
            public_params: Vec::new(),
            extra_attributes: Vec::new(),
        }
    }
}

impl MaterialDefinition {
    #[must_use]
    pub fn image_asset_path(&self, map: &MaterialTextureMap) -> Option<&str> {
        self.textures
            .iter()
            .find(|texture| texture.map == *map)
            .and_then(|texture| non_empty(texture.image_asset_path.as_deref()))
    }

    #[must_use]
    pub fn standard_material(&self) -> StandardMaterial {
        let opacity = self.opacity.clamp(0.0, 1.0);
        let base_color = Color::from(self.diffuse.unwrap_or(Srgba::WHITE)).with_alpha(opacity);
        let emissive = self
            .emittance
            .unwrap_or_else(|| self.emissive.map_or(LinearRgba::BLACK, LinearRgba::from));

        StandardMaterial {
            base_color,
            emissive,
            perceptual_roughness: 1.0 - (self.shininess / 255.0).clamp(0.0, 1.0),
            alpha_mode: match self.alpha_test {
                Some(alpha_test) => AlphaMode::Mask(alpha_test.clamp(0.0, 1.0)),
                None if opacity < 1.0 => AlphaMode::Blend,
                None => AlphaMode::Opaque,
            },
            ..Default::default()
        }
    }

    #[must_use]
    pub fn standard_material_with_images(
        &self,
        mut image: impl FnMut(&str) -> Handle<Image>,
    ) -> StandardMaterial {
        let mut material = self.standard_material();
        material.base_color_texture = self
            .image_asset_path(&MaterialTextureMap::Diffuse)
            .map(&mut image);
        material.normal_map_texture = self
            .image_asset_path(&MaterialTextureMap::Bumpmap)
            .map(&mut image);
        material.emissive_texture = self
            .image_asset_path(&MaterialTextureMap::Emittance)
            .map(&mut image);
        material.occlusion_texture = self
            .image_asset_path(&MaterialTextureMap::Occlusion)
            .map(&mut image);
        material.metallic_roughness_texture = self
            .image_asset_path(&MaterialTextureMap::Smoothness)
            .or_else(|| self.image_asset_path(&MaterialTextureMap::Specular))
            .map(&mut image);
        material
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then_some(value)
    })
}

fn override_standard_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.72,
        ..Default::default()
    }
}

fn apply_override_textures(
    material: &mut StandardMaterial,
    sub_material: &MaterialOverrideSubTarget,
    image: &mut impl FnMut(&str) -> Handle<Image>,
) {
    for block in &sub_material.texture_maps {
        let Some(path) = block.params.iter().find_map(|param| {
            (param.value_kind == MaterialOverrideValueKind::String)
                .then(|| param.value.trim())
                .filter(|path| !path.is_empty())
        }) else {
            continue;
        };

        if contains_ascii_case_insensitive(&block.name, "bump")
            || contains_ascii_case_insensitive(&block.name, "normal")
        {
            material.normal_map_texture = Some(image(path));
        } else if contains_ascii_case_insensitive(&block.name, "emiss") {
            material.emissive_texture = Some(image(path));
        } else if contains_ascii_case_insensitive(&block.name, "smooth")
            || contains_ascii_case_insensitive(&block.name, "spec")
            || contains_ascii_case_insensitive(&block.name, "rough")
        {
            material.metallic_roughness_texture = Some(image(path));
        } else if material.base_color_texture.is_none() {
            material.base_color_texture = Some(image(path));
        }
    }
}

fn apply_override_shader_params(
    material: &mut StandardMaterial,
    sub_material: &MaterialOverrideSubTarget,
) {
    for block in &sub_material.shader_params {
        let block_name = block.name.as_str();
        for param in &block.params {
            let matches_name = |needle| {
                contains_ascii_case_insensitive(block_name, needle)
                    || contains_ascii_case_insensitive(&param.name, needle)
            };

            match &param.value_kind {
                MaterialOverrideValueKind::Color => {
                    if let Some(color) = material_color_from_native(&param.value) {
                        let color = Color::from(color);
                        if matches_name("emiss") {
                            material.emissive = color.to_linear();
                        } else {
                            material.base_color = color;
                            if color.to_srgba().alpha < 1.0 {
                                material.alpha_mode = AlphaMode::Blend;
                            }
                        }
                    }
                }
                MaterialOverrideValueKind::Float => {
                    if let Ok(value) = param.value.trim().parse::<f32>() {
                        if matches_name("opacity") || matches_name("alpha") {
                            let alpha = value.clamp(0.0, 1.0);
                            material.base_color = material.base_color.with_alpha(alpha);
                            if alpha < 1.0 {
                                material.alpha_mode = AlphaMode::Blend;
                            }
                        } else if matches_name("rough") {
                            material.perceptual_roughness = value.clamp(0.0, 1.0);
                        } else if matches_name("gloss") || matches_name("smooth") {
                            material.perceptual_roughness = 1.0 - value.clamp(0.0, 1.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
