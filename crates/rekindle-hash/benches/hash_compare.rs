//! Comparative hash benchmark: single-buffer SHA-256 vs multi-buffer vs BLAKE3.
//!
//! Acceptance thresholds:
//! - sha256_single/64KiB >= 400 MiB/s
//! - sha256_multi_buffer/8x64KiB >= 1.0 GiB/s aggregate
//! - blake3/64KiB >= 3.5 GiB/s

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

const CHUNK_SIZE: usize = 65519;

fn print_capabilities() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let (mb_mibs, speedup) = rekindle_hash::probe_sha256_mb_performance();
        let single_mibs = {
            let chunk = vec![0xABu8; 65536];
            let start = std::time::Instant::now();
            for _ in 0..500 {
                let _ = rekindle_hash::single::sha256_oneshot(&chunk);
            }
            let elapsed = start.elapsed();
            (65536.0 * 500.0) / (1024.0 * 1024.0) / elapsed.as_secs_f64()
        };
        let simd = speedup >= 3.0;
        eprintln!("\n[bench] SHA256 single: {single_mibs:.0} MiB/s | SHA256 multi-buf: {mb_mibs:.0} MiB/s | speedup: {speedup:.1}x | SIMD active: {simd}");
    });
}

fn bench_sha256_single(c: &mut Criterion) {
    print_capabilities();
    let data = vec![0xABu8; CHUNK_SIZE];
    let mut group = c.benchmark_group("sha256_single");
    group.throughput(Throughput::Bytes(CHUNK_SIZE as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("64KiB", |b| {
        b.iter(|| {
            let d = rekindle_hash::single::sha256_oneshot(black_box(&data));
            black_box(d);
        });
    });
    group.finish();
}

#[cfg(feature = "sha256-mb")]
fn bench_sha256_mb(c: &mut Criterion) {
    let chunks: Vec<Vec<u8>> = (0..8).map(|_| vec![0xABu8; CHUNK_SIZE]).collect();
    let refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();
    let mut digests = vec![[0u8; 32]; 8];

    let mut group = c.benchmark_group("sha256_multi_buffer");
    group.throughput(Throughput::Bytes((CHUNK_SIZE * 8) as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("8x64KiB_avx2", |b| {
        b.iter(|| {
            rekindle_hash::sha256_parallel(black_box(&refs), &mut digests);
            black_box(&digests);
        });
    });
    group.finish();
}

fn bench_blake3(c: &mut Criterion) {
    let data = vec![0xABu8; CHUNK_SIZE];
    let mut group = c.benchmark_group("blake3_single");
    group.throughput(Throughput::Bytes(CHUNK_SIZE as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("64KiB", |b| {
        b.iter(|| {
            let d = rekindle_hash::single::blake3_oneshot(black_box(&data));
            black_box(d);
        });
    });
    group.finish();
}

fn bench_parallel_dispatch(c: &mut Criterion) {
    // Test the sha256_parallel dispatcher with varying chunk counts
    // to verify multi-buffer kicks in at >= 4 chunks.
    let chunks: Vec<Vec<u8>> = (0..16).map(|_| vec![0xCDu8; CHUNK_SIZE]).collect();

    let mut group = c.benchmark_group("sha256_parallel_dispatch");
    group.measurement_time(std::time::Duration::from_secs(15));

    for &n in &[1, 4, 8, 16] {
        let refs: Vec<&[u8]> = chunks[..n].iter().map(|c| c.as_slice()).collect();
        let mut digests = vec![[0u8; 32]; n];

        group.throughput(Throughput::Bytes((CHUNK_SIZE * n) as u64));
        group.bench_function(format!("{n}_chunks"), |b| {
            b.iter(|| {
                rekindle_hash::sha256_parallel(black_box(&refs), &mut digests);
                black_box(&digests);
            });
        });
    }
    group.finish();
}

#[cfg(feature = "sha256-mb")]
criterion_group!(benches, bench_sha256_single, bench_sha256_mb, bench_blake3, bench_parallel_dispatch);
#[cfg(not(feature = "sha256-mb"))]
criterion_group!(benches, bench_sha256_single, bench_blake3, bench_parallel_dispatch);
criterion_main!(benches);
