use std::hint::black_box;

use az_framework::math::transform_columns_to_bevy;
use criterion::{Criterion, criterion_group, criterion_main};

const TRANSFORM_COLUMNS: [f32; 12] = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 1.0, 2.0, 3.0];

fn bench_transform_columns(c: &mut Criterion) {
    c.bench_function("transform_columns_to_bevy", |b| {
        b.iter(|| transform_columns_to_bevy(black_box(TRANSFORM_COLUMNS)));
    });
}

criterion_group!(benches, bench_transform_columns);
criterion_main!(benches);
