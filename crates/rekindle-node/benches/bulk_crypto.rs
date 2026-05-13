//! Benchmarks for the BulkCipher (aws-lc-rs AES-256-GCM).
//!
//! Measures seal and open throughput at chunk sizes from 1 KiB to 1 MiB.
//! These numbers establish the crypto throughput ceiling for the bulk
//! transfer plane.
//!
//! Acceptance thresholds:
//! - `bulk_seal/65519` >= 3.0 GiB/s on Coffee Lake (AES-NI+AVX)
//! - `bulk_seal/65519` >= 5.0 GiB/s on AVX-512 hardware

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};

use rekindle_node::ipc::bulk::cipher::{BulkCipher, TAG_LEN};

fn bench_seal(c: &mut Criterion) {
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("bulk_seal");

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

    for &size in &[1024usize, 4096, 16_384, 65_519, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xABu8; n];
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                // Encrypt fresh each iteration (nonce must be unique).
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

criterion_group!(benches, bench_seal, bench_open);
criterion_main!(benches);
