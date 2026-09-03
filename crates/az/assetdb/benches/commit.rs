mod support;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn bench_commit(c: &mut Criterion) {
    let mut group = c.benchmark_group("assetdb/commit/batch_1");
    group.sample_size(10);
    for &bytes in support::payload_bytes() {
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(BenchmarkId::from_parameter(bytes), &bytes, |b, &bytes| {
            b.iter_batched(
                || support::PreparedCompletion::new(bytes),
                |fixture| {
                    let _: () = fixture.commit();
                    std::hint::black_box(());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_commit);
criterion_main!(benches);
