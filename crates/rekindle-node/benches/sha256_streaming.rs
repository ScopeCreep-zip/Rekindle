//! SHA-256 streaming vs one-shot benchmark.
//!
//! The bulk transfer spec requires streaming SHA-256 verification for OCI
//! containerd blob digests: `aws_lc_rs::digest::Context::update` per chunk
//! as it arrives, finalize on the last chunk. This avoids serializing a
//! post-download verification pass — at 10 Gbps a 2 GB blob arrives in
//! 1.6s and SHA-256 finishes in ~1s; interleaved ≈ 1.7s vs serialized 2.6s.
//!
//! This benchmark validates that streaming (per-chunk update) is within 5%
//! of one-shot on the same total bytes. If streaming is >5% slower, the
//! per-call overhead of `Context::update` is dominating and chunk size
//! should be increased.
//!
//! Also benchmarks blake3 streaming vs one-shot as a comparison — blake3
//! is the project's internal integrity hash (gossip dedup, keypair checksums,
//! content addressing). Both should be measured to inform which hash to use
//! where.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};

use aws_lc_rs::digest;

// ── SHA-256 Streaming vs One-Shot ───────────────────────────────────
//
// Measures the per-chunk update cost of aws_lc_rs::digest::Context.
// Chunk size fixed at 64 KiB (the bulk transfer chunk size).
// Total sizes: 256 KiB, 1 MiB, 16 MiB, 64 MiB.
//
// The streaming path calls Context::update(64 KiB) N times then finish().
// The one-shot path calls digest::digest(entire_payload).
//
// Expected: streaming within 5% of one-shot. If not, the per-call
// overhead of Context::update is the bottleneck — increase chunk size.

fn bench_sha256(c: &mut Criterion) {
    let mut group = c.benchmark_group("sha256");
    group.measurement_time(std::time::Duration::from_secs(30));
    let chunk_size = 65_536usize;
    let chunk = vec![0xCCu8; chunk_size];

    for &total_size in &[262_144usize, 1_048_576, 16_777_216, 67_108_864] {
        let num_chunks = total_size / chunk_size;
        group.throughput(Throughput::Bytes(total_size as u64));

        // Streaming: per-chunk update
        group.bench_with_input(
            BenchmarkId::new("streaming_64k", total_size),
            &total_size,
            |b, _| {
                b.iter(|| {
                    let mut ctx = digest::Context::new(&digest::SHA256);
                    for _ in 0..num_chunks {
                        ctx.update(black_box(&chunk));
                    }
                    let digest = ctx.finish();
                    black_box(digest);
                });
            },
        );

        // One-shot: single call
        let payload = vec![0xCCu8; total_size];
        group.bench_with_input(
            BenchmarkId::new("oneshot", total_size),
            &total_size,
            |b, _| {
                b.iter(|| {
                    let digest = digest::digest(&digest::SHA256, black_box(&payload));
                    black_box(digest);
                });
            },
        );
    }
    group.finish();
}

// ── BLAKE3 Streaming vs One-Shot (comparison) ───────────────────────
//
// Same structure as SHA-256 but using blake3. Provides a direct
// comparison for internal hash path selection. blake3 should be
// significantly faster than SHA-256 due to tree hashing and wider
// SIMD utilization — this benchmark quantifies the delta.

fn bench_blake3(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3");
    group.measurement_time(std::time::Duration::from_secs(30));
    let chunk_size = 65_536usize;
    let chunk = vec![0xDDu8; chunk_size];

    for &total_size in &[262_144usize, 1_048_576, 16_777_216, 67_108_864] {
        let num_chunks = total_size / chunk_size;
        group.throughput(Throughput::Bytes(total_size as u64));

        // Streaming
        group.bench_with_input(
            BenchmarkId::new("streaming_64k", total_size),
            &total_size,
            |b, _| {
                b.iter(|| {
                    let mut hasher = blake3::Hasher::new();
                    for _ in 0..num_chunks {
                        hasher.update(black_box(&chunk));
                    }
                    let hash = hasher.finalize();
                    black_box(hash);
                });
            },
        );

        // One-shot
        let payload = vec![0xDDu8; total_size];
        group.bench_with_input(
            BenchmarkId::new("oneshot", total_size),
            &total_size,
            |b, _| {
                b.iter(|| {
                    let hash = blake3::hash(black_box(&payload));
                    black_box(hash);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_sha256, bench_blake3);
criterion_main!(benches);
