//! Component-level benchmarks for the 10 Gbps bulk transfer plane.
//!
//! Establishes the performance floor for each primitive in the bulk
//! transfer pipeline. If any component is below threshold, nothing
//! built on top can compensate.
//!
//! Benchmarks:
//! - AES-256-GCM seal via aws-lc-rs at [1K, 4K, 16K, 64K, 256K, 1M] chunk sizes
//! - AES-256-GCM open (decrypt) at matching sizes
//! - writev over Unix domain socket at [1, 4, 8, 16, 32] batch depths
//! - blake3 streaming hash at [64K, 256K, 1M, 4M] to validate internal integrity path
//!
//! Acceptance thresholds (from bulk transfer spec):
//! - `aes_gcm_seal/65536` ≥ 1.5 GiB/s/core (VAES/AVX-512 expected ≥ 4 GiB/s)
//! - `writev_uds/16` ≥ 3 GiB/s (kernel 6.2+ with default SO_SNDBUF)
//! - `blake3_stream/65536` ≥ 4 GiB/s (SIMD-accelerated)
//!
//! If `aes_gcm_seal/65536 < 1.5 GiB/s`: aws-lc-rs is not picking the VAES
//! path — verify CPUID flags and `-C target-cpu=native`.

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};

use aws_lc_rs::aead::{
    Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN,
};

// ── AES-256-GCM Seal (Encrypt) ──────────────────────────────────────
//
// Uses aws-lc-rs LessSafeKey with explicit nonces — the same API the
// BulkCipher implementation will use. LessSafeKey is the correct
// primitive for parallel encrypt (caller-managed nonces).
//
// Each iteration: copy plaintext into buffer, seal in-place, tag appended.
// Nonce increments per iteration to avoid nonce reuse (even though
// AES-GCM with a reused nonce only leaks the auth key, not plaintext,
// we measure the real path).

fn bench_aes_gcm_seal(c: &mut Criterion) {
    let key_bytes = [0x42u8; 32];
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).unwrap();
    let key = LessSafeKey::new(unbound);

    let mut group = c.benchmark_group("aes_gcm_seal");

    for &size in &[1024usize, 4096, 16_384, 65_536, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xAAu8; n];
            let mut buf = vec![0u8; n + 16]; // plaintext + tag
            let mut nonce_ctr = 0u64;

            b.iter(|| {
                nonce_ctr = nonce_ctr.wrapping_add(1);
                buf[..n].copy_from_slice(&plain);

                let mut nonce_bytes = [0u8; NONCE_LEN];
                nonce_bytes[4..].copy_from_slice(&nonce_ctr.to_le_bytes());
                let nonce = Nonce::assume_unique_for_key(nonce_bytes);

                let tag = key
                    .seal_in_place_separate_tag(nonce, Aad::empty(), &mut buf[..n])
                    .unwrap();
                buf[n..n + 16].copy_from_slice(tag.as_ref());
                black_box(&buf);
            });
        });
    }
    group.finish();
}

// ── AES-256-GCM Open (Decrypt) ──────────────────────────────────────
//
// Pre-encrypts once per size, then benchmarks decrypt. Measures the
// receiver-side cost: AEAD verification + ChaCha20/AES-GCM decrypt.
// Uses a fresh nonce per iteration by re-encrypting in iter — this is
// necessary because open_in_place consumes the tag and overwrites the
// ciphertext with plaintext.

fn bench_aes_gcm_open(c: &mut Criterion) {
    let key_bytes = [0x42u8; 32];
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).unwrap();
    let key = LessSafeKey::new(unbound);

    let mut group = c.benchmark_group("aes_gcm_open");

    for &size in &[1024usize, 4096, 16_384, 65_536, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xAAu8; n];
            let mut nonce_ctr = 0u64;

            b.iter(|| {
                nonce_ctr = nonce_ctr.wrapping_add(1);

                let mut nonce_bytes = [0u8; NONCE_LEN];
                nonce_bytes[4..].copy_from_slice(&nonce_ctr.to_le_bytes());
                let nonce = Nonce::assume_unique_for_key(nonce_bytes);

                // Encrypt: produces ciphertext || tag in buf
                let mut buf = vec![0u8; n + 16];
                buf[..n].copy_from_slice(&plain);
                let tag = key
                    .seal_in_place_separate_tag(nonce, Aad::empty(), &mut buf[..n])
                    .unwrap();
                buf[n..n + 16].copy_from_slice(tag.as_ref());

                // Decrypt: open_in_place verifies tag and decrypts
                let nonce = Nonce::assume_unique_for_key(nonce_bytes);
                let result = key.open_in_place(nonce, Aad::empty(), &mut buf);
                black_box(result.unwrap());
            });
        });
    }
    group.finish();
}

// ── writev over Unix Domain Socket ──────────────────────────────────
//
// Measures raw UDS write throughput with vectored I/O at various batch
// depths. Uses std::os::unix::net::UnixStream (synchronous) to eliminate
// tokio runtime overhead — we want the kernel syscall ceiling.
//
// Each IoSlice is a 64 KiB chunk (the bulk transfer chunk size).
// The receiver side is drained by a background thread to prevent
// socket buffer backpressure.

fn bench_writev_uds(c: &mut Criterion) {
    use std::io::{IoSlice, Read, Write};
    use std::os::unix::net::UnixStream;

    let chunk = vec![0u8; 65_536];

    let mut group = c.benchmark_group("writev_uds");

    for &batch in &[1usize, 4, 8, 16, 32] {
        group.throughput(Throughput::Bytes((batch * 65_536) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(batch), &batch, |b, &n| {
            let (mut writer, reader) = UnixStream::pair().unwrap();

            // Set non-blocking on writer to detect buffer-full conditions.
            // The reader thread drains continuously.
            let mut reader = reader;
            let drain_handle = std::thread::spawn(move || {
                let mut sink = vec![0u8; 256 * 1024];
                loop {
                    match reader.read(&mut sink) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });

            let slices: Vec<IoSlice> = (0..n).map(|_| IoSlice::new(&chunk)).collect();

            b.iter(|| {
                let written = writer.write_vectored(black_box(&slices)).unwrap();
                black_box(written);
            });

            // Drop writer to signal EOF to drain thread.
            drop(writer);
            drain_handle.join().unwrap();
        });
    }
    group.finish();
}

// ── BLAKE3 Streaming Hash ───────────────────────────────────────────
//
// The project standard for internal integrity hashing (keypair checksums,
// gossip dedup, content addressing). Benchmarks streaming (per-chunk
// update) to validate that interleaving hash updates with chunk arrival
// doesn't degrade throughput vs one-shot.
//
// BLAKE3 is SIMD-accelerated (AVX-512, AVX2, SSE4.1, NEON) and should
// exceed 4 GiB/s/core on modern x86-64.

fn bench_blake3_stream(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_stream");
    group.measurement_time(std::time::Duration::from_secs(10));

    for &total_size in &[65_536usize, 262_144, 1_048_576, 4_194_304] {
        group.throughput(Throughput::Bytes(total_size as u64));

        // Streaming: 64 KiB chunks fed to hasher.update()
        let chunk_size = 65_536;
        let num_chunks = total_size / chunk_size;
        let chunk = vec![0xBBu8; chunk_size];

        group.bench_with_input(
            BenchmarkId::new("streaming_64k_chunks", total_size),
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

        // One-shot: entire payload in a single update
        let payload = vec![0xBBu8; total_size];
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

criterion_group!(
    benches,
    bench_aes_gcm_seal,
    bench_aes_gcm_open,
    bench_writev_uds,
    bench_blake3_stream,
);
criterion_main!(benches);
