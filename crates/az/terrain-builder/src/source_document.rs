use std::collections::BTreeSet;
use std::fmt::Display;
use std::str::FromStr;

use az_asset_builder::{ProductFormatId, SourceSchemaType};
use az_core::AssetType;
use az_terrain::{
    SurfaceTag as SourceSurfaceTag, TERRAIN_HEIGHTMAP_EXTENSION, TERRAIN_HEIGHTMAP_PATH_PREFIX,
    TERRAIN_HEIGHTMAP_SCHEMA_NAME, TERRAIN_LAYER_SET_EXTENSION, TERRAIN_LAYER_SET_PATH_PREFIX,
    TERRAIN_LAYER_SET_SCHEMA_NAME, TERRAIN_REGION_EXTENSION, TERRAIN_REGION_PATH_PREFIX,
    TERRAIN_REGION_SCHEMA_NAME, TERRAIN_WORLD_EXTENSION, TERRAIN_WORLD_PATH_PREFIX,
    TERRAIN_WORLD_SCHEMA_NAME, TerrainBounds as SourceBounds,
    TerrainConstantHeightSource as SourceConstantHeight, TerrainCoord as SourceCoord,
    TerrainHeightGraphSource as SourceHeightGraph, TerrainHeightImageSource as SourceHeightImage,
    TerrainHeightRange as SourceHeightRange, TerrainHeightSource as SourceHeight,
    TerrainHeightTilesSource as SourceHeightTiles, TerrainHeightmapSource,
    TerrainImageChannel as SourceImageChannel, TerrainLayer as SourceLayer, TerrainLayerSetSource,
    TerrainRegionRef as SourceRegionRef, TerrainRegionSource,
    TerrainResolution as SourceResolution, TerrainSurfaceChannel as SourceSurfaceChannel,
    TerrainSurfaceGraphSource as SourceSurfaceGraph,
    TerrainSurfaceImageSource as SourceSurfaceImage, TerrainSurfaceSource as SourceSurface,
    TerrainSurfaceWeightsSource as SourceSurfaceWeights, TerrainWorldSource,
};
use az_terrain_runtime::{
    SurfaceTag, TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION, TERRAIN_HEIGHTMAP_PRODUCT_VERSION,
    TERRAIN_LAYER_SET_PRODUCT_EXTENSION, TERRAIN_LAYER_SET_PRODUCT_VERSION,
    TERRAIN_REGION_PRODUCT_EXTENSION, TERRAIN_REGION_PRODUCT_VERSION,
    TERRAIN_WORLD_PRODUCT_EXTENSION, TERRAIN_WORLD_PRODUCT_VERSION, TerrainAssetCodecError,
    TerrainBounds, TerrainConstantHeightSource, TerrainCoord, TerrainHeightGraphSource,
    TerrainHeightImageSource, TerrainHeightRange, TerrainHeightSource, TerrainHeightTilesSource,
    TerrainHeightmapAsset, TerrainImageChannel, TerrainLayer, TerrainLayerSetAsset,
    TerrainRegionAsset, TerrainRegionRef, TerrainResolution, TerrainSurfaceChannel,
    TerrainSurfaceGraphSource, TerrainSurfaceImageSource, TerrainSurfaceSource,
    TerrainSurfaceWeightsSource, TerrainWorldAsset, encode_terrain_heightmap_asset,
    encode_terrain_layer_set_asset, encode_terrain_region_asset, encode_terrain_world_asset,
};
use glam::Vec2;
use ron::ser::PrettyConfig;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{
    TERRAIN_HEIGHTMAP_ASSET_TYPE, TERRAIN_HEIGHTMAP_FORMAT_ID, TERRAIN_LAYER_SET_ASSET_TYPE,
    TERRAIN_LAYER_SET_FORMAT_ID, TERRAIN_PRODUCT_SUB_ID, TERRAIN_REGION_ASSET_TYPE,
    TERRAIN_REGION_FORMAT_ID, TERRAIN_WORLD_ASSET_TYPE, TERRAIN_WORLD_FORMAT_ID,
    is_terrain_source_schema, terrain_product_path,
};

#[derive(Debug, Clone)]
pub struct TerrainProcessedProduct {
    pub product_path: String,
    pub asset_type: AssetType,
    pub format: ProductFormatId,
    pub format_version: u32,
    pub sub_id: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TerrainProcessError {
    #[error("unsupported terrain source schema `{schema}`")]
    UnsupportedSourceSchema { schema: String },
    #[error("decode typed RON terrain source `{schema}`: {source}")]
    Decode {
        schema: &'static str,
        source: ron::error::SpannedError,
    },
    #[error("terrain bounds `{field}` max must be greater than min")]
    InvalidBounds { field: String },
    #[error("terrain height range `{field}` max must be greater than min")]
    InvalidHeightRange { field: String },
    #[error("terrain resolution `{field}` spacing must be greater than zero")]
    InvalidResolution { field: String },
    #[error(
        "terrain heightmap sample count mismatch for {width} x {height}: expected {expected}, found {found}"
    )]
    HeightmapSampleCountMismatch {
        width: u32,
        height: u32,
        expected: usize,
        found: usize,
    },
    #[error("encode terrain product: {0}")]
    Encode(#[from] TerrainAssetCodecError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainSourceDocumentArtifact {
    pub path: String,
    pub schema: SourceSchemaType,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TerrainSourceDocumentError {
    #[error("encode typed RON terrain source: {0}")]
    Encode(#[from] ron::Error),
    #[error("terrain source asset path `{field}` is invalid: {reason}")]
    InvalidAssetPath { field: &'static str, reason: String },
    #[error(
        "terrain heightmap sample count mismatch for {width} x {height}: expected {expected}, found {found}"
    )]
    HeightmapSampleCountMismatch {
        width: u32,
        height: u32,
        expected: usize,
        found: usize,
    },
}

/// Emit a typed terrain-world source artifact from a neutral runtime payload.
///
/// # Errors
///
/// Returns an error if a referenced asset path is not well formed, or the
/// typed-RON encoding fails.
pub fn terrain_world_source_document_artifact(
    path: impl Into<String>,
    world: &TerrainWorldAsset,
) -> Result<TerrainSourceDocumentArtifact, TerrainSourceDocumentError> {
    let path = path.into();
    Ok(TerrainSourceDocumentArtifact {
        path,
        schema: SourceSchemaType::__from_static(TERRAIN_WORLD_SCHEMA_NAME),
        bytes: encode_terrain_world_source_document(world)?,
    })
}

/// Emit a typed terrain-region source artifact from a neutral runtime payload.
///
/// # Errors
///
/// Returns an error if a referenced asset path is not well formed, or the
/// typed-RON encoding fails.
pub fn terrain_region_source_document_artifact(
    path: impl Into<String>,
    region: &TerrainRegionAsset,
) -> Result<TerrainSourceDocumentArtifact, TerrainSourceDocumentError> {
    let path = path.into();
    Ok(TerrainSourceDocumentArtifact {
        path,
        schema: SourceSchemaType::__from_static(TERRAIN_REGION_SCHEMA_NAME),
        bytes: encode_terrain_region_source_document(region)?,
    })
}

/// Emit a typed terrain layer-set source artifact from a neutral runtime payload.
///
/// # Errors
///
/// Returns an error if a referenced asset path is not well formed, or the
/// typed-RON encoding fails.
pub fn terrain_layer_set_source_document_artifact(
    path: impl Into<String>,
    layer_set: &TerrainLayerSetAsset,
) -> Result<TerrainSourceDocumentArtifact, TerrainSourceDocumentError> {
    let path = path.into();
    Ok(TerrainSourceDocumentArtifact {
        path,
        schema: SourceSchemaType::__from_static(TERRAIN_LAYER_SET_SCHEMA_NAME),
        bytes: encode_terrain_layer_set_source_document(layer_set)?,
    })
}

/// Emit a typed terrain heightmap source artifact from a neutral runtime payload.
///
/// # Errors
///
/// Returns an error if `samples` does not hold `width * height` entries, or
/// the typed-RON encoding fails.
pub fn terrain_heightmap_source_document_artifact(
    path: impl Into<String>,
    heightmap: &TerrainHeightmapAsset,
) -> Result<TerrainSourceDocumentArtifact, TerrainSourceDocumentError> {
    let path = path.into();
    Ok(TerrainSourceDocumentArtifact {
        path,
        schema: SourceSchemaType::__from_static(TERRAIN_HEIGHTMAP_SCHEMA_NAME),
        bytes: encode_terrain_heightmap_source_document(heightmap)?,
    })
}

/// Encode a terrain world as its direct typed-RON source representation.
///
/// # Errors
///
/// Returns an error if the layer-set path or any region reference is not a
/// well-formed asset path, or the typed-RON encoding fails.
pub fn encode_terrain_world_source_document(
    world: &TerrainWorldAsset,
) -> Result<Vec<u8>, TerrainSourceDocumentError> {
    encode_source(&source_world(world)?)
}

/// Encode a terrain region as its direct typed-RON source representation.
///
/// # Errors
///
/// Returns an error if the height, surface, water, or layers path is not a
/// well-formed asset path, or the typed-RON encoding fails.
pub fn encode_terrain_region_source_document(
    region: &TerrainRegionAsset,
) -> Result<Vec<u8>, TerrainSourceDocumentError> {
    encode_source(&source_region(region)?)
}

/// Encode a terrain layer set as its direct typed-RON source representation.
///
/// # Errors
///
/// Returns an error if a layer references a path that is not a well-formed
/// asset path, or the typed-RON encoding fails.
pub fn encode_terrain_layer_set_source_document(
    layer_set: &TerrainLayerSetAsset,
) -> Result<Vec<u8>, TerrainSourceDocumentError> {
    encode_source(&source_layer_set(layer_set)?)
}

/// Encode a terrain heightmap as its direct typed-RON source representation.
///
/// # Errors
///
/// Returns an error if `samples` does not hold `width * height` entries, or
/// the typed-RON encoding fails.
pub fn encode_terrain_heightmap_source_document(
    heightmap: &TerrainHeightmapAsset,
) -> Result<Vec<u8>, TerrainSourceDocumentError> {
    validate_source_heightmap_sample_count(
        heightmap.width,
        heightmap.height,
        heightmap.samples.len(),
    )?;
    encode_source(&TerrainHeightmapSource {
        name: heightmap.name.clone(),
        width: heightmap.width,
        height: heightmap.height,
        samples: heightmap.samples.clone(),
    })
}

#[must_use]
pub fn terrain_world_source_path(world_name: &str) -> String {
    format!(
        "{}/{}.{}",
        TERRAIN_WORLD_PATH_PREFIX,
        terrain_source_segment(world_name),
        TERRAIN_WORLD_EXTENSION
    )
}

#[must_use]
pub fn terrain_region_source_path(world_name: &str, coord: TerrainCoord) -> String {
    format!(
        "{}/{}/{}.{}",
        TERRAIN_REGION_PATH_PREFIX,
        terrain_source_segment(world_name),
        terrain_region_coord_segment(coord),
        TERRAIN_REGION_EXTENSION
    )
}

#[must_use]
pub fn terrain_world_layers_source_path(world_name: &str) -> String {
    format!(
        "{}/{}/world.{}",
        TERRAIN_LAYER_SET_PATH_PREFIX,
        terrain_source_segment(world_name),
        TERRAIN_LAYER_SET_EXTENSION
    )
}

#[must_use]
pub fn terrain_region_layers_source_path(world_name: &str, coord: TerrainCoord) -> String {
    format!(
        "{}/{}/{}.{}",
        TERRAIN_LAYER_SET_PATH_PREFIX,
        terrain_source_segment(world_name),
        terrain_region_coord_segment(coord),
        TERRAIN_LAYER_SET_EXTENSION
    )
}

#[must_use]
pub fn terrain_heightmap_source_path(world_name: &str, coord: TerrainCoord) -> String {
    format!(
        "{}/{}/{}.{}",
        TERRAIN_HEIGHTMAP_PATH_PREFIX,
        terrain_source_segment(world_name),
        terrain_region_coord_segment(coord),
        TERRAIN_HEIGHTMAP_EXTENSION
    )
}

#[must_use]
pub fn terrain_region_display_name(world_name: &str, coord: TerrainCoord) -> String {
    format!(
        "{}/{}",
        terrain_source_segment(world_name),
        terrain_region_coord_segment(coord)
    )
}

#[must_use]
pub fn terrain_region_coord_segment(coord: TerrainCoord) -> String {
    format!("r_{:+03}_{:+03}", coord.x, coord.y)
}

#[must_use]
pub fn terrain_source_segment(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            segment
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+') {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Process one direct typed-RON terrain source into its native runtime product.
///
/// # Errors
///
/// Returns an error if `source_schema_type` is not a terrain source schema,
/// the typed RON fails to decode, the decoded bounds, height range, or
/// resolution are degenerate, a heightmap's sample count does not match
/// `width * height`, or the runtime product fails to encode.
pub fn process_terrain_source(
    source_path: &str,
    source_schema_type: &str,
    source_bytes: &[u8],
) -> Result<TerrainProcessedProduct, TerrainProcessError> {
    match source_schema_type {
        TERRAIN_WORLD_SCHEMA_NAME => {
            let source =
                decode_source::<TerrainWorldSource>(TERRAIN_WORLD_SCHEMA_NAME, source_bytes)?;
            let mut world = runtime_world(source)?;
            world.layers = terrain_runtime_asset_path(&world.layers);
            for region in &mut world.regions {
                region.asset = terrain_runtime_asset_path(&region.asset);
            }
            Ok(TerrainProcessedProduct {
                product_path: terrain_product_path(source_path, TERRAIN_WORLD_PRODUCT_EXTENSION),
                asset_type: TERRAIN_WORLD_ASSET_TYPE,
                format: TERRAIN_WORLD_FORMAT_ID,
                format_version: TERRAIN_WORLD_PRODUCT_VERSION,
                sub_id: TERRAIN_PRODUCT_SUB_ID,
                bytes: encode_terrain_world_asset(&world)?,
            })
        }
        TERRAIN_REGION_SCHEMA_NAME => {
            let source =
                decode_source::<TerrainRegionSource>(TERRAIN_REGION_SCHEMA_NAME, source_bytes)?;
            let mut region = runtime_region(source)?;
            rewrite_region_runtime_asset_paths(&mut region);
            Ok(TerrainProcessedProduct {
                product_path: terrain_product_path(source_path, TERRAIN_REGION_PRODUCT_EXTENSION),
                asset_type: TERRAIN_REGION_ASSET_TYPE,
                format: TERRAIN_REGION_FORMAT_ID,
                format_version: TERRAIN_REGION_PRODUCT_VERSION,
                sub_id: TERRAIN_PRODUCT_SUB_ID,
                bytes: encode_terrain_region_asset(&region)?,
            })
        }
        TERRAIN_LAYER_SET_SCHEMA_NAME => {
            let source = decode_source::<TerrainLayerSetSource>(
                TERRAIN_LAYER_SET_SCHEMA_NAME,
                source_bytes,
            )?;
            let layer_set = runtime_layer_set(source)?;
            Ok(TerrainProcessedProduct {
                product_path: terrain_product_path(
                    source_path,
                    TERRAIN_LAYER_SET_PRODUCT_EXTENSION,
                ),
                asset_type: TERRAIN_LAYER_SET_ASSET_TYPE,
                format: TERRAIN_LAYER_SET_FORMAT_ID,
                format_version: TERRAIN_LAYER_SET_PRODUCT_VERSION,
                sub_id: TERRAIN_PRODUCT_SUB_ID,
                bytes: encode_terrain_layer_set_asset(&layer_set)?,
            })
        }
        TERRAIN_HEIGHTMAP_SCHEMA_NAME => {
            let source = decode_source::<TerrainHeightmapSource>(
                TERRAIN_HEIGHTMAP_SCHEMA_NAME,
                source_bytes,
            )?;
            let heightmap = runtime_heightmap(source)?;
            Ok(TerrainProcessedProduct {
                product_path: terrain_product_path(
                    source_path,
                    TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION,
                ),
                asset_type: TERRAIN_HEIGHTMAP_ASSET_TYPE,
                format: TERRAIN_HEIGHTMAP_FORMAT_ID,
                format_version: TERRAIN_HEIGHTMAP_PRODUCT_VERSION,
                sub_id: TERRAIN_PRODUCT_SUB_ID,
                bytes: encode_terrain_heightmap_asset(&heightmap)?,
            })
        }
        schema => Err(TerrainProcessError::UnsupportedSourceSchema {
            schema: schema.to_string(),
        }),
    }
}

/// Resolve a typed terrain authoring reference to its processed product path.
#[must_use]
pub fn terrain_runtime_asset_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let product_extension = if normalized.ends_with(&format!(".{TERRAIN_WORLD_EXTENSION}")) {
        Some(TERRAIN_WORLD_PRODUCT_EXTENSION)
    } else if normalized.ends_with(&format!(".{TERRAIN_REGION_EXTENSION}")) {
        Some(TERRAIN_REGION_PRODUCT_EXTENSION)
    } else if normalized.ends_with(&format!(".{TERRAIN_LAYER_SET_EXTENSION}")) {
        Some(TERRAIN_LAYER_SET_PRODUCT_EXTENSION)
    } else if normalized.ends_with(&format!(".{TERRAIN_HEIGHTMAP_EXTENSION}")) {
        Some(TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION)
    } else {
        None
    };

    match product_extension {
        Some(extension) => terrain_product_path(&normalized, extension),
        None => normalized,
    }
}

fn rewrite_region_runtime_asset_paths(region: &mut TerrainRegionAsset) {
    match &mut region.height {
        TerrainHeightSource::Image(image) => image.image = terrain_runtime_asset_path(&image.image),
        TerrainHeightSource::Tiled(tiles) => tiles.asset = terrain_runtime_asset_path(&tiles.asset),
        TerrainHeightSource::Graph(graph) => graph.graph = terrain_runtime_asset_path(&graph.graph),
        TerrainHeightSource::Constant(_) => {}
    }
    if let Some(surface) = &mut region.surface {
        match surface {
            TerrainSurfaceSource::Image(image) => {
                image.image = terrain_runtime_asset_path(&image.image);
            }
            TerrainSurfaceSource::Weights(weights) => {
                weights.asset = terrain_runtime_asset_path(&weights.asset);
            }
            TerrainSurfaceSource::Graph(graph) => {
                graph.graph = terrain_runtime_asset_path(&graph.graph);
            }
        }
    }
    if let Some(water) = &mut region.water {
        *water = terrain_runtime_asset_path(water);
    }
    if let Some(layers) = &mut region.layers {
        *layers = terrain_runtime_asset_path(layers);
    }
}

/// Return direct source-file dependencies referenced by a typed terrain source.
///
/// # Errors
///
/// Returns an error if `source_schema_type` is not a terrain source schema, or
/// the typed RON fails to decode.
pub fn collect_terrain_source_dependencies(
    source_schema_type: &str,
    source_bytes: &[u8],
) -> Result<Vec<String>, TerrainProcessError> {
    if !is_terrain_source_schema(source_schema_type) {
        return Err(TerrainProcessError::UnsupportedSourceSchema {
            schema: source_schema_type.to_string(),
        });
    }

    let mut dependencies = BTreeSet::new();
    match source_schema_type {
        TERRAIN_WORLD_SCHEMA_NAME => {
            let world = runtime_world(decode_source(TERRAIN_WORLD_SCHEMA_NAME, source_bytes)?)?;
            push_non_empty_dependency(&mut dependencies, &world.layers);
            for region in &world.regions {
                push_non_empty_dependency(&mut dependencies, &region.asset);
            }
        }
        TERRAIN_REGION_SCHEMA_NAME => {
            let region = runtime_region(decode_source(TERRAIN_REGION_SCHEMA_NAME, source_bytes)?)?;
            collect_height_dependencies(&mut dependencies, &region.height);
            if let Some(surface) = &region.surface {
                collect_surface_dependencies(&mut dependencies, surface);
            }
            if let Some(water) = &region.water {
                push_non_empty_dependency(&mut dependencies, water);
            }
            if let Some(layers) = &region.layers {
                push_non_empty_dependency(&mut dependencies, layers);
            }
        }
        TERRAIN_LAYER_SET_SCHEMA_NAME => {
            let layer_set =
                runtime_layer_set(decode_source(TERRAIN_LAYER_SET_SCHEMA_NAME, source_bytes)?)?;
            for layer in &layer_set.layers {
                if let Some(material) = &layer.material {
                    push_non_empty_dependency(&mut dependencies, material);
                }
                if let Some(material) = &layer.physics_material {
                    push_non_empty_dependency(&mut dependencies, material);
                }
            }
        }
        TERRAIN_HEIGHTMAP_SCHEMA_NAME => {
            let _ = runtime_heightmap(decode_source(TERRAIN_HEIGHTMAP_SCHEMA_NAME, source_bytes)?)?;
        }
        _ => unreachable!("terrain schema was checked above"),
    }
    Ok(dependencies.into_iter().collect())
}

fn encode_source<T: Serialize>(source: &T) -> Result<Vec<u8>, TerrainSourceDocumentError> {
    Ok(ron::ser::to_string_pretty(source, PrettyConfig::default())?.into_bytes())
}

fn decode_source<T: DeserializeOwned>(
    schema: &'static str,
    source_bytes: &[u8],
) -> Result<T, TerrainProcessError> {
    ron::de::from_bytes(source_bytes)
        .map_err(|source| TerrainProcessError::Decode { schema, source })
}

fn source_asset_path<T>(path: &str, field: &'static str) -> Result<T, TerrainSourceDocumentError>
where
    T: FromStr,
    T::Err: Display,
{
    path.parse().map_err(
        |error: T::Err| TerrainSourceDocumentError::InvalidAssetPath {
            field,
            reason: error.to_string(),
        },
    )
}

fn source_optional_asset_path<T>(
    path: Option<&str>,
    field: &'static str,
) -> Result<Option<T>, TerrainSourceDocumentError>
where
    T: FromStr,
    T::Err: Display,
{
    path.map(|path| source_asset_path(path, field)).transpose()
}

fn source_world(
    world: &TerrainWorldAsset,
) -> Result<TerrainWorldSource, TerrainSourceDocumentError> {
    Ok(TerrainWorldSource {
        name: world.name.clone(),
        bounds: source_bounds(world.bounds),
        height_range: source_height_range(world.height_range),
        resolution: source_resolution(world.resolution),
        layers: source_asset_path(&world.layers, "layers")?,
        regions: world
            .regions
            .iter()
            .map(source_region_ref)
            .collect::<Result<_, _>>()?,
    })
}

fn source_region(
    region: &TerrainRegionAsset,
) -> Result<TerrainRegionSource, TerrainSourceDocumentError> {
    Ok(TerrainRegionSource {
        name: region.name.clone(),
        height: source_height(&region.height)?,
        surface: region.surface.as_ref().map(source_surface).transpose()?,
        water: source_optional_asset_path(region.water.as_deref(), "water")?,
        layers: source_optional_asset_path(region.layers.as_deref(), "layers")?,
    })
}

fn source_layer_set(
    layer_set: &TerrainLayerSetAsset,
) -> Result<TerrainLayerSetSource, TerrainSourceDocumentError> {
    Ok(TerrainLayerSetSource {
        name: layer_set.name.clone(),
        layers: layer_set
            .layers
            .iter()
            .map(source_layer)
            .collect::<Result<_, _>>()?,
    })
}

fn source_region_ref(
    region: &TerrainRegionRef,
) -> Result<SourceRegionRef, TerrainSourceDocumentError> {
    Ok(SourceRegionRef {
        asset: source_asset_path(&region.asset, "region asset")?,
        coord: region.coord.map(source_coord),
        bounds: source_bounds(region.bounds),
        priority: region.priority,
    })
}

fn source_layer(layer: &TerrainLayer) -> Result<SourceLayer, TerrainSourceDocumentError> {
    Ok(SourceLayer {
        tag: SourceSurfaceTag {
            name: layer.tag.name.clone(),
        },
        priority: layer.priority,
        material: source_optional_asset_path(layer.material.as_deref(), "material")?,
        physics_material: source_optional_asset_path(
            layer.physics_material.as_deref(),
            "physics material",
        )?,
        texture_scale: layer.texture_scale,
    })
}

fn source_height(source: &TerrainHeightSource) -> Result<SourceHeight, TerrainSourceDocumentError> {
    Ok(match source {
        TerrainHeightSource::Image(image) => SourceHeight::Image(SourceHeightImage {
            image: source_asset_path(&image.image, "height image")?,
            channel: source_image_channel(image.channel),
            mip: image.mip,
            tiling: image.tiling,
        }),
        TerrainHeightSource::Tiled(tiles) => SourceHeight::Tiled(SourceHeightTiles {
            asset: source_asset_path(&tiles.asset, "height asset")?,
        }),
        TerrainHeightSource::Graph(graph) => SourceHeight::Graph(SourceHeightGraph {
            graph: source_asset_path(&graph.graph, "height graph")?,
        }),
        TerrainHeightSource::Constant(constant) => SourceHeight::Constant(SourceConstantHeight {
            value: constant.value,
        }),
    })
}

fn source_surface(
    source: &TerrainSurfaceSource,
) -> Result<SourceSurface, TerrainSourceDocumentError> {
    Ok(match source {
        TerrainSurfaceSource::Image(image) => SourceSurface::Image(SourceSurfaceImage {
            image: source_asset_path(&image.image, "surface image")?,
            mip: image.mip,
            tiling: image.tiling,
            channels: image
                .channels
                .iter()
                .map(|channel| SourceSurfaceChannel {
                    channel: source_image_channel(channel.channel),
                    tag: SourceSurfaceTag {
                        name: channel.tag.name.clone(),
                    },
                })
                .collect(),
        }),
        TerrainSurfaceSource::Weights(weights) => SourceSurface::Weights(SourceSurfaceWeights {
            asset: source_asset_path(&weights.asset, "surface weights")?,
        }),
        TerrainSurfaceSource::Graph(graph) => SourceSurface::Graph(SourceSurfaceGraph {
            graph: source_asset_path(&graph.graph, "surface graph")?,
        }),
    })
}

const fn source_coord(coord: TerrainCoord) -> SourceCoord {
    SourceCoord {
        x: coord.x,
        y: coord.y,
    }
}

const fn source_bounds(bounds: TerrainBounds) -> SourceBounds {
    SourceBounds {
        min: bounds.min,
        max: bounds.max,
    }
}

const fn source_height_range(range: TerrainHeightRange) -> SourceHeightRange {
    SourceHeightRange {
        min: range.min,
        max: range.max,
    }
}

const fn source_resolution(resolution: TerrainResolution) -> SourceResolution {
    SourceResolution {
        height_spacing: resolution.height_spacing,
        surface_spacing: resolution.surface_spacing,
    }
}

const fn source_image_channel(channel: TerrainImageChannel) -> SourceImageChannel {
    match channel {
        TerrainImageChannel::Red => SourceImageChannel::Red,
        TerrainImageChannel::Green => SourceImageChannel::Green,
        TerrainImageChannel::Blue => SourceImageChannel::Blue,
        TerrainImageChannel::Alpha => SourceImageChannel::Alpha,
        TerrainImageChannel::Luminance => SourceImageChannel::Luminance,
    }
}

fn runtime_world(source: TerrainWorldSource) -> Result<TerrainWorldAsset, TerrainProcessError> {
    let bounds = runtime_bounds(source.bounds, "bounds")?;
    let height_range = runtime_height_range(source.height_range, "height_range")?;
    let resolution = runtime_resolution(source.resolution, "resolution")?;
    let regions = source
        .regions
        .into_iter()
        .map(runtime_region_ref)
        .collect::<Result<_, _>>()?;
    Ok(TerrainWorldAsset {
        name: source.name,
        bounds,
        height_range,
        resolution,
        layers: source.layers.into_string(),
        regions,
    })
}

fn runtime_region(source: TerrainRegionSource) -> Result<TerrainRegionAsset, TerrainProcessError> {
    Ok(TerrainRegionAsset {
        name: source.name,
        height: runtime_height(source.height)?,
        surface: source.surface.map(runtime_surface).transpose()?,
        water: source.water.map(az_core::AssetPathBuf::into_string),
        layers: source.layers.map(az_core::AssetPathBuf::into_string),
    })
}

fn runtime_layer_set(
    source: TerrainLayerSetSource,
) -> Result<TerrainLayerSetAsset, TerrainProcessError> {
    Ok(TerrainLayerSetAsset {
        name: source.name,
        layers: source
            .layers
            .into_iter()
            .map(runtime_layer)
            .collect::<Result<_, _>>()?,
    })
}

fn runtime_heightmap(
    source: TerrainHeightmapSource,
) -> Result<TerrainHeightmapAsset, TerrainProcessError> {
    validate_processed_heightmap_sample_count(source.width, source.height, source.samples.len())?;
    Ok(TerrainHeightmapAsset {
        name: source.name,
        width: source.width,
        height: source.height,
        samples: source.samples,
    })
}

fn runtime_region_ref(source: SourceRegionRef) -> Result<TerrainRegionRef, TerrainProcessError> {
    let bounds = runtime_bounds(source.bounds, "regions[]")?;
    Ok(TerrainRegionRef {
        asset: source.asset.into_string(),
        coord: source.coord.map(runtime_coord),
        bounds,
        priority: source.priority,
    })
}

fn runtime_layer(source: SourceLayer) -> Result<TerrainLayer, TerrainProcessError> {
    if source.texture_scale <= 0.0 {
        return Err(TerrainProcessError::InvalidResolution {
            field: "layers[]".to_string(),
        });
    }
    Ok(TerrainLayer {
        tag: SurfaceTag {
            name: source.tag.name,
        },
        priority: source.priority,
        material: source.material.map(az_core::AssetPathBuf::into_string),
        physics_material: source
            .physics_material
            .map(az_core::AssetPathBuf::into_string),
        texture_scale: source.texture_scale,
    })
}

fn runtime_height(source: SourceHeight) -> Result<TerrainHeightSource, TerrainProcessError> {
    Ok(match source {
        SourceHeight::Image(image) => {
            validate_tiling(image.tiling, "height")?;
            TerrainHeightSource::Image(TerrainHeightImageSource {
                image: image.image.into_string(),
                channel: runtime_image_channel(image.channel),
                mip: image.mip,
                tiling: image.tiling,
            })
        }
        SourceHeight::Tiled(tiles) => TerrainHeightSource::Tiled(TerrainHeightTilesSource {
            asset: tiles.asset.into_string(),
        }),
        SourceHeight::Graph(graph) => TerrainHeightSource::Graph(TerrainHeightGraphSource {
            graph: graph.graph.into_string(),
        }),
        SourceHeight::Constant(constant) => {
            TerrainHeightSource::Constant(TerrainConstantHeightSource {
                value: constant.value,
            })
        }
    })
}

fn runtime_surface(source: SourceSurface) -> Result<TerrainSurfaceSource, TerrainProcessError> {
    Ok(match source {
        SourceSurface::Image(image) => {
            validate_tiling(image.tiling, "surface")?;
            TerrainSurfaceSource::Image(TerrainSurfaceImageSource {
                image: image.image.into_string(),
                mip: image.mip,
                tiling: image.tiling,
                channels: image
                    .channels
                    .into_iter()
                    .map(|channel| TerrainSurfaceChannel {
                        channel: runtime_image_channel(channel.channel),
                        tag: SurfaceTag {
                            name: channel.tag.name,
                        },
                    })
                    .collect(),
            })
        }
        SourceSurface::Weights(weights) => {
            TerrainSurfaceSource::Weights(TerrainSurfaceWeightsSource {
                asset: weights.asset.into_string(),
            })
        }
        SourceSurface::Graph(graph) => TerrainSurfaceSource::Graph(TerrainSurfaceGraphSource {
            graph: graph.graph.into_string(),
        }),
    })
}

const fn runtime_coord(coord: SourceCoord) -> TerrainCoord {
    TerrainCoord {
        x: coord.x,
        y: coord.y,
    }
}

fn runtime_bounds(bounds: SourceBounds, field: &str) -> Result<TerrainBounds, TerrainProcessError> {
    let bounds = TerrainBounds {
        min: bounds.min,
        max: bounds.max,
    };
    validate_bounds(&bounds, field)?;
    Ok(bounds)
}

fn runtime_height_range(
    range: SourceHeightRange,
    field: &str,
) -> Result<TerrainHeightRange, TerrainProcessError> {
    let range = TerrainHeightRange {
        min: range.min,
        max: range.max,
    };
    if range.max <= range.min {
        return Err(TerrainProcessError::InvalidHeightRange {
            field: field.to_string(),
        });
    }
    Ok(range)
}

fn runtime_resolution(
    resolution: SourceResolution,
    field: &str,
) -> Result<TerrainResolution, TerrainProcessError> {
    let resolution = TerrainResolution {
        height_spacing: resolution.height_spacing,
        surface_spacing: resolution.surface_spacing,
    };
    if resolution.height_spacing <= 0.0 || resolution.surface_spacing <= 0.0 {
        return Err(TerrainProcessError::InvalidResolution {
            field: field.to_string(),
        });
    }
    Ok(resolution)
}

const fn runtime_image_channel(channel: SourceImageChannel) -> TerrainImageChannel {
    match channel {
        SourceImageChannel::Red => TerrainImageChannel::Red,
        SourceImageChannel::Green => TerrainImageChannel::Green,
        SourceImageChannel::Blue => TerrainImageChannel::Blue,
        SourceImageChannel::Alpha => TerrainImageChannel::Alpha,
        SourceImageChannel::Luminance => TerrainImageChannel::Luminance,
    }
}

fn validate_bounds(bounds: &TerrainBounds, field: &str) -> Result<(), TerrainProcessError> {
    if bounds.max.x <= bounds.min.x || bounds.max.y <= bounds.min.y {
        return Err(TerrainProcessError::InvalidBounds {
            field: field.to_string(),
        });
    }
    Ok(())
}

fn validate_tiling(value: Vec2, field: &str) -> Result<(), TerrainProcessError> {
    if value.x <= 0.0 || value.y <= 0.0 {
        return Err(TerrainProcessError::InvalidResolution {
            field: field.to_string(),
        });
    }
    Ok(())
}

fn validate_processed_heightmap_sample_count(
    width: u32,
    height: u32,
    found: usize,
) -> Result<(), TerrainProcessError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(usize::MAX);
    if expected != found {
        return Err(TerrainProcessError::HeightmapSampleCountMismatch {
            width,
            height,
            expected,
            found,
        });
    }
    Ok(())
}

fn validate_source_heightmap_sample_count(
    width: u32,
    height: u32,
    found: usize,
) -> Result<(), TerrainSourceDocumentError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(usize::MAX);
    if expected != found {
        return Err(TerrainSourceDocumentError::HeightmapSampleCountMismatch {
            width,
            height,
            expected,
            found,
        });
    }
    Ok(())
}

fn push_non_empty_dependency(dependencies: &mut BTreeSet<String>, path: &str) {
    if !path.is_empty() {
        dependencies.insert(path.to_string());
    }
}

fn collect_height_dependencies(dependencies: &mut BTreeSet<String>, source: &TerrainHeightSource) {
    match source {
        TerrainHeightSource::Image(source) => {
            push_non_empty_dependency(dependencies, &source.image);
        }
        TerrainHeightSource::Tiled(source) => {
            push_non_empty_dependency(dependencies, &source.asset);
        }
        TerrainHeightSource::Graph(source) => {
            push_non_empty_dependency(dependencies, &source.graph);
        }
        TerrainHeightSource::Constant(_) => {}
    }
}

fn collect_surface_dependencies(
    dependencies: &mut BTreeSet<String>,
    source: &TerrainSurfaceSource,
) {
    match source {
        TerrainSurfaceSource::Image(source) => {
            push_non_empty_dependency(dependencies, &source.image);
        }
        TerrainSurfaceSource::Weights(source) => {
            push_non_empty_dependency(dependencies, &source.asset);
        }
        TerrainSurfaceSource::Graph(source) => {
            push_non_empty_dependency(dependencies, &source.graph);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fmt::Debug, str::FromStr};

    use az_terrain_runtime::{
        TerrainHeightSource as RuntimeHeightSource, TerrainSurfaceSource as RuntimeSurfaceSource,
        decode_terrain_layer_set_asset, decode_terrain_region_asset, decode_terrain_world_asset,
    };

    use super::*;

    fn asset<T>(path: &str) -> T
    where
        T: FromStr,
        T::Err: Debug,
    {
        path.parse().unwrap()
    }

    #[test]
    fn processes_world_source_to_native_product() {
        let source = TerrainWorldSource {
            name: "open-world".to_string(),
            bounds: SourceBounds {
                min: Vec2::ZERO,
                max: Vec2::splat(2048.0),
            },
            height_range: SourceHeightRange {
                min: -100.0,
                max: 500.0,
            },
            resolution: SourceResolution {
                height_spacing: 1.0,
                surface_spacing: 2.0,
            },
            layers: asset("terrain/layers/base.azterrain-layers.ron"),
            regions: vec![SourceRegionRef {
                asset: asset("terrain/regions/r0.azterrain-region.ron"),
                coord: Some(SourceCoord { x: 0, y: 1 }),
                bounds: SourceBounds {
                    min: Vec2::ZERO,
                    max: Vec2::splat(512.0),
                },
                priority: 0,
            }],
        };
        let bytes = encode_source(&source).unwrap();
        let product = process_terrain_source(
            "terrain/worlds/main.azterrain.ron",
            TERRAIN_WORLD_SCHEMA_NAME,
            &bytes,
        )
        .unwrap();
        assert_eq!(
            product.product_path,
            "terrain/worlds/main.azterrain-world.bin"
        );
        let decoded = decode_terrain_world_asset(&product.bytes).unwrap();
        assert_eq!(decoded.name, "open-world");
        assert_eq!(decoded.regions[0].coord.unwrap().y, 1);
        assert_eq!(
            decoded.layers,
            "terrain/layers/base.azterrain-layer-set.bin"
        );
        assert_eq!(
            decoded.regions[0].asset,
            "terrain/regions/r0.azterrain-region.bin"
        );
        assert_eq!(
            collect_terrain_source_dependencies(TERRAIN_WORLD_SCHEMA_NAME, &bytes).unwrap(),
            vec![
                "terrain/layers/base.azterrain-layers.ron".to_string(),
                "terrain/regions/r0.azterrain-region.ron".to_string(),
            ]
        );
    }

    #[test]
    fn processes_region_source_to_native_product() {
        let source = TerrainRegionSource {
            name: "region-0".to_string(),
            height: SourceHeight::Image(SourceHeightImage {
                image: asset("textures/terrain/height.tif"),
                channel: SourceImageChannel::Red,
                mip: 0,
                tiling: Vec2::ONE,
            }),
            surface: Some(SourceSurface::Graph(SourceSurfaceGraph {
                graph: asset("graphs/terrain/surface.azgraph.ron"),
            })),
            water: Some(asset("terrain/water/default.water.ron")),
            layers: None,
        };
        let bytes = encode_source(&source).unwrap();
        let product = process_terrain_source(
            "terrain/regions/r0.azterrain-region.ron",
            TERRAIN_REGION_SCHEMA_NAME,
            &bytes,
        )
        .unwrap();
        let decoded = decode_terrain_region_asset(&product.bytes).unwrap();
        assert_eq!(decoded.name, "region-0");
        assert!(matches!(decoded.height, RuntimeHeightSource::Image(_)));
        assert!(matches!(
            decoded.surface,
            Some(RuntimeSurfaceSource::Graph(_))
        ));
        assert_eq!(
            collect_terrain_source_dependencies(TERRAIN_REGION_SCHEMA_NAME, &bytes).unwrap(),
            vec![
                "graphs/terrain/surface.azgraph.ron".to_string(),
                "terrain/water/default.water.ron".to_string(),
                "textures/terrain/height.tif".to_string(),
            ]
        );
    }

    #[test]
    fn processes_layer_set_source_to_native_product() {
        let source = TerrainLayerSetSource {
            name: "base".to_string(),
            layers: vec![SourceLayer {
                tag: SourceSurfaceTag {
                    name: "grass".to_string(),
                },
                priority: 0,
                material: Some(asset("materials/terrain/grass.azmaterial.ron")),
                physics_material: None,
                texture_scale: 1.0,
            }],
        };
        let bytes = encode_source(&source).unwrap();
        let product = process_terrain_source(
            "terrain/layers/base.azterrain-layers.ron",
            TERRAIN_LAYER_SET_SCHEMA_NAME,
            &bytes,
        )
        .unwrap();
        let decoded = decode_terrain_layer_set_asset(&product.bytes).unwrap();
        assert_eq!(decoded.layers[0].tag.name, "grass");
        assert_eq!(
            collect_terrain_source_dependencies(TERRAIN_LAYER_SET_SCHEMA_NAME, &bytes).unwrap(),
            vec!["materials/terrain/grass.azmaterial.ron".to_string()]
        );
    }

    #[test]
    fn source_artifact_round_trips_direct_typed_ron() {
        let heightmap = TerrainHeightmapAsset {
            name: "h0".to_string(),
            width: 2,
            height: 2,
            samples: vec![0, 1, 2, 3],
        };
        let bytes = encode_terrain_heightmap_source_document(&heightmap).unwrap();
        let decoded: TerrainHeightmapSource = ron::de::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.samples, heightmap.samples);
    }

    #[test]
    fn terrain_runtime_paths_rewrite_only_typed_terrain_sources() {
        assert_eq!(
            terrain_runtime_asset_path("terrain/worlds/main.azterrain.ron"),
            "terrain/worlds/main.azterrain-world.bin"
        );
        assert_eq!(
            terrain_runtime_asset_path("terrain/regions/r0.azterrain-region.ron"),
            "terrain/regions/r0.azterrain-region.bin"
        );
        assert_eq!(
            terrain_runtime_asset_path("terrain/heights/r0.azterrain-heightmap.ron"),
            "terrain/heights/r0.azterrain-height.bin"
        );
        assert_eq!(
            terrain_runtime_asset_path("textures/terrain/height.exr"),
            "textures/terrain/height.exr"
        );
    }
}
