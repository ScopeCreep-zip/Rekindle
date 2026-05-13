//! End-to-end bulk transfer benchmark: encrypt → channel → decrypt → reassemble.
//!
//! Measures the complete pipeline throughput through the entire software
//! stack without socket I/O. Tests both BLAKE3 and SHA-256 Merkle paths.
//!
//! Acceptance threshold:
//! - `bulk_e2e/blake3_16MiB` >= 1.25 GB/s (10 Gbps)

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use rekindle_node::ipc::bulk::{
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk},
    encrypt::build_encrypt_pool,
    frame::MAX_CHUNK_PLAIN,
    nonce::NonceCounter,
    pool::BufferPool,
    reassembly::Reassembler,
    stream::BulkStream,
    verify::{DigestAlgorithm, digest_oneshot},
};
use std::sync::Arc;

fn make_encrypted_frames(
    cipher: &Arc<BulkCipher>,
    encrypt_pool: &Arc<rayon::ThreadPool>,
    buffer_pool: &Arc<BufferPool>,
    data_chunk: &[u8],
    fin_plaintext: &[u8],
    num_chunks: usize,
) -> Vec<Vec<u8>> {
    let (stream_tx, stream_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
    let stream = BulkStream::new(
        0, Arc::clone(cipher),
        Arc::new(NonceCounter::new()),
        Arc::clone(buffer_pool), stream_tx,
    );

    for i in 0..num_chunks {
        if i == num_chunks - 1 {
            stream.submit_chunk(encrypt_pool, fin_plaintext.to_vec(), true);
        } else {
            stream.submit_chunk(encrypt_pool, data_chunk.to_vec(), false);
        }
    }

    let mut frames = Vec::with_capacity(num_chunks);
    for _ in 0..num_chunks {
        frames.push(
            stream_rx.recv_timeout(std::time::Duration::from_secs(10))
                .expect("timed out waiting for encrypted frame")
        );
    }

    frames
}

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

fn bench_e2e(c: &mut Criterion, algo: DigestAlgorithm, name: &str) {
    print_capabilities();
    let encrypt_pool = build_encrypt_pool();
    let buffer_pool = BufferPool::new();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));

    let chunk_size = MAX_CHUNK_PLAIN;
    let num_chunks = 256usize;
    let total_bytes = num_chunks * chunk_size;
    let data_chunk = vec![0xABu8; chunk_size];

    let chunk_digest = digest_oneshot(algo, &data_chunk);
    let mut merkle_input = Vec::with_capacity(num_chunks * 32);
    for _ in 0..num_chunks { merkle_input.extend_from_slice(&chunk_digest); }
    let merkle_root = digest_oneshot(algo, &merkle_input);

    let mut fin_plaintext = Vec::with_capacity(32 + chunk_size);
    fin_plaintext.extend_from_slice(&merkle_root);
    fin_plaintext.extend_from_slice(&data_chunk);

    let mut group = c.benchmark_group("bulk_e2e");
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.measurement_time(std::time::Duration::from_secs(20));

    // Pre-encrypt frames ONCE, outside the bench closure entirely.
    // Criterion calls the |b| closure multiple times (warmup + samples).
    // If make_encrypted_frames runs inside |b|, it submits encrypt tasks
    // to the same rayon pool that still has decrypt tasks queued from
    // the previous iteration, causing pool starvation and timeouts.
    let frames = make_encrypted_frames(
        &cipher, &encrypt_pool, &buffer_pool,
        &data_chunk, &fin_plaintext, num_chunks,
    );

    group.bench_function(name, |b| {
        b.iter(|| {
            let recv_pool = rekindle_node::ipc::bulk::pool::BufferPool::new();
            let (tx, rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
            let mut dispatcher = BulkDispatcher::with_algorithm(
                Arc::clone(&cipher), Arc::clone(&encrypt_pool), tx, algo, recv_pool,
            );

            let reassembly_handle = std::thread::spawn(move || {
                let mut reassembler = Reassembler::with_algorithm(1024, algo);
                let mut total = 0usize;
                while let Ok(chunk) = rx.recv() {
                    total += reassembler.process(chunk).unwrap().len();
                }
                total
            });

            for frame in &frames {
                dispatcher.dispatch(frame.clone()).unwrap();
            }
            drop(dispatcher);

            let total = reassembly_handle.join().unwrap();
            assert_eq!(total, num_chunks);
        });
    });
    group.finish();
}

fn bench_e2e_blake3(c: &mut Criterion) {
    bench_e2e(c, DigestAlgorithm::Blake3, "blake3_16MiB");
}

fn bench_e2e_sha256(c: &mut Criterion) {
    bench_e2e(c, DigestAlgorithm::Sha256, "sha256_16MiB");
}

criterion_group!(benches, bench_e2e_blake3, bench_e2e_sha256);
criterion_main!(benches);
