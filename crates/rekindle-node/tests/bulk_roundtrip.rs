//! Integration test: full bulk roundtrip.
//!
//! Proves the entire pipeline composes correctly:
//! chunk → BulkStream submit → rayon encrypt → crossbeam channel →
//! BulkDispatcher → rayon decrypt + digest → reassembly → Merkle verify.
//!
//! Default algorithm is BLAKE3 Merkle.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use bytes::Bytes;
use rekindle_node::ipc::bulk::{
    self,
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk},
    encrypt::build_encrypt_pool,
    frame::{MAX_CHUNK_PLAIN, HEADER_LEN, TAG_LEN},
    pool::BufferPool,
    reassembly::Reassembler,
    stream::BulkStream,
    verify::merkle_root,
};

/// Roundtrip: encrypt chunks, feed through dispatcher, reassemble, verify digest.
///
/// This test exercises the in-process pipeline without a real socket.
/// It proves that the encrypt → dispatch → decrypt → reassemble chain
/// produces correct, verified output.
#[test]
fn bulk_encrypt_decrypt_reassemble_roundtrip() {
    let encrypt_pool = build_encrypt_pool();
    let buffer_pool = BufferPool::new();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));

    // ── Send side: encrypt chunks ───────────────────────────────
    let (stream_tx, stream_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
    let stream = BulkStream::new(
        0,
        Arc::clone(&cipher),
        Arc::new(AtomicU64::new(0)),
        buffer_pool,
        stream_tx,
    );

    // 1 MiB of test data, split into MAX_CHUNK_PLAIN chunks.
    let total_size = 1024 * 1024;
    let test_data: Vec<u8> = (0..total_size).map(|i| (i % 251) as u8).collect();

    let chunks: Vec<&[u8]> = test_data.chunks(MAX_CHUNK_PLAIN).collect();
    let num_chunks = chunks.len();

    // Compute the Merkle root: sha256(sha256(chunk_0) || sha256(chunk_1) || ...)
    let merkle_root = merkle_root(&chunks);

    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i == num_chunks - 1;
        let payload = chunk.to_vec();
        if is_last {
            // BulkFin: prepend the 32-byte Merkle root to the last chunk's plaintext.
            let mut fin_payload = merkle_root.to_vec();
            fin_payload.extend_from_slice(chunk);
            stream.submit_chunk(&encrypt_pool, Bytes::from(fin_payload), true);
        } else {
            stream.submit_chunk(&encrypt_pool, Bytes::from(payload), false);
        }
    }

    // Collect all encrypted frames.
    let mut encrypted_frames = Vec::new();
    for _ in 0..num_chunks {
        let frame = stream_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("timed out waiting for encrypted frame");
        encrypted_frames.push(frame);
    }
    assert_eq!(encrypted_frames.len(), num_chunks);

    // ── Receive side: dispatch → decrypt → reassemble ───────────
    let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher),
        encrypt_pool,
        reassembly_tx,
    );
    let mut reassembler = Reassembler::new(1024);

    // The slab IS the frame body: [header(10)][ct][tag].
    // No lane byte, no length prefix — those are added by the write path.
    // The dispatcher expects exactly this format.
    for frame in &encrypted_frames {
        dispatcher
            .dispatch(frame.clone())
            .expect("dispatch should succeed");
    }

    // Collect decrypted chunks and reassemble.
    let mut all_plaintext = Vec::new();
    let mut total_delivered = 0usize;

    for _ in 0..num_chunks {
        let chunk = reassembly_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("timed out waiting for decrypted chunk");
        let delivered = reassembler
            .process(chunk)
            .expect("reassembly should succeed");
        for reassembled in &delivered {
            all_plaintext.extend_from_slice(&reassembled.plaintext);
        }
        total_delivered += delivered.len();
    }

    assert_eq!(total_delivered, num_chunks);
    assert_eq!(all_plaintext.len(), total_size);
    assert_eq!(&all_plaintext, &test_data);
}

/// Verify that the buffer pool does not drain after a full roundtrip.
#[test]
fn buffer_pool_no_drain_after_roundtrip() {
    let encrypt_pool = build_encrypt_pool();
    let pool = BufferPool::new();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let initial_available = pool.available();

    let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);
    let stream = BulkStream::new(
        0,
        cipher,
        Arc::new(AtomicU64::new(0)),
        Arc::clone(&pool),
        tx,
    );

    // Submit and drain 100 chunks.
    for i in 0..100 {
        stream.submit_chunk(&encrypt_pool, Bytes::from(vec![0xAB; 1024]), i == 99);
    }
    for _ in 0..100 {
        let slab = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        // Simulate the writer returning the slab to the pool.
        pool.replenish(slab);
    }

    // Pool should be back to full.
    assert_eq!(pool.available(), initial_available);
}

/// Verify replay filter rejects duplicate nonces in the dispatcher.
#[test]
fn dispatcher_rejects_replay() {
    let encrypt_pool = build_encrypt_pool();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let (tx, _rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);
    let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), encrypt_pool, tx);

    // Create an encrypted frame.
    let header = bulk::BulkFrameHeader::new(0, bulk::FrameKind::BulkData, 42);
    let hdr_bytes = header.encode_array();
    let mut frame = Vec::new();
    frame.extend_from_slice(&hdr_bytes);
    frame.extend_from_slice(&[0xAB; 10]);
    let tag = cipher
        .seal_in_place(42, &hdr_bytes, &mut frame[HEADER_LEN..HEADER_LEN + 10])
        .unwrap();
    frame.extend_from_slice(&tag);

    // First dispatch succeeds.
    assert!(dispatcher.dispatch(frame.clone()).is_ok());

    // Second dispatch with same nonce fails (replay).
    assert!(dispatcher.dispatch(frame).is_err());
}
