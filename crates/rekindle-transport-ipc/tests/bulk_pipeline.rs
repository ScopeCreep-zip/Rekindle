//! Bulk pipeline tests: rayon encrypt → channel → decrypt → reassembly.
//!
//! Tests the in-process pipeline WITHOUT sockets. Isolates crypto throughput,
//! channel backpressure, and reassembly correctness from the IO loop.
//!
//! Uses std::sync::mpsc (not tokio::sync::mpsc) per rayon's canonical pattern:
//! - rx.iter() terminates when all Sender clones are dropped
//! - recv_timeout provides hard deadlines
//! - No risk of parking a tokio worker thread
//!
//! Upstream reference: rayon-core/src/spawn/test.rs

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_transport_ipc::bulk::{
    BulkCounters,
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk, DEFAULT_REASSEMBLY_CAPACITY},
    encrypt::build_encrypt_pool,
    frame::MAX_CHUNK_PLAIN,
    nonce::NonceCounter,
    pool::BufferPool,
    reassembly::Reassembler,
    transfer::{send_payload, BulkTransferAccumulator},
    verify::DigestAlgorithm,
};

fn make_payload(size: usize, seed: u8) -> Vec<u8> {
    (0..size).map(|i| ((i % 251) as u8).wrapping_add(seed)).collect()
}

// ---- Stage 1: Encrypt pipeline produces correct frame count ----

/// send_payload with N data chunks produces N+1 frames (N data + 1 fin).
/// Uses std::sync::mpsc for timeout-safe drain.
#[test]
fn encrypt_produces_correct_frame_count() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    // 3 full chunks = 3 data + 1 fin = 4 frames
    let payload = make_payload(3 * MAX_CHUNK_PLAIN, 0);
    let expected_frames = 3 + 1; // 3 data + 1 fin

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut count = 0u64;
        while let Some(_) = rx.blocking_recv() { count += 1; }
        let _ = done_tx.send(count);
    });

    let count = done_rx.recv_timeout(Duration::from_secs(10))
        .expect("encrypt pipeline stalled: channel never closed within 10s");
    assert_eq!(count, expected_frames, "expected {expected_frames} frames, got {count}");
}

/// Empty payload produces exactly 1 frame (fin-only with merkle root).
#[test]
fn encrypt_empty_payload_produces_one_frame() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, b"", DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut count = 0u64;
        while let Some(_) = rx.blocking_recv() { count += 1; }
        let _ = done_tx.send(count);
    });

    let count = done_rx.recv_timeout(Duration::from_secs(5))
        .expect("empty payload encrypt stalled");
    assert_eq!(count, 1, "empty payload must produce exactly 1 fin frame");
}

/// Exact MAX_CHUNK_PLAIN payload: 1 data + 1 fin = 2 frames.
/// This was the boundary condition that triggered the original assert failure.
#[test]
fn encrypt_exact_chunk_boundary() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = make_payload(MAX_CHUNK_PLAIN, 0);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut count = 0u64;
        while let Some(_) = rx.blocking_recv() { count += 1; }
        let _ = done_tx.send(count);
    });

    let count = done_rx.recv_timeout(Duration::from_secs(5))
        .expect("exact boundary encrypt stalled");
    assert_eq!(count, 2, "exact MAX_CHUNK_PLAIN must produce 2 frames (1 data + 1 fin)");
}

// ---- Stage 2: Full encrypt → decrypt → reassembly roundtrip ----

/// Small payload roundtrip with timeout protection.
#[test]
fn roundtrip_small_with_timeout() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = b"hello bulk pipeline test";

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, payload, DigestAlgorithm::Blake3);

    // Drain encrypted frames with timeout.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() { frames.push(f); }
        let _ = done_tx.send(frames);
    });
    let frames = done_rx.recv_timeout(Duration::from_secs(5))
        .expect("encrypt stalled");
    assert!(!frames.is_empty());

    // Decrypt and reassemble.
    let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Blake3, BulkCounters::new(),
    );
    let mut reassembler = Reassembler::new(1024);
    let mut acc = BulkTransferAccumulator::new(payload.len() as u64);

    for frame in frames {
        dispatcher.dispatch(frame).unwrap();
    }
    drop(dispatcher);

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }
        let _ = result_tx.send(result);
    });

    let result = result_rx.recv_timeout(Duration::from_secs(5))
        .expect("reassembly stalled")
        .expect("no complete payload assembled");
    assert_eq!(result, payload);
}

/// 1MB roundtrip — exercises multiple chunks with Merkle verification.
#[test]
fn roundtrip_1mb() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = make_payload(1_000_000, 42);
    let start = Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() { frames.push(f); }
        let _ = done_tx.send(frames);
    });
    let frames = done_rx.recv_timeout(Duration::from_secs(10))
        .expect("1MB encrypt stalled");

    let encrypt_elapsed = start.elapsed();
    tracing::info!(
        frames = frames.len(),
        encrypt_ms = encrypt_elapsed.as_millis() as u64,
        "1MB encrypt complete"
    );

    let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    for frame in frames {
        dispatcher.dispatch(frame).unwrap();
    }
    drop(dispatcher);

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reassembler = Reassembler::new(1024);
        let mut acc = BulkTransferAccumulator::new(0);
        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }
        let _ = result_tx.send(result);
    });

    let result = result_rx.recv_timeout(Duration::from_secs(10))
        .expect("1MB reassembly stalled")
        .expect("no complete payload");

    let total_elapsed = start.elapsed();
    let throughput_mibs = 1.0 / total_elapsed.as_secs_f64();
    tracing::info!(
        total_ms = total_elapsed.as_millis() as u64,
        throughput_mibs = throughput_mibs as u64,
        "1MB roundtrip complete"
    );

    common::assert_payload_eq(&result, &payload);
}

/// 10MB roundtrip — sustained pipeline under load.
#[test]
fn roundtrip_10mb() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(256);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = make_payload(10_000_000, 99);
    let start = Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() { frames.push(f); }
        let _ = done_tx.send(frames);
    });
    let frames = done_rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| {
            panic!(
                "10MB encrypt stalled after 30s. Elapsed: {:?}. \
                 Check rayon pool size, channel capacity, or worker panic.",
                start.elapsed()
            )
        });

    let encrypt_elapsed = start.elapsed();

    let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    for frame in frames {
        dispatcher.dispatch(frame).unwrap();
    }
    drop(dispatcher);

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reassembler = Reassembler::new(1024);
        let mut acc = BulkTransferAccumulator::new(0);
        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }
        let _ = result_tx.send(result);
    });

    let result = result_rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| {
            panic!(
                "10MB reassembly stalled after 30s. Encrypt took {:?}. \
                 Check dispatcher dispatch rate or reassembly buffer.",
                encrypt_elapsed
            )
        })
        .expect("no complete payload");

    let total_elapsed = start.elapsed();
    let throughput_mibs = 10.0 / total_elapsed.as_secs_f64();
    tracing::info!(
        encrypt_ms = encrypt_elapsed.as_millis() as u64,
        total_ms = total_elapsed.as_millis() as u64,
        throughput_mibs = throughput_mibs as u64,
        "10MB roundtrip complete"
    );

    common::assert_payload_eq(&result, &payload);
}

// ---- Stage 3: Channel backpressure under constrained capacity ----

/// Small channel capacity (8) with 100 chunks. Rayon workers must park
/// on blocking_send when full and resume when the consumer drains.
/// No data loss, no deadlock.
#[test]
fn backpressure_small_channel() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(256);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    // ~100 chunks × 65KB = ~6.5MB
    let payload = make_payload(100 * MAX_CHUNK_PLAIN, 77);

    // Tiny encrypt channel — forces encrypt-side backpressure.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

    // Drain encrypted frames through a slow consumer (1ms per read).
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() {
            frames.push(f);
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = done_tx.send(frames);
    });

    let frames = done_rx.recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("backpressure encrypt stalled — possible deadlock"));

    // Full decrypt → reassembly under constrained reassembly channel (8).
    let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    for frame in frames {
        dispatcher.dispatch(frame).unwrap();
    }
    drop(dispatcher);

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reassembler = Reassembler::new(2048);
        let mut acc = BulkTransferAccumulator::new(0);
        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }
        let _ = result_tx.send(result);
    });

    let result = result_rx.recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("backpressure reassembly stalled — possible deadlock"))
        .expect("no payload under backpressure");
    common::assert_payload_eq(&result, &payload);
}

// ---- Stage 4: SHA-256 algorithm path ----

/// Roundtrip using SHA-256 instead of BLAKE3. Exercises the alternate
/// Merkle path and parallel SHA-256 multi-buffer implementation.
#[test]
fn roundtrip_sha256() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = make_payload(500_000, 33);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Sha256);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() { frames.push(f); }
        let _ = done_tx.send(frames);
    });
    let frames = done_rx.recv_timeout(Duration::from_secs(10)).expect("SHA-256 encrypt stalled");

    let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Sha256, BulkCounters::new(),
    );

    for frame in frames {
        dispatcher.dispatch(frame).unwrap();
    }
    drop(dispatcher);

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reassembler = Reassembler::with_algorithm(1024, DigestAlgorithm::Sha256);
        let mut acc = BulkTransferAccumulator::new(0);
        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }
        let _ = result_tx.send(result);
    });

    let result = result_rx.recv_timeout(Duration::from_secs(10))
        .expect("SHA-256 reassembly stalled")
        .expect("no payload");
    common::assert_payload_eq(&result, &payload);
}

// ---- Stage 5: Replay filter under pipeline load ----

/// Dispatch the same frame twice — second must be rejected as replay.
/// Proves the replay filter works within the dispatcher pipeline.
#[test]
fn replay_rejected_in_pipeline() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(16);

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    let payload = b"replay test data";

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, payload, DigestAlgorithm::Blake3);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        while let Some(f) = rx.blocking_recv() { frames.push(f); }
        let _ = done_tx.send(frames);
    });
    let frames = done_rx.recv_timeout(Duration::from_secs(5)).expect("encrypt stalled");

    let (reassembly_tx, _reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
        DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    // First dispatch: all frames accepted.
    for frame in &frames {
        dispatcher.dispatch(frame.clone()).unwrap();
    }

    // Second dispatch: all frames rejected as replay.
    for frame in &frames {
        let result = dispatcher.dispatch(frame.clone());
        assert!(result.is_err(), "replayed frame must be rejected");
    }
}

// ---- Stage 6: Sequential transfers reuse same pipeline ----

/// 5 sequential transfers on the same cipher/nonce counter.
/// Proves nonce counter advances correctly across transfers and
/// no state leaks between transfers.
#[test]
fn sequential_transfers_no_state_leak() {
    common::init_tracing();
    let pool = build_encrypt_pool(4);
    let buf_pool = BufferPool::new(64);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let nonce = Arc::new(NonceCounter::new());

    for round in 0u8..5 {
    
        let payload = make_payload(100_000, round * 30);

        // With chunk_seq, each transfer's reassembly ordering starts at 0
        // regardless of the global nonce counter position.

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        send_payload(&pool, &cipher, &nonce, &buf_pool, tx, 0, &payload, DigestAlgorithm::Blake3);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut frames = Vec::new();
            while let Some(f) = rx.blocking_recv() { frames.push(f); }
            let _ = done_tx.send(frames);
        });
        let frames = done_rx.recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("round {round}: encrypt stalled"));

        let (reassembly_tx, mut reassembly_rx) = tokio::sync::mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), Arc::clone(&pool), reassembly_tx,
            DigestAlgorithm::Blake3, BulkCounters::new(),
        );

        for frame in frames {
            dispatcher.dispatch(frame).unwrap();
        }
        drop(dispatcher);

        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Each transfer uses chunk_seq starting at 0 — fresh reassembler
            // handles this correctly without any reset.
            let mut reassembler = Reassembler::new(1024);
            let mut acc = BulkTransferAccumulator::new(0);
            let mut result = None;
            while let Some(chunk) = reassembly_rx.blocking_recv() {
                for r in reassembler.process(chunk).unwrap() {
                    if let Some(complete) = acc.push(&r) {
                        result = Some(complete);
                    }
                }
                if acc.is_complete() { break; }
            }
            let _ = result_tx.send(result);
        });

        let result = result_rx.recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("round {round}: reassembly stalled"))
            .unwrap_or_else(|| panic!("round {round}: no payload"));
        common::assert_payload_eq(&result, &payload);
    }
}
