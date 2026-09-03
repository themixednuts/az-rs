//! Terrain surface and material-layer data.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use crate::world::TerrainRegionId;

use super::type_ids::{
    TERRAIN_SURFACE_HOLE_ID, TERRAIN_SURFACE_UNDEFINED_ID, TERRAIN_SURFACE_WEIGHT_COUNT,
};
use crate::heightmap::math::remap_index;

/// Terrain surface ids and weights.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/ITerrain.h:26`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct TerrainSurfaceWeight {
    pub ids: [u8; TERRAIN_SURFACE_WEIGHT_COUNT],
    pub weights: [u8; TERRAIN_SURFACE_WEIGHT_COUNT],
}

impl Default for TerrainSurfaceWeight {
    fn default() -> Self {
        Self {
            ids: [TERRAIN_SURFACE_UNDEFINED_ID; TERRAIN_SURFACE_WEIGHT_COUNT],
            weights: [0; TERRAIN_SURFACE_WEIGHT_COUNT],
        }
    }
}

impl TerrainSurfaceWeight {
    pub const HOLE: Self = Self {
        ids: [TERRAIN_SURFACE_HOLE_ID; TERRAIN_SURFACE_WEIGHT_COUNT],
        weights: [u8::MAX, 0, 0],
    };

    #[must_use]
    pub const fn primary_id(self) -> u8 {
        self.ids[0]
    }

    #[must_use]
    pub const fn is_hole(self) -> bool {
        self.primary_id() == TERRAIN_SURFACE_HOLE_ID
    }
}

/// Terrain surface weights sampled for rendering and queries.
///
/// Lumberyard reference: `dev/Gems/LegacyTerrain/Code/Source/terrain.h:151`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct TerrainSurfaceMap {
    pub resolution: usize,
    pub weights: Vec<TerrainSurfaceWeight>,
}

impl TerrainSurfaceMap {
    /// Build a surface map from a square grid of surface weights.
    ///
    /// # Errors
    ///
    /// Returns `"terrain surface map resolution must be non-zero"` for a
    /// `resolution` of `0`, or `"terrain surface map weights do not match
    /// resolution"` if `weights` does not hold exactly
    /// `resolution * resolution` entries.
    pub fn new(
        resolution: usize,
        weights: Vec<TerrainSurfaceWeight>,
    ) -> Result<Self, &'static str> {
        if resolution == 0 {
            return Err("terrain surface map resolution must be non-zero");
        }
        if weights.len() != resolution * resolution {
            return Err("terrain surface map weights do not match resolution");
        }
        Ok(Self {
            resolution,
            weights,
        })
    }

    #[must_use]
    pub fn surface_weight_at(&self, x: usize, y: usize) -> Option<TerrainSurfaceWeight> {
        if x >= self.resolution || y >= self.resolution {
            return None;
        }
        self.weights.get(y * self.resolution + x).copied()
    }

    #[must_use]
    pub(crate) fn is_hole_quad(&self, x: usize, z: usize, mesh_resolution: usize) -> bool {
        if mesh_resolution < 2 {
            return false;
        }
        let max_mesh_quad = mesh_resolution - 2;
        let max_surface = self.resolution.saturating_sub(1);
        let surface_x = remap_index(x, max_mesh_quad, max_surface);
        let surface_z = remap_index(z, max_mesh_quad, max_surface);

        self.surface_weight_at(surface_x, surface_z)
            .is_some_and(TerrainSurfaceWeight::is_hole)
    }

    #[must_use]
    pub(crate) fn contains_hole_quad(
        &self,
        min_x: usize,
        min_z: usize,
        max_x: usize,
        max_z: usize,
        mesh_resolution: usize,
    ) -> bool {
        (min_z..max_z).any(|z| (min_x..max_x).any(|x| self.is_hole_quad(x, z, mesh_resolution)))
    }
}

/// Surface-layer colors used by terrain mesh generation.
#[derive(Component, Debug, Clone, PartialEq, Reflect)]
#[reflect(Component)]
pub struct TerrainSurfacePalette {
    pub default_color: [f32; 4],
    pub layer_colors: Vec<[f32; 4]>,
}

impl TerrainSurfacePalette {
    #[must_use]
    pub fn from_material_layers(
        default_color: Color,
        layers: &[TerrainMaterialLayerData],
    ) -> Option<Self> {
        if layers.is_empty() {
            return None;
        }

        Some(Self {
            default_color: color_array(default_color),
            layer_colors: layers
                .iter()
                .enumerate()
                .map(|(index, layer)| material_layer_color(index, layer))
                .collect(),
        })
    }

    #[must_use]
    pub fn color_for_weight(&self, weight: TerrainSurfaceWeight) -> [f32; 4] {
        let mut total = 0.0;
        let mut color = [0.0; 4];

        for (id, raw_weight) in weight.ids.into_iter().zip(weight.weights) {
            if raw_weight == 0 || id == TERRAIN_SURFACE_HOLE_ID {
                continue;
            }

            let layer_color = if id == TERRAIN_SURFACE_UNDEFINED_ID {
                self.default_color
            } else {
                self.layer_colors
                    .get(id as usize)
                    .copied()
                    .unwrap_or(self.default_color)
            };
            let weight = f32::from(raw_weight);
            total += weight;
            for (channel, layer_channel) in color.iter_mut().zip(layer_color) {
                *channel = layer_channel.mul_add(weight, *channel);
            }
        }

        if total == 0.0 {
            return self.default_color;
        }

        for channel in &mut color {
            *channel /= total;
        }
        color
    }
}

/// Terrain material layer data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct TerrainMaterialLayerData {
    pub material_path: String,
    pub splat_map_path: String,
    pub affected_tiles: u64,
    pub priority: u8,
}

const TERRAIN_LAYER_COLORS: [[f32; 4]; 10] = [
    [0.30, 0.48, 0.24, 1.0],
    [0.47, 0.38, 0.27, 1.0],
    [0.56, 0.57, 0.53, 1.0],
    [0.22, 0.43, 0.47, 1.0],
    [0.54, 0.49, 0.30, 1.0],
    [0.36, 0.31, 0.44, 1.0],
    [0.44, 0.52, 0.34, 1.0],
    [0.50, 0.33, 0.22, 1.0],
    [0.38, 0.44, 0.50, 1.0],
    [0.28, 0.40, 0.30, 1.0],
];

fn color_array(color: Color) -> [f32; 4] {
    let color = color.to_srgba();
    [color.red, color.green, color.blue, color.alpha]
}

fn material_layer_color(index: usize, layer: &TerrainMaterialLayerData) -> [f32; 4] {
    let path = if layer.material_path.is_empty() {
        layer.splat_map_path.as_str()
    } else {
        layer.material_path.as_str()
    };
    let hash = stable_layer_hash(path, index);
    TERRAIN_LAYER_COLORS[hash as usize % TERRAIN_LAYER_COLORS.len()]
}

fn stable_layer_hash(path: &str, index: usize) -> u32 {
    // Layer indices are small; the saturation is a bound, not a wrap.
    let mut hash = 0x811c_9dc5u32 ^ u32::try_from(index).unwrap_or(u32::MAX);
    for byte in path.bytes() {
        hash ^= u32::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Serializable terrain macro-material parameters.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/Terrain/Bus/WorldMaterialRequestsBus.h:22`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct SerializableMacroMaterialParams {
    pub macro_color_scale: f32,
    pub macro_color: LinearRgba,
    pub macro_gloss_scale: f32,
    pub macro_normal_scale: f32,
    pub macro_specular_reflectance: f32,
}

impl Default for SerializableMacroMaterialParams {
    fn default() -> Self {
        Self {
            macro_color_scale: 1.0,
            macro_color: LinearRgba::WHITE,
            macro_gloss_scale: 1.0,
            macro_normal_scale: 1.0,
            macro_specular_reflectance: 0.03,
        }
    }
}

/// Per-region terrain material data.
#[derive(Asset, Debug, Clone, Default, PartialEq, Reflect)]
pub struct RegionMaterialDataAsset {
    pub layers: Vec<TerrainMaterialLayerData>,
    pub default_material_path: String,
    pub macro_color_map_path: String,
    pub macro_gloss_map_path: String,
    pub macro_normal_map_path: String,
    pub pertinent_layers_mip_chain: Vec<u64>,
    pub enable_custom_background_params: bool,
    pub macro_material_params: SerializableMacroMaterialParams,
    pub enable_custom_foreground_params: bool,
    pub custom_macro_material_compositing_params: SerializableMacroMaterialParams,
}

/// Tile-to-region-material mapping inside a world material asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct TileMaterialData {
    pub tile: TerrainRegionId,
    pub layers_path: String,
}

/// World material data used by terrain rendering.
#[derive(Asset, Debug, Clone, Default, PartialEq, Reflect)]
pub struct WorldMaterialDataAsset {
    pub regions: Vec<TileMaterialData>,
    pub background_macro_material: SerializableMacroMaterialParams,
    pub foreground_macro_material: SerializableMacroMaterialParams,
    pub pom_height_bias: f32,
    pub pom_displacement: f32,
    pub pom_self_shadow_strength: f32,
}
