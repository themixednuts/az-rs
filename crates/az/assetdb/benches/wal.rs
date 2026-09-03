mod support;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn bench_wal(c: &mut Criterion) {
    let mut group = c.benchmark_group("assetdb/wal");
    group.sample_size(10);
    for &bytes in support::payload_bytes() {
        let probe = support::PreparedCompletion::new(bytes).commit_and_sample_journal();
        println!("assetdb/wal/{bytes} {probe}");
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(BenchmarkId::from_parameter(bytes), &bytes, |b, &bytes| {
            b.iter_batched(
                || support::PreparedCompletion::new(bytes),
                |fixture| std::hint::black_box(fixture.commit_and_sample_journal()),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_wal);
criterion_main!(benches);
