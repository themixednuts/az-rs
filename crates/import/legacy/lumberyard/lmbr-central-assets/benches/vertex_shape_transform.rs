use criterion::{Criterion, criterion_group, criterion_main};
use lmbr_central_assets::transform_vertex_shape_asset;
use std::hint::black_box;

fn vertex_shape_bytes(vertices: u16, metadata: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&u32::from(vertices).to_le_bytes());
    for index in 0..vertices {
        let x = f32::from(index);
        let y = f32::from(index % 17);
        let z = 0.0f32;
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&z.to_le_bytes());
    }
    bytes.extend_from_slice(&u32::from(metadata).to_le_bytes());
    for index in 0..metadata {
        bytes.extend_from_slice(&u32::from(index).to_le_bytes());
        bytes.extend_from_slice(&(u32::from(index) * 3).to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&32.0f32.to_le_bytes());
    bytes.extend_from_slice(&7u32.to_le_bytes());
    bytes
}

fn bench_vertex_shape_transform(c: &mut Criterion) {
    let bytes = vertex_shape_bytes(512, 64);
    c.bench_function("vertex_shape_transform_512_vertices", |b| {
        b.iter(|| {
            let asset = transform_vertex_shape_asset(black_box(&bytes)).unwrap();
            black_box(asset.local_bounds());
            asset
        });
    });
}

criterion_group!(benches, bench_vertex_shape_transform);
criterion_main!(benches);
