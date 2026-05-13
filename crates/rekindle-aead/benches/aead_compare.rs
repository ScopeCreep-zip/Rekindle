//! Comparative AEAD benchmark: AES-256-GCM vs AEGIS-128L.
//!
//! Measures encrypt and decrypt throughput at 64 KiB.
//! Acceptance thresholds:
//! - AES-256-GCM seal >= 3.0 GiB/s
//! - AEGIS-128L seal >= 6.0 GiB/s
//! - AEGIS-128L seal/open ratio <= 1.02 (structural symmetry)

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use rekindle_aead::aes_gcm::AesGcmKey;
use rekindle_aead::BulkAead;

const DATA_SIZE: usize = 65519;

fn print_capabilities() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Quick inline probe — rekindle-aead doesn't depend on rekindle-node
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let chunk = vec![0xABu8; 65536];
        let mut buf = chunk.clone();
        let start = std::time::Instant::now();
        for i in 0..500u64 {
            buf.copy_from_slice(&chunk);
            let _ = key.seal_in_place(&key.build_nonce(i), b"", &mut buf);
        }
        let gcm = (65536.0 * 500.0) / (1024.0 * 1024.0 * 1024.0) / start.elapsed().as_secs_f64();

        #[cfg(feature = "aegis")]
        let aegis = {
            let akey = rekindle_aead::aegis128l::Aegis128LKey::new(&[0x42; 16]);
            let nonce = akey.build_nonce(0);
            let mut ct = vec![0u8; 65536];
            let mut tag = [0u8; 16];
            let start = std::time::Instant::now();
            for _ in 0..500 {
                let _ = akey.seal_detached(&nonce, b"", &chunk, &mut ct, &mut tag);
            }
            (65536.0 * 500.0) / (1024.0 * 1024.0 * 1024.0) / start.elapsed().as_secs_f64()
        };
        #[cfg(not(feature = "aegis"))]
        let aegis = 0.0;

        eprintln!("\n[bench] AES-256-GCM: {gcm:.2} GiB/s | AEGIS-128L: {aegis:.2} GiB/s");
    });
}

fn bench_aes_gcm(c: &mut Criterion) {
    print_capabilities();
    let key = AesGcmKey::new(&[0x42; 32]).unwrap();
    let plain = vec![0xABu8; DATA_SIZE];
    let mut ct = vec![0u8; DATA_SIZE];
    let mut tag = [0u8; 16];
    let mut pt = vec![0u8; DATA_SIZE];

    let mut group = c.benchmark_group("aead_aes256gcm");
    group.throughput(Throughput::Bytes(DATA_SIZE as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("seal_64KiB", |b| {
        let mut ctr = 0u64;
        b.iter(|| {
            ctr += 1;
            let nonce = key.build_nonce(ctr);
            key.seal_detached(&nonce, b"", black_box(&plain), &mut ct, &mut tag).unwrap();
            black_box(&ct);
        });
    });

    group.bench_function("open_64KiB", |b| {
        let nonce = key.build_nonce(0);
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        b.iter(|| {
            key.open_detached(&nonce, b"", black_box(&ct), &tag, &mut pt).unwrap();
            black_box(&pt);
        });
    });

    group.finish();
}

#[cfg(feature = "aegis")]
fn bench_aegis128l(c: &mut Criterion) {
    use rekindle_aead::aegis128l::Aegis128LKey;

    let key = Aegis128LKey::new(&[0x42; 16]);
    let plain = vec![0xABu8; DATA_SIZE];
    let mut ct = vec![0u8; DATA_SIZE];
    let mut tag = [0u8; 16];
    let mut pt = vec![0u8; DATA_SIZE];

    let mut group = c.benchmark_group("aead_aegis128l");
    group.throughput(Throughput::Bytes(DATA_SIZE as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    group.bench_function("seal_64KiB", |b| {
        let mut ctr = 0u64;
        b.iter(|| {
            ctr += 1;
            let nonce = key.build_nonce(ctr);
            key.seal_detached(&nonce, b"", black_box(&plain), &mut ct, &mut tag).unwrap();
            black_box(&ct);
        });
    });

    group.bench_function("open_64KiB", |b| {
        let nonce = key.build_nonce(0);
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        b.iter(|| {
            key.open_detached(&nonce, b"", black_box(&ct), &tag, &mut pt).unwrap();
            black_box(&pt);
        });
    });

    group.finish();
}

#[cfg(feature = "aegis")]
criterion_group!(benches, bench_aes_gcm, bench_aegis128l);
#[cfg(not(feature = "aegis"))]
criterion_group!(benches, bench_aes_gcm);
criterion_main!(benches);
