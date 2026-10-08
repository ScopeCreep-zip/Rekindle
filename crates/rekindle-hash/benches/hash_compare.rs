//! Comparative hash benchmark: single-buffer SHA-256, every multi-buffer
//! kernel this host supports, the `sha256_parallel` policy, and BLAKE3.
//!
//! Each batch group hashes N messages of 64 KiB - 17 bytes (a ragged final
//! block) for N from 1 to 32, so a kernel is measured with its lanes partly
//! empty, exactly full, and refilled: `sha256_batch_single` is aws-lc per
//! message, `sha256_batch_<kernel>` one multi-buffer kernel, and
//! `sha256_parallel` the policy (`multi_buffer::for_batch`). Where a
//! kernel's line crosses single-buffer's is the batch size at which
//! multi-buffer starts to pay on this host.
//!
//! Acceptance thresholds:
//! - sha256_single/64KiB >= 400 MiB/s
//! - sha256_batch_avx2/8 >= 1.0 GiB/s aggregate (AVX2 hosts)
//! - blake3_single/64KiB >= 3.5 GiB/s

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

const CHUNK_SIZE: usize = 65519;
const BATCHES: [usize; 10] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32];
const MEASUREMENT: std::time::Duration = std::time::Duration::from_secs(15);

fn messages(count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|seed| {
            (0..CHUNK_SIZE)
                .map(|i| u8::try_from((i * 31 + seed * 7) % 251).unwrap_or(0))
                .collect()
        })
        .collect()
}

fn batch_bytes(count: usize) -> u64 {
    u64::try_from(CHUNK_SIZE * count).unwrap_or(u64::MAX)
}

fn bench_sha256_single(c: &mut Criterion) {
    let data = vec![0xABu8; CHUNK_SIZE];
    let mut group = c.benchmark_group("sha256_single");
    group.throughput(Throughput::Bytes(batch_bytes(1)));
    group.measurement_time(MEASUREMENT);
    group.bench_function("64KiB", |b| {
        b.iter(|| black_box(rekindle_hash::single::sha256_oneshot(black_box(&data))));
    });
    group.finish();
}

fn bench_batch_single(c: &mut Criterion) {
    let all = messages(BATCHES[BATCHES.len() - 1]);
    let mut group = c.benchmark_group("sha256_batch_single");
    group.measurement_time(MEASUREMENT);
    for count in BATCHES {
        let refs: Vec<&[u8]> = all[..count].iter().map(Vec::as_slice).collect();
        let mut digests = vec![[0u8; 32]; count];
        group.throughput(Throughput::Bytes(batch_bytes(count)));
        group.bench_with_input(BenchmarkId::from_parameter(count), &refs, |b, refs| {
            b.iter(|| {
                for (message, digest) in refs.iter().zip(digests.iter_mut()) {
                    *digest = rekindle_hash::single::sha256_oneshot(black_box(message));
                }
                black_box(&digests);
            });
        });
    }
    group.finish();
}

#[cfg(feature = "sha256-mb")]
fn bench_batch_kernels(c: &mut Criterion) {
    use rekindle_hash::multi_buffer::{sha256_mb_with, supported, Workspace};

    let all = messages(BATCHES[BATCHES.len() - 1]);
    for kernel in supported() {
        let mut group = c.benchmark_group(format!("sha256_batch_{kernel}"));
        group.measurement_time(MEASUREMENT);
        let mut workspace = Workspace::new();
        for count in BATCHES {
            let refs: Vec<&[u8]> = all[..count].iter().map(Vec::as_slice).collect();
            let mut digests = vec![[0u8; 32]; count];
            group.throughput(Throughput::Bytes(batch_bytes(count)));
            group.bench_with_input(BenchmarkId::from_parameter(count), &refs, |b, refs| {
                b.iter(|| {
                    if let Err(error) =
                        sha256_mb_with(&mut workspace, kernel, black_box(refs), &mut digests)
                    {
                        panic!("{kernel} with {count} messages: {error}");
                    }
                    black_box(&digests);
                });
            });
        }
        group.finish();
    }
}

fn bench_parallel_policy(c: &mut Criterion) {
    let all = messages(BATCHES[BATCHES.len() - 1]);
    let mut group = c.benchmark_group("sha256_parallel");
    group.measurement_time(MEASUREMENT);
    for count in BATCHES {
        let refs: Vec<&[u8]> = all[..count].iter().map(Vec::as_slice).collect();
        let mut digests = vec![[0u8; 32]; count];
        group.throughput(Throughput::Bytes(batch_bytes(count)));
        group.bench_with_input(BenchmarkId::from_parameter(count), &refs, |b, refs| {
            b.iter(|| {
                rekindle_hash::sha256_parallel(black_box(refs), &mut digests);
                black_box(&digests);
            });
        });
    }
    group.finish();
}

fn bench_blake3(c: &mut Criterion) {
    let data = vec![0xABu8; CHUNK_SIZE];
    let mut group = c.benchmark_group("blake3_single");
    group.throughput(Throughput::Bytes(batch_bytes(1)));
    group.measurement_time(MEASUREMENT);
    group.bench_function("64KiB", |b| {
        b.iter(|| black_box(rekindle_hash::single::blake3_oneshot(black_box(&data))));
    });
    group.finish();
}

#[cfg(feature = "sha256-mb")]
criterion_group!(
    benches,
    bench_sha256_single,
    bench_batch_single,
    bench_batch_kernels,
    bench_parallel_policy,
    bench_blake3
);
#[cfg(not(feature = "sha256-mb"))]
criterion_group!(
    benches,
    bench_sha256_single,
    bench_batch_single,
    bench_parallel_policy,
    bench_blake3
);
criterion_main!(benches);
