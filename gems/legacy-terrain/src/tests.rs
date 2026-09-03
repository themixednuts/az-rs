use super::*;
use crate::render::material::terrain_water_material;
use bevy::mesh::Indices;

#[test]
fn terrain_info_defaults_match_lumberyard_runtime_defaults() {
    let info = LegacyTerrainInfo::default();

    assert_eq!(info.unit_size_in_meters, DEFAULT_TERRAIN_UNIT_SIZE_METERS);
    assert_eq!(info.terrain_size_in_meters(), DEFAULT_TERRAIN_SIZE_METERS);
    assert_eq!(
        info.sector_size_in_meters,
        DEFAULT_TERRAIN_SECTOR_SIZE_METERS
    );
    assert_eq!(
        info.sectors_table_size_in_sectors,
        DEFAULT_TERRAIN_SECTORS_TABLE_SIZE
    );
    assert_eq!(info.sector_size_in_units(), 32);
}

#[test]
fn type_ids_match_lumberyard_source() {
    assert_eq!(
        LEGACY_TERRAIN_LEVEL_CONFIG_TYPE_ID,
        "65950218-99C8-4DF5-A949-6642A8C69444"
    );
    assert_eq!(
        LEGACY_TERRAIN_LEVEL_COMPONENT_TYPE_ID,
        "9BA33CA7-07DF-409F-A240-5A7AA67EFFE8"
    );
    assert_eq!(
        LEGACY_TERRAIN_MODULE_TYPE_ID,
        "4774487F-71DC-4CA4-943C-A31CE05D0616"
    );
    assert_eq!(TERRAIN_NODE_CHUNK_VERSION, 8);
}

#[test]
fn type_ids_match_serialized_type_contract() {
    assert_eq!(
        TERRAIN_INFO_COMPONENT_TYPE_ID,
        "2041EB3F-62B8-491A-914E-4DAF1F1D70A2"
    );
    assert_eq!(
        TERRAIN_INFO_SERVER_FACET_TYPE_ID,
        "4FB54D01-E8CE-45C2-A4ED-1059401C6168"
    );
    assert_eq!(
        SURFACE_MAP_DATA_ASSET_TYPE_ID,
        "0F9D3341-6C8D-4DD1-A636-3622878FF8F6"
    );
    assert_eq!(
        SURFACE_MAP_ASSET_HANDLER_TYPE_ID,
        "89C47C7F-424D-4557-859F-F0BB0DFD04E4"
    );
    assert_eq!(
        TERRAIN_MATERIAL_LAYER_DATA_TYPE_ID,
        "180454CF-AD7E-440B-91F9-A071574422F4"
    );
    assert_eq!(
        WORLD_MATERIAL_DATA_ASSET_TYPE_ID,
        "0C5DEBF7-4320-42AB-B77B-B7270D04206A"
    );
    assert_eq!(
        WORLD_MATERIAL_MANAGER_TYPE_ID,
        "51AA2DE7-CD24-45D2-9C8B-FD84FA4BD12D"
    );
    assert_eq!(
        SERIALIZABLE_WATER_QUADTREE_TYPE_ID,
        "23082A77-84B8-423E-B4CD-F601AA5D1D44"
    );
    assert_eq!(
        WATER_NODE_DATA_TYPE_ID,
        "79BCCE0C-D451-47C0-B2A1-5CAD1D7313BD"
    );
}

#[test]
fn surface_weight_preserves_hole_and_undefined_ids() {
    let undefined = TerrainSurfaceWeight::default();
    let hole = TerrainSurfaceWeight::HOLE;

    assert_eq!(undefined.primary_id(), TERRAIN_SURFACE_UNDEFINED_ID);
    assert!(!undefined.is_hole());
    assert_eq!(hole.primary_id(), TERRAIN_SURFACE_HOLE_ID);
    assert!(hole.is_hole());
}

// Heights are `sample * 0.5 - 5.0` over samples 0..=30 and the bilinear
// midpoint of two of them: exact halves of small integers, and pinning them
// exactly is the point of the test.
#[allow(clippy::float_cmp)]
#[test]
fn region_heightmap_samples_scaled_heights() {
    let heightmap = RegionHeightmap::from_samples(vec![0, 10, 20, 30], 2, 2.0, 0.5, -5.0).unwrap();

    assert_eq!(heightmap.height_at(1, 1), Some(10.0));
    assert_eq!(heightmap.bilinear_height(0.5, 0.5), Some(2.5));
    assert_eq!(heightmap.max_height, 10.0);
}

// `cell_size()` returns the `2.0` handed to `from_samples`, unchanged.
#[allow(clippy::float_cmp)]
#[test]
fn terrain_region_provider_maps_world_to_heightmap() {
    let heightmap = RegionHeightmap::from_samples(vec![0, 10, 20, 30], 2, 2.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        origin: Vec2::new(100.0, 200.0),
        heightmap,
        surface_resolution: 2,
        surface_weights: vec![TerrainSurfaceWeight::default(); 4],
        ..default()
    };

    assert_eq!(region.cell_size(), 2.0);
    assert_eq!(region.height_at_world(101.0, 201.0), Some(15.0));
    assert_eq!(
        region
            .surface_weight_at(1, 1)
            .map(TerrainSurfaceWeight::primary_id),
        Some(TERRAIN_SURFACE_UNDEFINED_ID)
    );
}

#[test]
fn terrain_region_provider_calculates_surface_normals() {
    let heightmap = RegionHeightmap::from_samples(vec![0, 0, 0, 0], 2, 2.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        origin: Vec2::new(100.0, 200.0),
        heightmap,
        ..default()
    };

    assert_eq!(region.normal_at_world(101.0, 201.0), Some(Vec3::Y));
    assert_eq!(region.normal_at_world(99.0, 201.0), None);
}

#[test]
fn terrain_region_provider_maps_world_to_surface_weights() {
    let heightmap = RegionHeightmap::from_samples(vec![0, 0, 0, 0], 2, 2.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        origin: Vec2::new(100.0, 200.0),
        heightmap,
        surface_resolution: 2,
        surface_weights: vec![
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::HOLE,
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::default(),
        ],
        ..default()
    };

    assert!(region.contains_world_position(103.0, 201.0));
    assert_eq!(
        region.surface_weight_at_world(103.0, 201.0),
        Some(TerrainSurfaceWeight::HOLE)
    );
    assert_eq!(region.surface_weight_at_world(104.0, 201.0), None);
}

#[test]
fn terrain_world_queries_loaded_region_data() {
    let mut app = App::new();
    app.init_resource::<Assets<TerrainRegionAsset>>();

    let heightmap = RegionHeightmap::from_samples(vec![0, 10, 20, 30], 2, 2.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        origin: Vec2::new(100.0, 200.0),
        heightmap,
        surface_resolution: 1,
        surface_weights: vec![TerrainSurfaceWeight::HOLE],
        ..default()
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<TerrainRegionAsset>>()
        .add(region);
    let mut terrain_world = TerrainWorld::default();
    terrain_world
        .loaded_regions
        .insert(TerrainRegionId::new(0, 0), handle);

    let terrain_assets = app.world().resource::<Assets<TerrainRegionAsset>>();
    assert_eq!(
        terrain_world.height_at_world(101.0, 201.0, terrain_assets),
        Some(15.0)
    );
    assert_eq!(
        terrain_world
            .normal_at_world(101.0, 201.0, terrain_assets)
            .map(|normal| normal.length().round()),
        Some(1.0)
    );
    assert_eq!(
        terrain_world.surface_weight_at_world(101.0, 201.0, terrain_assets),
        Some(TerrainSurfaceWeight::HOLE)
    );
    assert_eq!(
        terrain_world.height_at_world(99.0, 201.0, terrain_assets),
        None
    );
}

// The asserted sizes are `resolution * cell_size` = `2 * 2.0`: an exact
// product of small integers that the test exists to pin.
#[allow(clippy::float_cmp)]
#[test]
fn terrain_region_asset_builds_render_bundle_from_region_heightmap() {
    let heightmap = RegionHeightmap::from_samples(vec![0, 10, 20, 30], 2, 2.0, 1.0, 0.0).unwrap();
    let region =
        TerrainRegionAsset::from_region_heightmap(TerrainRegionId::new(2, 3), 2048.0, heightmap);

    let bundle = region.render_bundle_with_resolution(2).unwrap();

    assert_eq!(region.origin, Vec2::new(4096.0, 6144.0));
    assert_eq!(region.region_world_size(), 4.0);
    assert_eq!(bundle.chunk.size, 4.0);
    assert_eq!(bundle.chunk.resolution, 2);
    assert_eq!(bundle.transform.translation, Vec3::new(4098.0, 0.0, 6146.0));
    assert_eq!(bundle.heightmap.get_height(1, 1), Some(30.0));
}

#[test]
fn terrain_mesh_resolution_policy_resolves_against_region_heightmap() {
    assert_eq!(TerrainMeshResolution::Native.resolve(513), 513);
    assert_eq!(TerrainMeshResolution::Max(257).resolve(513), 257);
    assert_eq!(TerrainMeshResolution::Max(1025).resolve(513), 513);
    assert_eq!(TerrainMeshResolution::Fixed(129).resolve(513), 129);
    assert_eq!(TerrainMeshResolution::Fixed(0).resolve(513), 2);
    assert_eq!(TerrainMeshResolution::Native.resolve(0), 2);
}

#[test]
fn terrain_region_asset_builds_water_surface_mesh() {
    let region = TerrainRegionAsset {
        origin: Vec2::new(10.0, 20.0),
        water_quadtree: Some(SerializableWaterQuadtree {
            region_size: 8,
            quadtree_nodes: vec![WaterNodeData {
                height: 4.0,
                floor_height: 0.0,
                flags: WaterNodeFlags::from_bits(0x0a00_0000),
            }],
        }),
        ..default()
    };

    let mesh = region.water_surface_mesh(0.25).unwrap();

    assert_eq!(region.water_region_world_size(), Some(8.0));
    assert_eq!(mesh.count_vertices(), 4);
    assert!(matches!(mesh.indices(), Some(Indices::U32(indices)) if indices.len() == 6));
}

#[test]
fn terrain_region_asset_exposes_surface_map() {
    let region = TerrainRegionAsset {
        surface_resolution: 2,
        surface_weights: vec![
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::HOLE,
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::default(),
        ],
        ..default()
    };

    let surface_map = region.surface_map().unwrap();

    assert_eq!(surface_map.resolution, 2);
    assert_eq!(
        surface_map.surface_weight_at(1, 0),
        Some(TerrainSurfaceWeight::HOLE)
    );
}

// The color channels are already compared against a tolerance; alpha is the
// exception because both layers carry alpha `1.0`, so the weighted average is
// exactly `1.0` and asserting that is the point.
#[allow(clippy::float_cmp)]
#[test]
fn terrain_surface_palette_blends_weighted_layers() {
    let palette = TerrainSurfacePalette {
        default_color: [0.2, 0.3, 0.4, 1.0],
        layer_colors: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]],
    };
    let color = palette.color_for_weight(TerrainSurfaceWeight {
        ids: [0, 1, TERRAIN_SURFACE_UNDEFINED_ID],
        weights: [128, 127, 0],
    });

    assert!((color[0] - (128.0 / 255.0)).abs() < 0.001);
    assert!((color[2] - (127.0 / 255.0)).abs() < 0.001);
    assert_eq!(color[3], 1.0);
}

#[test]
fn terrain_region_asset_selects_render_material_path_for_region() {
    let region = TerrainRegionAsset {
        region: TerrainRegionId::new(2, 3),
        default_material_path: "materials/terrain/default.mtl".to_string(),
        material_layers: vec![
            TerrainMaterialLayerData {
                material_path: "materials/terrain/other.mtl".to_string(),
                affected_tiles: 0,
                priority: 20,
                ..default()
            },
            TerrainMaterialLayerData {
                material_path: "materials/terrain/base.mtl".to_string(),
                affected_tiles: 0,
                priority: 1,
                ..default()
            },
            TerrainMaterialLayerData {
                material_path: "materials/terrain/local.mtl".to_string(),
                affected_tiles: 0,
                priority: 10,
                ..default()
            },
        ],
        ..default()
    };

    assert_eq!(
        region.render_material_path(),
        Some("materials/terrain/default.mtl")
    );
}

#[test]
fn terrain_region_asset_falls_back_to_first_material_layer() {
    let region = TerrainRegionAsset {
        material_layers: vec![TerrainMaterialLayerData {
            material_path: "materials/terrain/layer.mtl".to_string(),
            ..default()
        }],
        ..default()
    };

    assert_eq!(
        region.render_material_path(),
        Some("materials/terrain/layer.mtl")
    );
}

#[test]
fn terrain_water_material_uses_transparency() {
    let material = terrain_water_material(&TerrainWaterRenderConfig::default());

    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert!(material.reflectance > 0.0);
}

#[test]
fn terrain_mesh_skips_hole_surface_quads() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .add_systems(Update, generate_terrain_meshes);

    let entity = app
        .world_mut()
        .spawn((
            TerrainChunkBundle {
                chunk: TerrainChunk::new(Vec3::ZERO, 2.0, 2),
                heightmap: Heightmap::flat(2, 0.0),
                ..Default::default()
            },
            TerrainSurfaceMap::new(1, vec![TerrainSurfaceWeight::HOLE]).unwrap(),
        ))
        .id();

    app.update();

    let mesh = app
        .world()
        .entity(entity)
        .get::<Mesh3d>()
        .unwrap()
        .0
        .clone();
    assert_eq!(mesh_index_count(app.world(), &mesh), Some(0));
}

#[test]
fn terrain_mesh_uses_surface_palette_vertex_colors() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .add_systems(Update, generate_terrain_meshes);

    let palette = TerrainSurfacePalette {
        default_color: [0.2, 0.3, 0.4, 1.0],
        layer_colors: vec![[1.0, 0.0, 0.0, 1.0]],
    };
    let entity = app
        .world_mut()
        .spawn((
            TerrainChunkBundle {
                chunk: TerrainChunk::new(Vec3::ZERO, 2.0, 2),
                heightmap: Heightmap::flat(2, 0.0),
                ..Default::default()
            },
            TerrainSurfaceMap::new(
                1,
                vec![TerrainSurfaceWeight {
                    ids: [
                        0,
                        TERRAIN_SURFACE_UNDEFINED_ID,
                        TERRAIN_SURFACE_UNDEFINED_ID,
                    ],
                    weights: [u8::MAX, 0, 0],
                }],
            )
            .unwrap(),
            palette,
        ))
        .id();

    app.update();

    let mesh = app
        .world()
        .entity(entity)
        .get::<Mesh3d>()
        .unwrap()
        .0
        .clone();
    assert_eq!(
        mesh_vertex_color(app.world(), &mesh, 0),
        Some([1.0, 0.0, 0.0, 1.0])
    );
}

#[test]
fn terrain_mesh_uses_chunk_lod_for_sample_spacing() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .add_systems(Update, generate_terrain_meshes);

    let mut chunk = TerrainChunk::new(Vec3::ZERO, 4.0, 5);
    chunk.current_lod = 1;

    let entity = app
        .world_mut()
        .spawn(TerrainChunkBundle {
            chunk,
            heightmap: Heightmap::flat(5, 0.0),
            ..Default::default()
        })
        .id();

    app.update();

    let mesh = app
        .world()
        .entity(entity)
        .get::<Mesh3d>()
        .unwrap()
        .0
        .clone();
    let meshes = app.world().resource::<Assets<Mesh>>();
    let mesh = meshes.get(&mesh).unwrap();

    assert_eq!(mesh.count_vertices(), 9);
    assert!(matches!(mesh.indices(), Some(Indices::U32(indices)) if indices.len() == 24));
}

#[test]
fn terrain_mesh_regenerates_when_heightmap_changes() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .add_systems(Update, generate_terrain_meshes);

    let entity = app
        .world_mut()
        .spawn(TerrainChunkBundle {
            chunk: TerrainChunk::new(Vec3::ZERO, 2.0, 2),
            heightmap: Heightmap::flat(2, 0.0),
            ..Default::default()
        })
        .id();

    app.update();

    let first_mesh = app
        .world()
        .entity(entity)
        .get::<Mesh3d>()
        .unwrap()
        .0
        .clone();
    assert_eq!(mesh_position_y(app.world(), &first_mesh, 3), Some(0.0));

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Heightmap>()
        .unwrap()
        .set_height(1, 1, 5.0);
    app.update();

    let second_mesh = app
        .world()
        .entity(entity)
        .get::<Mesh3d>()
        .unwrap()
        .0
        .clone();
    assert_ne!(second_mesh, first_mesh);
    assert_eq!(mesh_position_y(app.world(), &second_mesh, 3), Some(5.0));
}

fn mesh_index_count(world: &World, handle: &Handle<Mesh>) -> Option<usize> {
    let meshes = world.resource::<Assets<Mesh>>();
    let mesh = meshes.get(handle)?;
    mesh.indices().map(Indices::len)
}

fn mesh_position_y(world: &World, handle: &Handle<Mesh>, vertex: usize) -> Option<f32> {
    let meshes = world.resource::<Assets<Mesh>>();
    let mesh = meshes.get(handle)?;
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?;
    let bevy::mesh::VertexAttributeValues::Float32x3(values) = positions else {
        return None;
    };
    values.get(vertex).map(|position| position[1])
}

fn mesh_vertex_color(world: &World, handle: &Handle<Mesh>, vertex: usize) -> Option<[f32; 4]> {
    let meshes = world.resource::<Assets<Mesh>>();
    let mesh = meshes.get(handle)?;
    let colors = mesh.attribute(Mesh::ATTRIBUTE_COLOR)?;
    let bevy::mesh::VertexAttributeValues::Float32x4(values) = colors else {
        return None;
    };
    values.get(vertex).copied()
}
