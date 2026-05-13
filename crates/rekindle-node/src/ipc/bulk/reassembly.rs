//! Ordered chunk reassembly with parallel Merkle digest verification.
//!
//! The parallel decryption rayon workers produce `DecryptedChunk` values
//! that may arrive out of order (different workers complete at different
//! times). Each chunk carries a pre-computed digest (SHA-256 or BLAKE3,
//! determined by session's `DigestAlgorithm`, computed on the rayon worker
//! while the plaintext is in L1 cache). The reassembler reorders chunks
//! by `chunk_index` and aggregates their per-chunk digests into a Merkle
//! root for verification.
//!
//! # Plaintext zeroization
//!
//! Both `DecryptedChunk::plaintext` and `ReassembledChunk::plaintext` are
//! `PooledBuf`. Plaintext is zeroized and returned to the receive pool
//! when the chunk is dropped — whether by the reassembler (buffered
//! out-of-order chunk evicted), by the application (after consumption),
//! or by panic unwind. Chunks held in the `BTreeMap` reorder buffer are
//! zeroized and returned to the pool when `reset()` clears it.
//!
//! # Reorder buffer
//!
//! A `BTreeMap<u64, DecryptedChunk>` holds out-of-order chunks. The
//! reassembler maintains a `next_expected` counter. When a chunk arrives
//! with `chunk_index == next_expected`, it is immediately delivered and
//! `next_expected` is incremented. Then the buffer is drained of any
//! consecutive chunks that follow.
//!
//! # Overflow protection
//!
//! The buffer has a maximum depth (`max_buffered`). If exceeded, the
//! transfer is declared failed. This prevents unbounded memory growth
//! from a misbehaving sender or extreme rayon scheduling variance.
//!
//! # Digest verification
//!
//! On `BulkFin`, the reassembler compares its Merkle root against
//! the sender-provided digest. Mismatch rejects the entire transfer.
//! The per-chunk digest is computed in parallel by rayon workers
//! (SHA-256 or BLAKE3, configurable); the aggregation here is sequential
//! but operates on 32-byte digests, not 64 KiB chunks — instantaneous.

use std::collections::BTreeMap;

use super::dispatcher::DecryptedChunk;
use super::verify::MerkleDigest;

/// A reassembled, verified chunk ready for delivery to the application.
///
/// `plaintext` is a `PooledBuf` — zeroized and returned to the receive
/// pool on drop. The application receives plaintext that will be
/// automatically zeroized when the `ReassembledChunk` is dropped, even
/// if the application forgets to zeroize it explicitly.
pub struct ReassembledChunk {
    /// Which bulk stream this chunk belongs to.
    pub stream_id: u8,
    /// Chunk ordering index (monotonically increasing, gap-free).
    pub chunk_index: u64,
    /// Verified plaintext bytes. Zeroized and returned to receive pool on drop.
    pub plaintext: super::pooled_buf::PooledBuf,
    /// True if this is the final chunk of the transfer.
    pub is_last: bool,
}

/// Reassembly errors.
#[derive(Debug)]
pub enum ReassemblyError {
    /// The out-of-order buffer exceeded the maximum allowed depth.
    BufferOverflow { buffered: usize, max: usize },
    /// The streaming SHA-256 digest does not match the sender's digest.
    DigestMismatch { stream_id: u8 },
    /// AEAD decryption failed for a chunk.
    DecryptFailed { stream_id: u8, chunk_index: u64 },
}

impl std::fmt::Display for ReassemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferOverflow { buffered, max } => {
                write!(f, "reassembly buffer overflow: {buffered} buffered, max {max}")
            }
            Self::DigestMismatch { stream_id } => {
                write!(f, "digest mismatch on stream {stream_id}")
            }
            Self::DecryptFailed { stream_id, chunk_index } => {
                write!(f, "AEAD decrypt failed on stream {stream_id} chunk {chunk_index}")
            }
        }
    }
}

impl std::error::Error for ReassemblyError {}

/// Ordered reassembly of decrypted chunks with parallel Merkle verification.
pub struct Reassembler {
    /// The next expected chunk index (starts at 0, increments by 1).
    next_expected: u64,
    /// Out-of-order buffer: chunks waiting for earlier chunks to arrive.
    buffer: BTreeMap<u64, DecryptedChunk>,
    /// Merkle digest aggregator — feeds per-chunk digests in order.
    digest: MerkleDigest,
    /// Maximum allowed out-of-order buffer depth.
    max_buffered: usize,
    /// Digest algorithm for this reassembler.
    algorithm: super::verify::DigestAlgorithm,
}

impl Reassembler {
    /// Create a new reassembler with default algorithm (BLAKE3).
    pub fn new(max_buffered: usize) -> Self {
        Self::with_algorithm(max_buffered, super::verify::DigestAlgorithm::default())
    }

    /// Create a reassembler with a specific digest algorithm.
    pub fn with_algorithm(max_buffered: usize, algorithm: super::verify::DigestAlgorithm) -> Self {
        Self {
            next_expected: 0,
            buffer: BTreeMap::new(),
            digest: MerkleDigest::with_algorithm(algorithm),
            max_buffered,
            algorithm,
        }
    }

    /// Process a received decrypted chunk.
    ///
    /// Returns a `Vec` of in-order plaintext chunks ready for delivery.
    /// May return:
    /// - 0 chunks: the received chunk is out of order and buffered.
    /// - 1 chunk: the received chunk is the next expected one.
    /// - N chunks: the received chunk fills a gap, releasing N buffered
    ///   chunks in sequence.
    ///
    /// On `BulkFin`, verifies the streaming digest against the sender's
    /// digest. Returns `Err(DigestMismatch)` on failure.
    pub fn process(
        &mut self,
        chunk: DecryptedChunk,
    ) -> Result<Vec<ReassembledChunk>, ReassemblyError> {
        let idx = chunk.chunk_index;

        // Stale/duplicate chunk — silently drop.
        if idx < self.next_expected {
            tracing::debug!(
                chunk_index = idx,
                expected = self.next_expected,
                "stale chunk dropped"
            );
            return Ok(Vec::new());
        }

        // Fail-fast on decrypt failure.
        if chunk.decrypt_failed {
            return Err(ReassemblyError::DecryptFailed {
                stream_id: chunk.stream_id,
                chunk_index: chunk.chunk_index,
            });
        }

        // Buffer the chunk.
        self.buffer.insert(idx, chunk);

        // Overflow check.
        if self.buffer.len() > self.max_buffered {
            return Err(ReassemblyError::BufferOverflow {
                buffered: self.buffer.len(),
                max: self.max_buffered,
            });
        }

        // Drain consecutive chunks starting from next_expected.
        let mut output = Vec::new();

        while let Some(chunk) = self.buffer.remove(&self.next_expected) {
            // Feed the per-chunk digest into the Merkle aggregator.
            // The chunk_digest was computed on the rayon worker in parallel.
            self.digest.feed_chunk_digest(&chunk.chunk_digest);

            let is_last = chunk.is_last;
            let fin_digest = chunk.fin_digest;
            let stream_id = chunk.stream_id;

            output.push(ReassembledChunk {
                stream_id: chunk.stream_id,
                chunk_index: chunk.chunk_index,
                plaintext: chunk.plaintext, // PooledBuf moved, returned to recv pool on drop
                is_last,
            });

            self.next_expected += 1;

            // On the final chunk, verify the digest.
            if is_last {
                if let Some(expected) = fin_digest {
                    let computed = std::mem::take(&mut self.digest);
                    if !computed.verify(&expected) {
                        return Err(ReassemblyError::DigestMismatch { stream_id });
                    }
                    tracing::debug!(
                        stream_id,
                        chunks = self.next_expected,
                        "bulk transfer digest verified"
                    );
                }
                // Do NOT reset next_expected to 0. The nonce counter
                // is per-session and never resets. The next transfer's
                // first chunk will have nonce = self.next_expected
                // (the value after the last increment). The digest is
                // reset for the new transfer's verification.
            }
        }

        Ok(output)
    }

    /// Reset for a new transfer after cancellation.
    ///
    /// Clears the buffer, resets the digest, sets next_expected to the
    /// given nonce value (the session's current nonce counter position).
    pub fn reset(&mut self, next_nonce: u64) {
        self.buffer.clear();
        self.digest = MerkleDigest::with_algorithm(self.algorithm);
        self.next_expected = next_nonce;
    }

    /// Number of chunks currently buffered out of order.
    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::verify::{DigestAlgorithm, digest_oneshot, merkle_root_with_algorithm};
    use super::super::pooled_buf::PooledBuf;
    use super::super::pool::BufferPool;

    fn test_pool() -> std::sync::Arc<BufferPool> {
        BufferPool::new()
    }

    fn make_chunk_with(algo: DigestAlgorithm, idx: u64, data: &[u8], is_last: bool) -> DecryptedChunk {
        DecryptedChunk {
            stream_id: 0,
            chunk_index: idx,
            plaintext: PooledBuf::new(data.to_vec(), test_pool()),
            is_last,
            fin_digest: None,
            chunk_digest: digest_oneshot(algo, data),
            decrypt_failed: false,
        }
    }

    fn make_fin_with(algo: DigestAlgorithm, idx: u64, data: &[u8], digest: [u8; 32]) -> DecryptedChunk {
        DecryptedChunk {
            stream_id: 0,
            chunk_index: idx,
            plaintext: PooledBuf::new(data.to_vec(), test_pool()),
            is_last: true,
            fin_digest: Some(digest),
            chunk_digest: digest_oneshot(algo, data),
            decrypt_failed: false,
        }
    }

    /// Run the full reassembly test suite for a given algorithm.
    fn run_suite(algo: DigestAlgorithm) {
        // ── In-order delivery ────────────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            let out = r.process(make_chunk_with(algo, 0, b"hello", false)).unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(&*out[0].plaintext, b"hello");
            let out = r.process(make_chunk_with(algo, 1, b"world", false)).unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(&*out[0].plaintext, b"world");
        }

        // ── Out-of-order buffering and release ───────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            let out = r.process(make_chunk_with(algo, 1, b"world", false)).unwrap();
            assert_eq!(out.len(), 0);
            assert_eq!(r.buffered_count(), 1);
            let out = r.process(make_chunk_with(algo, 0, b"hello", false)).unwrap();
            assert_eq!(out.len(), 2);
            assert_eq!(&*out[0].plaintext, b"hello");
            assert_eq!(&*out[1].plaintext, b"world");
            assert_eq!(r.buffered_count(), 0);
        }

        // ── Buffer overflow ──────────────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(2, algo);
            r.process(make_chunk_with(algo, 1, b"a", false)).unwrap();
            r.process(make_chunk_with(algo, 2, b"b", false)).unwrap();
            let result = r.process(make_chunk_with(algo, 3, b"c", false));
            assert!(matches!(result, Err(ReassemblyError::BufferOverflow { buffered: 3, max: 2 })));
        }

        // ── Duplicate chunk dropped ──────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            r.process(make_chunk_with(algo, 0, b"hello", false)).unwrap();
            let out = r.process(make_chunk_with(algo, 0, b"hello", false)).unwrap();
            assert_eq!(out.len(), 0);
        }

        // ── Fin with correct digest ─────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            let data = b"hello world";
            let root = merkle_root_with_algorithm(&[&data[..]], algo);
            let out = r.process(make_fin_with(algo, 0, data, root)).unwrap();
            assert_eq!(out.len(), 1);
            assert!(out[0].is_last);
        }

        // ── Fin with wrong digest ────────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            let result = r.process(make_fin_with(algo, 0, b"hello world", [0xFF; 32]));
            assert!(matches!(result, Err(ReassemblyError::DigestMismatch { .. })));
        }

        // ── Multi-chunk digest verification ──────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            let chunk0 = b"hello ";
            let chunk1 = b"world";
            let root = merkle_root_with_algorithm(&[&chunk0[..], &chunk1[..]], algo);
            let out = r.process(make_chunk_with(algo, 0, chunk0, false)).unwrap();
            assert_eq!(out.len(), 1);
            let out = r.process(make_fin_with(algo, 1, chunk1, root)).unwrap();
            assert_eq!(out.len(), 1);
            assert!(out[0].is_last);
        }

        // ── Reverse order three chunks ───────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            assert_eq!(r.process(make_chunk_with(algo, 2, b"c", false)).unwrap().len(), 0);
            assert_eq!(r.process(make_chunk_with(algo, 1, b"b", false)).unwrap().len(), 0);
            let out = r.process(make_chunk_with(algo, 0, b"a", false)).unwrap();
            assert_eq!(out.len(), 3);
            assert_eq!(&*out[0].plaintext, b"a");
            assert_eq!(&*out[1].plaintext, b"b");
            assert_eq!(&*out[2].plaintext, b"c");
        }

        // ── Reset clears state ───────────────────────────────────
        {
            let mut r = Reassembler::with_algorithm(1024, algo);
            r.process(make_chunk_with(algo, 1, b"a", false)).unwrap();
            r.process(make_chunk_with(algo, 2, b"b", false)).unwrap();
            assert_eq!(r.buffered_count(), 2);
            r.reset(100);
            assert_eq!(r.buffered_count(), 0);
            let out = r.process(make_chunk_with(algo, 1, b"a", false)).unwrap();
            assert_eq!(out.len(), 0); // stale
            let out = r.process(make_chunk_with(algo, 100, b"new", false)).unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(&*out[0].plaintext, b"new");
        }
    }

    #[test]
    fn full_suite_blake3() {
        run_suite(DigestAlgorithm::Blake3);
    }

    #[test]
    fn full_suite_sha256() {
        run_suite(DigestAlgorithm::Sha256);
    }

    #[test]
    fn decrypt_failure_propagates() {
        let mut r = Reassembler::new(1024);
        let failed = DecryptedChunk {
            stream_id: 0,
            chunk_index: 0,
            plaintext: PooledBuf::new(Vec::new(), test_pool()),
            is_last: false,
            fin_digest: None,
            chunk_digest: [0u8; 32],
            decrypt_failed: true,
        };
        assert!(matches!(
            r.process(failed),
            Err(ReassemblyError::DecryptFailed { stream_id: 0, chunk_index: 0 })
        ));
    }
}
