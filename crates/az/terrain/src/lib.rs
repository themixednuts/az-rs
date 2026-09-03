//! Native Azoth terrain authored-source schemas.
//!
//! Terrain source documents describe editable terrain worlds, reusable region
//! content, and layer sets. Asset builders process these sources into renderer,
//! physics, and streaming products; this crate stays schema-only so the editor
//! can expose terrain authoring without linking terrain runtime code.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod height;
mod layer;
mod region;
mod surface;
mod types;
mod world;

pub use height::{
    TerrainConstantHeightSource, TerrainHeightGraphSource, TerrainHeightImageSource,
    TerrainHeightSource, TerrainHeightTilesSource, TerrainHeightmapSource,
};
pub use layer::{SurfaceTag, TerrainLayer, TerrainLayerSetSource};
pub use region::TerrainRegionSource;
pub use surface::{
    TerrainImageChannel, TerrainSurfaceChannel, TerrainSurfaceGraphSource,
    TerrainSurfaceImageSource, TerrainSurfaceSource, TerrainSurfaceWeightsSource,
};
pub use types::{TerrainBounds, TerrainCoord, TerrainHeightRange, TerrainResolution};
pub use world::{TerrainRegionRef, TerrainWorldSource};

pub const TERRAIN_WORLD_SCHEMA_NAME: &str = "azoth.terrain.World";
pub const TERRAIN_REGION_SCHEMA_NAME: &str = "azoth.terrain.Region";
pub const TERRAIN_LAYER_SET_SCHEMA_NAME: &str = "azoth.terrain.LayerSet";
pub const TERRAIN_HEIGHTMAP_SCHEMA_NAME: &str = "azoth.terrain.Heightmap";
pub const TERRAIN_REGION_REF_SCHEMA_NAME: &str = "azoth.terrain.RegionRef";
pub const TERRAIN_COORD_SCHEMA_NAME: &str = "azoth.terrain.Coord";
pub const TERRAIN_BOUNDS_SCHEMA_NAME: &str = "azoth.terrain.Bounds";
pub const TERRAIN_HEIGHT_RANGE_SCHEMA_NAME: &str = "azoth.terrain.HeightRange";
pub const TERRAIN_RESOLUTION_SCHEMA_NAME: &str = "azoth.terrain.Resolution";
pub const TERRAIN_HEIGHT_SOURCE_SCHEMA_NAME: &str = "azoth.terrain.HeightSource";
pub const TERRAIN_HEIGHT_IMAGE_SCHEMA_NAME: &str = "azoth.terrain.HeightImage";
pub const TERRAIN_HEIGHT_TILES_SCHEMA_NAME: &str = "azoth.terrain.HeightTiles";
pub const TERRAIN_HEIGHT_GRAPH_SCHEMA_NAME: &str = "azoth.terrain.HeightGraph";
pub const TERRAIN_CONSTANT_HEIGHT_SCHEMA_NAME: &str = "azoth.terrain.ConstantHeight";
pub const TERRAIN_SURFACE_SOURCE_SCHEMA_NAME: &str = "azoth.terrain.SurfaceSource";
pub const TERRAIN_SURFACE_IMAGE_SCHEMA_NAME: &str = "azoth.terrain.SurfaceImage";
pub const TERRAIN_SURFACE_CHANNEL_SCHEMA_NAME: &str = "azoth.terrain.SurfaceChannel";
pub const TERRAIN_SURFACE_WEIGHTS_SCHEMA_NAME: &str = "azoth.terrain.SurfaceWeights";
pub const TERRAIN_SURFACE_GRAPH_SCHEMA_NAME: &str = "azoth.terrain.SurfaceGraph";
pub const TERRAIN_IMAGE_CHANNEL_SCHEMA_NAME: &str = "azoth.terrain.ImageChannel";
pub const TERRAIN_LAYER_SCHEMA_NAME: &str = "azoth.terrain.Layer";
pub const SURFACE_TAG_SCHEMA_NAME: &str = "azoth.terrain.SurfaceTag";

pub const TERRAIN_SOURCE_ROOT: &str = "project:source-root";
pub const TERRAIN_WORLD_PATH_PREFIX: &str = "terrain/worlds";
pub const TERRAIN_REGION_PATH_PREFIX: &str = "terrain/regions";
pub const TERRAIN_LAYER_SET_PATH_PREFIX: &str = "terrain/layers";
pub const TERRAIN_HEIGHTMAP_PATH_PREFIX: &str = "terrain/heights";
pub const TERRAIN_WORLD_EXTENSION: &str = "azterrain.ron";
pub const TERRAIN_REGION_EXTENSION: &str = "azterrain-region.ron";
pub const TERRAIN_LAYER_SET_EXTENSION: &str = "azterrain-layers.ron";
pub const TERRAIN_HEIGHTMAP_EXTENSION: &str = "azterrain-heightmap.ron";

pub const TERRAIN_WORLD_ASSET_TYPE_HINT: &str = "terrain-world";
pub const TERRAIN_REGION_ASSET_TYPE_HINT: &str = "terrain-region";
pub const TERRAIN_LAYER_SET_ASSET_TYPE_HINT: &str = "terrain-layer-set";
pub const TERRAIN_HEIGHT_ASSET_TYPE_HINT: &str = "terrain-height";
pub const TERRAIN_SURFACE_ASSET_TYPE_HINT: &str = "terrain-surface";
pub const TERRAIN_WATER_ASSET_TYPE_HINT: &str = "terrain-water";
pub const TERRAIN_GRAPH_ASSET_TYPE_HINT: &str = "azoth.terrain.graph";
pub const TERRAIN_SOURCE_SET_SCHEMA_NAME: &str = "azoth.terrain.SourceSet";
pub const MATERIAL_ASSET_TYPE_HINT: &str = "material";
pub const PHYSICS_MATERIAL_ASSET_TYPE_HINT: &str = "physics-material";
pub const TEXTURE_ASSET_TYPE_HINT: &str = "texture";

pub const UNRELEASED_TERRAIN_SCHEMA_VERSION_ERROR: &str = "Azoth is not released; terrain authored schemas must stay at version 1. Do not bump terrain schema versions until the first release defines migrations.";

pub const TERRAIN_SCHEMA_VERSION: u32 = assert_unreleased_terrain_schema_v1(1);
pub const TERRAIN_ENUM_SCHEMA_VERSION: u32 = assert_unreleased_terrain_schema_v1(1);

/// # Panics
///
/// Panics if `version` is anything but 1. Being a `const fn`, a bump in a
/// `const` initializer fails the build rather than the test run.
#[must_use]
pub const fn assert_unreleased_terrain_schema_v1(version: u32) -> u32 {
    assert!(
        version == 1,
        "Azoth is not released; terrain authored schemas must stay at version 1. Do not bump terrain schema versions until the first release defines migrations."
    );
    version
}
