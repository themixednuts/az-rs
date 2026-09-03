//! Benches for az-objectstream: DOM-style `from_bytes` vs the
//! streaming `visit::parse_streaming_bytes` API across different
//! element counts.
//!
//! Run with `cargo bench -p az-objectstream`.

use az_objectstream::visit::{ElementHeader, ElementVisitor, VisitFlow, parse_streaming_bytes};
use az_objectstream::{
    Element, ObjectStream, ObjectStreamError, ST_BINARYFLAG_ELEMENT_HEADER, StreamTag,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use uuid::Uuid;

/// Build a synthetic `ObjectStream` with `n` flat top-level elements.
///
/// `flags` MUST be non-zero — `flags == 0` is the
/// `ST_BINARYFLAG_ELEMENT_END` sentinel and would terminate the
/// list at the first element.
fn build_synthetic(n: usize) -> Vec<u8> {
    let mut stream = ObjectStream::new(3);
    for i in 0..n {
        let id = Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 | i as u128);
        stream
            .elements
            .push(Element::new(id).with_wire_flags(ST_BINARYFLAG_ELEMENT_HEADER));
    }
    let mut buf = Vec::new();
    stream.write_to(&mut buf).expect("write");
    buf
}

fn bench_parse_dom_vs_streaming(c: &mut Criterion) {
    for &n in &[100usize, 1_000, 10_000] {
        let bytes = build_synthetic(n);

        let mut group = c.benchmark_group("parse");
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::new("dom", n), &bytes, |b, bytes| {
            // Use iter_with_large_drop so the allocated ObjectStream
            // is dropped outside the measured region — and so LLVM
            // can't elide the parse via CSE on the (otherwise
            // unused) result.
            b.iter_with_large_drop(|| {
                ObjectStream::from_bytes(black_box(bytes.as_slice()), None).expect("parse")
            });
        });

        group.bench_with_input(
            BenchmarkId::new("streaming_count", n),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let mut counter = Counter(0);
                    parse_streaming_bytes(black_box(bytes.as_slice()), None, &mut counter)
                        .expect("parse");
                    counter.0
                });
            },
        );

        group.finish();
    }
}

fn bench_stream_tag(c: &mut Criterion) {
    let mut group = c.benchmark_group("stream_tag");
    group.bench_function("from_byte_binary", |b| {
        b.iter(|| black_box(StreamTag::from_byte(black_box(0))));
    });
    group.bench_function("from_byte_xml", |b| {
        b.iter(|| black_box(StreamTag::from_byte(black_box(b'<'))));
    });
    group.bench_function("from_byte_unknown", |b| {
        b.iter(|| black_box(StreamTag::from_byte(black_box(b'?'))));
    });
    group.finish();
}

struct Counter(usize);

impl ElementVisitor for Counter {
    type Error = ObjectStreamError;
    #[inline]
    fn open_element(&mut self, _h: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
        self.0 += 1;
        Ok(VisitFlow::Continue)
    }
}

criterion_group!(benches, bench_parse_dom_vs_streaming, bench_stream_tag);
criterion_main!(benches);
