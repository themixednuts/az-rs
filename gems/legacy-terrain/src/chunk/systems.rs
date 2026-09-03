use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh3d, PrimitiveTopology};
use bevy::prelude::*;

use crate::heightmap::Heightmap;
use crate::heightmap::math::{ExactF32, mesh_index, remap_index};
use crate::lod::lod_level;
use crate::region::{TerrainSurfaceMap, TerrainSurfacePalette};

use super::component::TerrainChunk;

/// Update terrain LOD based on camera distance.
pub fn update_terrain_lod(
    camera_query: Query<&Transform, With<Camera>>,
    mut terrain_query: Query<(&mut TerrainChunk, &Transform), Without<Camera>>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    for (mut chunk, chunk_transform) in terrain_query.iter_mut() {
        let distance = camera_transform
            .translation
            .distance(chunk_transform.translation);

        let mut new_lod = chunk.max_lod_levels - 1;
        for (level, &threshold) in chunk.lod_distances.iter().enumerate() {
            if distance < threshold {
                new_lod = lod_level(level);
                break;
            }
        }

        chunk.current_lod = new_lod;
    }
}

/// Generate terrain meshes from heightmap data.
#[allow(clippy::type_complexity)]
pub fn generate_terrain_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    terrain_query: Query<
        (
            Entity,
            &TerrainChunk,
            &Heightmap,
            Option<&TerrainSurfaceMap>,
            Option<&TerrainSurfacePalette>,
        ),
        Or<(
            Changed<TerrainChunk>,
            Changed<Heightmap>,
            Changed<TerrainSurfaceMap>,
            Changed<TerrainSurfacePalette>,
            Without<Mesh3d>,
        )>,
    >,
) {
    for (entity, chunk, heightmap, surface_map, surface_palette) in terrain_query.iter() {
        let mesh = generate_terrain_mesh(chunk, heightmap, surface_map, surface_palette);
        let mesh_handle = meshes.add(mesh);

        commands.entity(entity).insert(Mesh3d(mesh_handle));
    }
}

fn generate_terrain_mesh(
    chunk: &TerrainChunk,
    heightmap: &Heightmap,
    surface_map: Option<&TerrainSurfaceMap>,
    surface_palette: Option<&TerrainSurfacePalette>,
) -> Mesh {
    let resolution = chunk.resolution;
    let size = chunk.size;
    let sample_x = terrain_sample_indices(resolution, chunk.current_lod);
    let sample_z = terrain_sample_indices(resolution, chunk.current_lod);
    let cells = (resolution - 1).exact_f32();
    let vertex_spacing = size / cells;

    let sample_vertex_count = sample_x.len() * sample_z.len();
    let mut positions = Vec::with_capacity(sample_vertex_count);
    let mut normals = Vec::with_capacity(sample_vertex_count);
    let mut uvs = Vec::with_capacity(sample_vertex_count);
    let mut colors = surface_palette.map(|_| Vec::with_capacity(sample_vertex_count));
    let mut indices = Vec::with_capacity((sample_x.len() - 1) * (sample_z.len() - 1) * 6);

    for &z in &sample_z {
        for &x in &sample_x {
            let height = heightmap.get_height(x, z).unwrap_or(0.0);

            let world_x = x.exact_f32().mul_add(vertex_spacing, -(size / 2.0));
            let world_z = z.exact_f32().mul_add(vertex_spacing, -(size / 2.0));

            positions.push([world_x, height, world_z]);

            let normal = heightmap
                .calculate_normal(x, z, vertex_spacing)
                .unwrap_or(Vec3::Y);
            normals.push(normal.to_array());

            uvs.push([x.exact_f32() / cells, z.exact_f32() / cells]);

            if let (Some(colors), Some(surface_map), Some(surface_palette)) =
                (&mut colors, surface_map, surface_palette)
            {
                colors.push(surface_vertex_color(
                    surface_map,
                    surface_palette,
                    x,
                    z,
                    resolution,
                ));
            }
        }
    }

    let sample_width = sample_x.len();
    for z in 0..(sample_z.len() - 1) {
        for x in 0..(sample_x.len() - 1) {
            let min_x = sample_x[x];
            let min_z = sample_z[z];
            let max_x = sample_x[x + 1];
            let max_z = sample_z[z + 1];
            if surface_map.is_some_and(|surface_map| {
                surface_map.contains_hole_quad(min_x, min_z, max_x, max_z, resolution)
            }) {
                continue;
            }

            let i0 = mesh_index(z * sample_width + x);
            let i1 = i0 + 1;
            let i2 = i0 + mesh_index(sample_width);
            let i3 = i2 + 1;

            indices.push(i0);
            indices.push(i2);
            indices.push(i1);

            indices.push(i1);
            indices.push(i2);
            indices.push(i3);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices));
    if let Some(colors) = colors {
        mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh
}

fn surface_vertex_color(
    surface_map: &TerrainSurfaceMap,
    surface_palette: &TerrainSurfacePalette,
    x: usize,
    z: usize,
    mesh_resolution: usize,
) -> [f32; 4] {
    let max_mesh = mesh_resolution.saturating_sub(1);
    let max_surface = surface_map.resolution.saturating_sub(1);
    let surface_x = remap_index(x, max_mesh, max_surface);
    let surface_z = remap_index(z, max_mesh, max_surface);
    surface_map
        .surface_weight_at(surface_x, surface_z)
        .map_or(surface_palette.default_color, |weight| {
            surface_palette.color_for_weight(weight)
        })
}

fn terrain_sample_indices(resolution: usize, lod: u8) -> Vec<usize> {
    let max_index = resolution.saturating_sub(1);
    let step = terrain_lod_step(resolution, lod);
    let mut indices = (0..=max_index).step_by(step).collect::<Vec<_>>();
    if indices.last().is_none_or(|last| *last != max_index) {
        indices.push(max_index);
    }
    indices
}

fn terrain_lod_step(resolution: usize, lod: u8) -> usize {
    let max_step = resolution.saturating_sub(1).max(1);
    1usize
        .checked_shl(u32::from(lod))
        .unwrap_or(usize::MAX)
        .clamp(1, max_step)
}
