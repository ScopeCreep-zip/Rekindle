//! Benchmarks for the BulkCipher (aws-lc-rs AES-256-GCM).
//!
//! Measures seal, open, and open_separate throughput at chunk sizes
//! from 1 KiB to 1 MiB. These numbers establish the crypto throughput
//! ceiling for the bulk transfer plane.
//!
//! Acceptance thresholds:
//! - `bulk_seal/65519` >= 3.0 GiB/s on Coffee Lake (AES-NI+AVX)
//! - `bulk_open_separate/65519` >= 2.5 GiB/s (detached-tag decrypt)
//! - `key_construction/reused_key` >= 3.5 GiB/s (confirms no per-call setup)

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};

use rekindle_node::ipc::bulk::cipher::{BulkCipher, TAG_LEN};

fn print_capabilities() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let caps = rekindle_node::ipc::bulk::capability::probe();
        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  CRYPTO CAPABILITY PROBE                                     ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  AES-256-GCM seal:    {:>8.2} GiB/s                        ║", caps.aes_gcm_seal_gibs);
        eprintln!("║  AES-256-GCM open:    {:>8.2} GiB/s                        ║", caps.aes_gcm_open_gibs);
        eprintln!("║  AEGIS-128L seal:     {:>8.2} GiB/s                        ║", caps.aegis_seal_gibs);
        eprintln!("║  SHA-256 single:      {:>8.0} MiB/s                        ║", caps.sha256_single_mibs);
        eprintln!("║  SHA-256 multi-buf:   {:>8.0} MiB/s  (SIMD: {})          ║", caps.sha256_mb_mibs, if caps.sha256_mb_simd_active { "YES" } else { " NO" });
        eprintln!("║  BLAKE3:              {:>8.2} GiB/s                        ║", caps.blake3_gibs);
        eprintln!("║  Bulk AEAD:           {:>30}  ║", caps.bulk_aead_algorithm);
        eprintln!("║  Meets targets:       {:>30}  ║", if caps.meets_targets() { "YES" } else { "NO — see warnings" });
        eprintln!("╚══════════════════════════════════════════════════════════════╝\n");
    });
}

fn bench_seal(c: &mut Criterion) {
    print_capabilities();
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("bulk_seal");
    group.measurement_time(std::time::Duration::from_secs(15));

    for &size in &[1024usize, 4096, 16_384, 65_519, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xABu8; n];
            let mut buf = vec![0u8; n];
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                buf[..n].copy_from_slice(&plain);
                let _tag = cipher.seal_in_place(nonce, b"aad", &mut buf).unwrap();
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_open(c: &mut Criterion) {
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("bulk_open");
    group.measurement_time(std::time::Duration::from_secs(15));

    for &size in &[1024usize, 4096, 16_384, 65_519, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xABu8; n];
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                let mut buf = plain.clone();
                let tag = cipher.seal_in_place(nonce, b"aad", &mut buf).unwrap();
                let mut combined = Vec::with_capacity(n + TAG_LEN);
                combined.extend_from_slice(&buf);
                combined.extend_from_slice(&tag);
                let _pt_len = cipher
                    .open_in_place(nonce, b"aad", &mut combined)
                    .unwrap();
                black_box(&combined);
            });
        });
    }
    group.finish();
}

fn bench_open_separate(c: &mut Criterion) {
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("bulk_open_separate");
    group.measurement_time(std::time::Duration::from_secs(15));

    for &size in &[1024usize, 4096, 16_384, 65_519, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xABu8; n];
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                // Encrypt to get ciphertext + tag.
                let mut ct = plain.clone();
                let tag = cipher.seal_in_place(nonce, b"aad", &mut ct).unwrap();
                // Decrypt with detached tag into separate output.
                let mut pt = vec![0u8; n];
                cipher.open_separate(nonce, b"aad", &ct, &tag, &mut pt).unwrap();
                black_box(&pt);
            });
        });
    }
    group.finish();
}

fn bench_key_construction(c: &mut Criterion) {
    let key_bytes = [0x42u8; 32];
    let plain = vec![0xABu8; 65_519];

    let mut group = c.benchmark_group("key_construction");
    group.throughput(Throughput::Bytes(65_519));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("reused_key", |b| {
        let cipher = BulkCipher::new(&key_bytes);
        let mut nonce = 0u64;
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            let mut buf = plain.clone();
            let _tag = cipher.seal_in_place(nonce, b"", &mut buf).unwrap();
            black_box(&buf);
        });
    });

    group.bench_function("new_key_per_chunk", |b| {
        let mut nonce = 0u64;
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            let cipher = BulkCipher::new(&key_bytes);
            let mut buf = plain.clone();
            let _tag = cipher.seal_in_place(nonce, b"", &mut buf).unwrap();
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_pool_replenish_zeroize(c: &mut Criterion) {
    use rekindle_node::ipc::bulk::pool::BufferPool;

    let pool = BufferPool::new();
    let mut group = c.benchmark_group("pool_replenish_zeroize");
    group.throughput(Throughput::Bytes(65_549)); // SLAB_SIZE
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("replenish_64KiB_slab", |b| {
        b.iter(|| {
            let mut slab = pool.acquire();
            // Simulate a full encryption cycle: fill with "ciphertext"
            slab.extend_from_slice(&[0xAB; 65_549]);
            pool.replenish(slab);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_seal, bench_open, bench_open_separate, bench_key_construction, bench_pool_replenish_zeroize);
criterion_main!(benches);
