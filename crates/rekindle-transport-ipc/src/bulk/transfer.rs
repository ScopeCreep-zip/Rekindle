//! Bulk transfer orchestration: send complete payloads.
//!
//! `send_payload` chunks a byte payload, computes the Merkle root,
//! prepends it to the final chunk, and submits all through the encrypt pipeline.
//!
//! `BulkTransferAccumulator` collects ReassembledChunks into a complete payload.

use std::sync::Arc;
use rayon::ThreadPool;
use tokio::sync::mpsc;

use super::cipher::BulkCipher;
use super::frame::MAX_CHUNK_PLAIN;
use super::nonce::NonceCounter;
use super::pool::BufferPool;
use super::reassembly::ReassembledChunk;
use super::stream::ZeroizingStream;
use super::verify::{DigestAlgorithm, merkle_root_with_algorithm};

/// Send a complete payload through the bulk encrypt pipeline.
///
/// `out_tx` is a `tokio::sync::mpsc::Sender` — rayon workers call `blocking_send`
/// directly. The caller must ensure the `Sender` is the only clone that is passed
/// here; additional clones keep the channel alive and prevent the receiver from
/// seeing channel closure as a termination signal.
///
/// The caller should drop `out_tx` after calling this function so that the
/// receiver sees `None` from `recv()` once all rayon workers complete and drop
/// their cloned Senders.
pub fn send_payload(
    encrypt_pool: &Arc<ThreadPool>,
    cipher: &Arc<BulkCipher>,
    nonce_ctr: &Arc<NonceCounter>,
    pool: &Arc<BufferPool>,
    out_tx: mpsc::Sender<Vec<u8>>,
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
        let merkle = merkle_root_with_algorithm(&[], algorithm);
        stream.submit_chunk(encrypt_pool, &merkle, true, 0);
        return;
    }

    let merkle = merkle_root_with_algorithm(&chunks, algorithm);

    // chunk_seq: per-transfer, 0-indexed. Independent of the global nonce.
    let mut chunk_seq: u32 = 0;

    // All data chunks are BulkData — uniform size, uniform handling.
    for chunk in &chunks {
        stream.submit_chunk(encrypt_pool, chunk, false, chunk_seq);
        chunk_seq += 1;
    }

    // Separate BulkFin frame carries ONLY the 32-byte merkle root.
    stream.submit_chunk(encrypt_pool, &merkle, true, chunk_seq);
}

/// Accumulates reassembled chunks into a complete payload.
///
/// Returns `None` until the final chunk arrives, then `Some(payload)`.
pub struct BulkTransferAccumulator {
    buf: Vec<u8>,
    chunks_received: u64,
    complete: bool,
}

impl BulkTransferAccumulator {
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

    pub fn chunks_received(&self) -> u64 {
        self.chunks_received
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::cipher::BulkCipher;
    use super::super::dispatcher::{BulkDispatcher, DecryptedChunk};
    use super::super::encrypt::build_encrypt_pool;
    use super::super::reassembly::Reassembler;

    // This test is #[test] not #[tokio::test] — blocking_recv is safe
    // on plain OS threads. blocking_recv panics inside a tokio runtime.
    #[test]
    fn roundtrip_small() {
        let encrypt_pool = build_encrypt_pool(0);
        let buffer_pool = BufferPool::new(64);
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let nonce_ctr = Arc::new(NonceCounter::new());
        let payload = b"hello bulk transfer";
        let (stream_tx, mut stream_rx) = mpsc::channel::<Vec<u8>>(256);

        send_payload(
            &encrypt_pool, &cipher, &nonce_ctr, &buffer_pool,
            stream_tx, // moved — no other clones. When rayon workers finish
                       // and drop their clones, stream_rx.blocking_recv()
                       // returns None.
            0, payload, DigestAlgorithm::Blake3,
        );

        // Drain until channel closure (all Sender clones dropped by rayon workers).
        // NEVER use is_empty() as termination — it is a point-in-time snapshot.
        let mut frames = Vec::new();
        while let Some(frame) = stream_rx.blocking_recv() {
            frames.push(frame);
        }
        assert!(!frames.is_empty());

        let (reassembly_tx, mut reassembly_rx) = mpsc::channel::<DecryptedChunk>(
            super::super::dispatcher::DEFAULT_REASSEMBLY_CAPACITY,
        );
        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), Arc::clone(&encrypt_pool), reassembly_tx,
            DigestAlgorithm::Blake3,
            super::super::BulkCounters::new(),
        );
        let mut reassembler = Reassembler::new(1024);
        let mut acc = BulkTransferAccumulator::new(payload.len() as u64);

        for frame in frames {
            dispatcher.dispatch(frame).unwrap();
        }

        // Drop dispatcher to drop its Sender clone so reassembly_rx sees closure.
        drop(dispatcher);

        let mut result = None;
        while let Some(chunk) = reassembly_rx.blocking_recv() {
            for r in reassembler.process(chunk).unwrap() {
                if let Some(complete) = acc.push(&r) {
                    result = Some(complete);
                }
            }
            if acc.is_complete() { break; }
        }

        assert_eq!(result.unwrap(), payload);
    }
}
