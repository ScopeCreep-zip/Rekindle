//! Bulk transfer orchestration: send and receive complete payloads.
//!
//! `BulkTransferSender` takes a byte payload, chunks it into ≤65,519-byte
//! segments, computes the BLAKE3 Merkle root, prepends it to the final
//! chunk, and submits all chunks through the encrypt pipeline.
//!
//! `BulkTransferAccumulator` collects `ReassembledChunk` values from the
//! reassembler and accumulates them into a complete contiguous payload.
//! Returns `Some(Vec<u8>)` when the final chunk arrives with a verified
//! Merkle root.
//!
//! These are the faucet and drain of the bulk transport pipe.

use std::sync::Arc;
use rayon::ThreadPool;

use super::cipher::BulkCipher;
use super::frame::MAX_CHUNK_PLAIN;
use super::lending::ZeroizingStream;
use super::nonce::NonceCounter;
use super::pool::BufferPool;
use super::reassembly::ReassembledChunk;
use super::verify::{DigestAlgorithm, merkle_root_with_algorithm};

/// Sends a complete payload through the bulk encrypt pipeline.
///
/// Chunks the payload, computes the Merkle root, submits all chunks
/// through a `BulkStream`. The encrypted frames appear on `out_tx`
/// (the crossbeam channel passed to `BulkStream::new`).
///
/// This function blocks the calling thread until all chunks are
/// submitted to the rayon pool. The actual encryption is async
/// (rayon workers). The caller must drain `out_rx` and write the
/// frames to the socket.
pub fn send_payload(
    encrypt_pool: &Arc<ThreadPool>,
    cipher: &Arc<BulkCipher>,
    nonce_ctr: &Arc<NonceCounter>,
    pool: &Arc<BufferPool>,
    out_tx: crossbeam::channel::Sender<Vec<u8>>,
    stream_id: u8,
    payload: &[u8],
    algorithm: DigestAlgorithm,
) {
    let stream = ZeroizingStream::new(
        stream_id,
        Arc::clone(cipher),
        Arc::clone(nonce_ctr),
        Arc::clone(pool),
        out_tx,
    );

    let chunks: Vec<&[u8]> = payload.chunks(MAX_CHUNK_PLAIN).collect();
    let num_chunks = chunks.len();

    if num_chunks == 0 {
        // Empty payload: send a single BulkFin with just the Merkle root.
        let merkle = merkle_root_with_algorithm(&[], algorithm);
        stream.submit_chunk(encrypt_pool, &merkle, true);
        return;
    }

    // Compute Merkle root over all chunk digests.
    let merkle = merkle_root_with_algorithm(&chunks, algorithm);

    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i == num_chunks - 1;
        if is_last {
            // Final chunk: prepend 32-byte Merkle root.
            let mut fin_payload = Vec::with_capacity(32 + chunk.len());
            fin_payload.extend_from_slice(&merkle);
            fin_payload.extend_from_slice(chunk);
            stream.submit_chunk(encrypt_pool, &fin_payload, true);
        } else {
            stream.submit_chunk(encrypt_pool, chunk, false);
        }
    }
}

/// Accumulates reassembled chunks into a complete payload.
///
/// Call `push()` for each `ReassembledChunk` delivered by the
/// reassembler. Returns `None` until the final chunk arrives,
/// then returns `Some(complete_payload)`.
///
/// The accumulator pre-allocates based on `expected_size` if provided.
pub struct BulkTransferAccumulator {
    buf: Vec<u8>,
    chunks_received: u64,
    complete: bool,
}

impl BulkTransferAccumulator {
    /// Create a new accumulator.
    ///
    /// `expected_size` is used for pre-allocation. Pass 0 if unknown.
    pub fn new(expected_size: u64) -> Self {
        #[allow(clippy::cast_possible_truncation)]
        let cap = if expected_size > 0 && expected_size < 256 * 1024 * 1024 {
            expected_size as usize
        } else {
            0
        };
        Self {
            buf: Vec::with_capacity(cap),
            chunks_received: 0,
            complete: false,
        }
    }

    /// Push a reassembled chunk. Returns `Some(payload)` on the final chunk.
    pub fn push(&mut self, chunk: &ReassembledChunk) -> Option<Vec<u8>> {
        if self.complete {
            return None;
        }
        self.buf.extend_from_slice(&chunk.plaintext);
        self.chunks_received += 1;
        if chunk.is_last {
            self.complete = true;
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }

    /// Number of chunks received so far.
    pub fn chunks_received(&self) -> u64 {
        self.chunks_received
    }

    /// Total bytes accumulated so far.
    pub fn bytes_received(&self) -> usize {
        self.buf.len()
    }

    /// Whether the final chunk has been received.
    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encrypt::build_encrypt_pool;
    use super::super::dispatcher::{BulkDispatcher, DecryptedChunk};
    use super::super::reassembly::Reassembler;

    #[test]
    fn send_receive_roundtrip_small() {
        let encrypt_pool = build_encrypt_pool();
        let buffer_pool = BufferPool::new();
        let recv_pool = BufferPool::new();
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let nonce_ctr = Arc::new(NonceCounter::new());

        let payload = b"hello bulk transfer end to end";
        let (stream_tx, stream_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

        send_payload(
            &encrypt_pool, &cipher, &nonce_ctr, &buffer_pool,
            stream_tx, 0, payload, DigestAlgorithm::Blake3,
        );

        // Collect encrypted frames.
        let mut frames = Vec::new();
        while let Ok(frame) = stream_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            frames.push(frame);
        }
        assert!(!frames.is_empty());

        // Receive side: dispatch → decrypt → reassemble → accumulate.
        let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), Arc::clone(&encrypt_pool), reassembly_tx, recv_pool,
        );
        let mut reassembler = Reassembler::new(1024);
        let mut accumulator = BulkTransferAccumulator::new(payload.len() as u64);

        for frame in frames {
            dispatcher.dispatch(frame).unwrap();
        }

        let mut result = None;
        for _ in 0..100 {
            match reassembly_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(chunk) => {
                    let delivered = reassembler.process(chunk).unwrap();
                    for r in &delivered {
                        if let Some(complete) = accumulator.push(r) {
                            result = Some(complete);
                        }
                    }
                    if accumulator.is_complete() { break; }
                }
                Err(_) => break,
            }
        }

        let received = result.expect("transfer should complete");
        assert_eq!(received, payload);
    }

    #[test]
    fn send_receive_roundtrip_1mib() {
        let encrypt_pool = build_encrypt_pool();
        let buffer_pool = BufferPool::new();
        let recv_pool = BufferPool::new();
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let nonce_ctr = Arc::new(NonceCounter::new());

        let payload: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
        let (stream_tx, stream_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

        send_payload(
            &encrypt_pool, &cipher, &nonce_ctr, &buffer_pool,
            stream_tx, 0, &payload, DigestAlgorithm::Blake3,
        );

        let mut frames = Vec::new();
        while let Ok(frame) = stream_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            frames.push(frame);
        }

        let num_expected_chunks = (payload.len() + MAX_CHUNK_PLAIN - 1) / MAX_CHUNK_PLAIN;
        assert_eq!(frames.len(), num_expected_chunks);

        let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), Arc::clone(&encrypt_pool), reassembly_tx, recv_pool,
        );
        let mut reassembler = Reassembler::new(1024);
        let mut accumulator = BulkTransferAccumulator::new(payload.len() as u64);

        for frame in frames {
            dispatcher.dispatch(frame).unwrap();
        }

        let mut result = None;
        loop {
            match reassembly_rx.recv_timeout(std::time::Duration::from_secs(10)) {
                Ok(chunk) => {
                    let delivered = reassembler.process(chunk).unwrap();
                    for r in &delivered {
                        if let Some(complete) = accumulator.push(r) {
                            result = Some(complete);
                        }
                    }
                    if accumulator.is_complete() { break; }
                }
                Err(_) => panic!("timed out waiting for chunks — got {}/{}", accumulator.chunks_received(), num_expected_chunks),
            }
        }

        let received = result.expect("transfer should complete");
        assert_eq!(received.len(), payload.len());
        assert_eq!(received, payload);
    }

    #[test]
    fn empty_payload_roundtrip() {
        let encrypt_pool = build_encrypt_pool();
        let buffer_pool = BufferPool::new();
        let recv_pool = BufferPool::new();
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let nonce_ctr = Arc::new(NonceCounter::new());

        let (stream_tx, stream_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

        send_payload(
            &encrypt_pool, &cipher, &nonce_ctr, &buffer_pool,
            stream_tx, 0, b"", DigestAlgorithm::Blake3,
        );

        let mut frames = Vec::new();
        while let Ok(frame) = stream_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            frames.push(frame);
        }
        assert_eq!(frames.len(), 1); // Single BulkFin with Merkle root only

        let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), Arc::clone(&encrypt_pool), reassembly_tx, recv_pool,
        );
        let mut reassembler = Reassembler::new(1024);
        let mut accumulator = BulkTransferAccumulator::new(0);

        for frame in frames {
            dispatcher.dispatch(frame).unwrap();
        }

        let chunk = reassembly_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let delivered = reassembler.process(chunk).unwrap();
        for r in &delivered {
            accumulator.push(r);
        }

        assert!(accumulator.is_complete());
        assert_eq!(accumulator.bytes_received(), 0);
    }
}
