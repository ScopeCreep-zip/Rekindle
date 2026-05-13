//! Receive pipeline benchmark: decrypt + reassembly + Merkle digest verification.
//!
//! Benchmarks both SHA-256 and BLAKE3 Merkle digest paths to quantify
//! the throughput gain from BLAKE3. Each benchmark measures the full
//! receive-side pipeline: encrypted frame → rayon decrypt (+ per-chunk
//! digest) → crossbeam channel → reassembly → Merkle verification.
//!
//! Dispatch and reassembly run concurrently on separate threads.
//!
//! Acceptance threshold:
//! - `bulk_receive/sha256_merkle_64KiB` >= 1.25 GB/s (10 Gbps)
//! - `bulk_receive/blake3_merkle_64KiB` >= 1.25 GB/s (10 Gbps)

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use rekindle_node::ipc::bulk::{
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk},
    encrypt::build_encrypt_pool,
    frame::{BulkFrameHeader, FrameKind, HEADER_LEN, MAX_CHUNK_PLAIN},
    reassembly::Reassembler,
    verify::{DigestAlgorithm, digest_oneshot},
};
use std::sync::Arc;

fn make_encrypted_frame(
    cipher: &BulkCipher,
    stream_id: u8,
    kind: FrameKind,
    nonce: u64,
    plaintext: &[u8],
) -> Vec<u8> {
    let header = BulkFrameHeader::new(stream_id, kind, nonce);
    let hdr_bytes = header.encode_array();
    let mut frame = Vec::new();
    frame.extend_from_slice(&hdr_bytes);
    frame.extend_from_slice(plaintext);
    let ct_start = HEADER_LEN;
    let ct_len = plaintext.len();
    let tag = cipher
        .seal_in_place(nonce, &hdr_bytes, &mut frame[ct_start..ct_start + ct_len])
        .unwrap();
    frame.extend_from_slice(&tag);
    frame
}

fn bench_receive(c: &mut Criterion, algo: DigestAlgorithm, name: &str) {
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let decrypt_pool = build_encrypt_pool();

    let mut group = c.benchmark_group("bulk_receive");

    let chunk_size = MAX_CHUNK_PLAIN;
    let num_chunks = 256usize;
    let total_bytes = num_chunks * chunk_size;
    let data_chunk = vec![0xABu8; chunk_size];

    // Compute Merkle root with the specified algorithm.
    let chunk_digest = digest_oneshot(algo, &data_chunk);
    let mut merkle_input = Vec::with_capacity(num_chunks * 32);
    for _ in 0..num_chunks {
        merkle_input.extend_from_slice(&chunk_digest);
    }
    let merkle_root = digest_oneshot(algo, &merkle_input);

    let mut fin_plaintext = Vec::with_capacity(32 + chunk_size);
    fin_plaintext.extend_from_slice(&merkle_root);
    fin_plaintext.extend_from_slice(&data_chunk);

    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.measurement_time(std::time::Duration::from_secs(20));
    group.bench_function(name, |b| {
        let frames: Vec<Vec<u8>> = (0..num_chunks)
            .map(|i| {
                if i == num_chunks - 1 {
                    make_encrypted_frame(&cipher, 0, FrameKind::BulkFin, i as u64, &fin_plaintext)
                } else {
                    make_encrypted_frame(&cipher, 0, FrameKind::BulkData, i as u64, &data_chunk)
                }
            })
            .collect();

        b.iter(|| {
            let (tx, rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
            let mut dispatcher = BulkDispatcher::with_algorithm(
                Arc::clone(&cipher),
                Arc::clone(&decrypt_pool),
                tx,
                algo,
            );

            let reassembly_handle = std::thread::spawn(move || {
                let mut reassembler = Reassembler::with_algorithm(1024, algo);
                let mut total_delivered = 0usize;
                while let Ok(chunk) = rx.recv() {
                    let delivered = reassembler.process(chunk).unwrap();
                    total_delivered += delivered.len();
                }
                total_delivered
            });

            for frame in &frames {
                dispatcher.dispatch(frame.clone()).unwrap();
            }
            drop(dispatcher);

            let total_delivered = reassembly_handle.join().unwrap();
            assert_eq!(total_delivered, num_chunks);
        });
    });
    group.finish();
}

fn bench_receive_sha256(c: &mut Criterion) {
    bench_receive(c, DigestAlgorithm::Sha256, "sha256_merkle_64KiB");
}

fn bench_receive_blake3(c: &mut Criterion) {
    bench_receive(c, DigestAlgorithm::Blake3, "blake3_merkle_64KiB");
}

criterion_group!(benches, bench_receive_sha256, bench_receive_blake3);
criterion_main!(benches);
