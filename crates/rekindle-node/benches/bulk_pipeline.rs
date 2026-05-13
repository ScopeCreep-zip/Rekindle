//! End-to-end encrypt pipeline benchmark.
//!
//! Measures the full send-side pipeline: plaintext → rayon encrypt →
//! crossbeam channel → drain. No socket I/O — isolates the encrypt +
//! framing + channel overhead.
//!
//! The drain thread runs concurrently with submission and replenishes
//! buffer slabs — matching the production write loop lifecycle where
//! slabs are returned to the pool after socket write.
//!
//! Acceptance threshold:
//! - `bulk_pipeline/encrypt_4cores_64KiB` >= 1.25 GB/s (10 Gbps)

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use rekindle_node::ipc::bulk::{
    cipher::BulkCipher,
    pool::BufferPool,
    nonce::NonceCounter,
    stream::BulkStream,
    encrypt::build_encrypt_pool,
    frame::MAX_CHUNK_PLAIN,
};
use std::sync::Arc;

fn print_capabilities() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let caps = rekindle_node::ipc::bulk::capability::probe();
        eprintln!("\n[bench] AES-GCM: {:.2}/{:.2} GiB/s seal/open | AEGIS: {:.2} GiB/s | SHA256-mb: {:.0} MiB/s (SIMD: {}) | BLAKE3: {:.2} GiB/s",
            caps.aes_gcm_seal_gibs, caps.aes_gcm_open_gibs, caps.aegis_seal_gibs,
            caps.sha256_mb_mibs, caps.sha256_mb_simd_active, caps.blake3_gibs);
    });
}

fn bench_pipeline(c: &mut Criterion) {
    print_capabilities();
    let encrypt_pool = build_encrypt_pool();
    let buf_pool = BufferPool::new();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));

    let mut group = c.benchmark_group("bulk_pipeline");

    // 16 MiB total = ~250 chunks of ~64 KiB
    let chunk_size = MAX_CHUNK_PLAIN;
    let total_bytes = 16 * 1024 * 1024usize;
    let num_chunks = total_bytes / chunk_size;

    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.measurement_time(std::time::Duration::from_secs(15));
    group.bench_function("encrypt_4cores_64KiB", |b| {
        b.iter(|| {
            let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

            // Drain concurrently — replenish slabs to prevent pool exhaustion.
            // This matches the production write loop: receive frame, write to
            // socket, return slab to pool.
            let drain_pool = Arc::clone(&buf_pool);
            let drain_handle = std::thread::spawn(move || {
                let mut count = 0usize;
                while let Ok(slab) = rx.recv() {
                    drain_pool.replenish(slab);
                    count += 1;
                }
                count
            });

            let stream = BulkStream::new(
                0,
                Arc::clone(&cipher),
                Arc::new(NonceCounter::new()),
                Arc::clone(&buf_pool),
                tx,
            );
            for i in 0..num_chunks {
                let plain = vec![0xCDu8; chunk_size];
                stream.submit_chunk(&encrypt_pool, plain, i == num_chunks - 1);
            }

            // Drop the stream (and its Sender clone). Rayon tasks still hold
            // their Sender clones until they complete. The drain thread exits
            // when all Sender clones are dropped.
            drop(stream);

            let drained = drain_handle.join().unwrap();
            assert_eq!(drained, num_chunks);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_pipeline);
criterion_main!(benches);
