//! Benches for az-pak: AZCS header peek + zip decompression
//! (Stored / Deflated round-trips) over synthetic in-memory paks.
//!
//! Oodle (compression method 15) is excluded because a useful benchmark
//! requires licensed encoder output rather than synthetic bytes.
//!
//! Run with `cargo bench -p az-pak`.

use std::hint::black_box;
use std::io::{Cursor, Write};

use az_pak::{AZCS_SIGNATURE, AzcsHeader, AzcsId, decompress_zip_entry_into, is_azcs};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use zip::CompressionMethod;

const KB: usize = 1024;

fn azcs_yes_buf() -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(AZCS_SIGNATURE);
    buf.extend_from_slice(&AzcsId::Zlib.as_u32().to_be_bytes());
    buf.extend_from_slice(&1024u64.to_be_bytes());
    buf
}

fn azcs_no_buf() -> Vec<u8> {
    vec![0u8; 16]
}

fn bench_azcs_predicates(c: &mut Criterion) {
    let yes = azcs_yes_buf();
    let no = azcs_no_buf();

    let mut group = c.benchmark_group("azcs");
    group.bench_function("is_azcs_yes", |b| {
        b.iter(|| black_box(is_azcs(black_box(&yes))));
    });
    group.bench_function("is_azcs_no", |b| {
        b.iter(|| black_box(is_azcs(black_box(&no))));
    });
    group.bench_function("peek_header", |b| {
        b.iter(|| black_box(AzcsHeader::peek(black_box(&yes))));
    });
    group.bench_function("id_from_u32_match", |b| {
        b.iter(|| black_box(AzcsId::from_u32(black_box(0x7388_7D3A))));
    });
    group.bench_function("id_from_u32_miss", |b| {
        b.iter(|| black_box(AzcsId::from_u32(black_box(0xDEAD_BEEF))));
    });
    group.finish();
}

/// Build a one-entry in-memory zip with the requested compression.
fn build_zip(method: CompressionMethod, payload: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::FileOptions<'_, '_, ()> =
            zip::write::FileOptions::default().compression_method(method);
        zip.start_file("data.bin", opts).expect("start_file");
        zip.write_all(payload).expect("write");
        zip.finish().expect("finish");
    }
    buf.into_inner()
}

fn bench_decompress_stored(c: &mut Criterion) {
    let payload = vec![0xABu8; 64 * KB];
    let zip_bytes = build_zip(CompressionMethod::Stored, &payload);

    let mut group = c.benchmark_group("decompress_zip");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("stored_64kb", |b| {
        b.iter(|| {
            let cursor = Cursor::new(black_box(&zip_bytes));
            let mut archive = zip::ZipArchive::new(cursor).expect("zip");
            let mut entry = archive.by_index(0).expect("by_index");
            let mut out = Vec::with_capacity(payload.len());
            decompress_zip_entry_into(&mut entry, &mut out).expect("decompress");
            black_box(out);
        });
    });
    group.finish();
}

fn bench_decompress_deflate(c: &mut Criterion) {
    // Generate a moderately compressible payload (alternating bytes).
    let mut payload = Vec::with_capacity(64 * KB);
    for i in 0..64 * KB {
        payload.push(u8::try_from(i % 251).expect("i % 251 is always below 256"));
    }
    let zip_bytes = build_zip(CompressionMethod::Deflated, &payload);

    let mut group = c.benchmark_group("decompress_zip");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("deflated_64kb", |b| {
        b.iter(|| {
            let cursor = Cursor::new(black_box(&zip_bytes));
            let mut archive = zip::ZipArchive::new(cursor).expect("zip");
            let mut entry = archive.by_index(0).expect("by_index");
            let mut out = Vec::with_capacity(payload.len());
            decompress_zip_entry_into(&mut entry, &mut out).expect("decompress");
            black_box(out);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_azcs_predicates,
    bench_decompress_stored,
    bench_decompress_deflate,
);
criterion_main!(benches);
