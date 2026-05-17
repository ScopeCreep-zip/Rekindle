//! Ordered chunk reassembly with parallel Merkle digest verification.
//!
//! Rayon workers produce DecryptedChunks out of order. Each carries a
//! pre-computed digest. The reassembler reorders by chunk_seq and
//! aggregates digests into a Merkle root for verification.
//!
//! Each Reassembler instance handles ONE stream. For multi-stream
//! concurrent transfers, use `HashMap<u8, Reassembler>` keyed by stream_id.
//!
//! All plaintext is ZeroizingBuf — zeroized on drop, no pool.

use std::collections::BTreeMap;

use super::dispatcher::DecryptedChunk;
use super::pool::ZeroizingBuf;
use super::verify::{DigestAlgorithm, MerkleDigest};

/// A reassembled, verified chunk ready for delivery.
///
/// Carries the `MemoryReservation` from the original `DecryptedChunk`.
/// The reservation is released when this chunk is dropped (after
/// `on_bulk_chunk` returns). This is the RAII lifecycle endpoint.
pub struct ReassembledChunk {
    pub stream_id: u8,
    pub chunk_seq: u32,
    /// Verified plaintext. Zeroized on drop (full capacity, volatile writes).
    /// No pool reference — recv-path memory is returned to the OS allocator.
    pub plaintext: ZeroizingBuf,
    pub is_last: bool,
    /// RAII global memory reservation from GlobalMemoryGuard.
    pub reservation: Option<crate::backpressure::MemoryReservation>,
    /// RAII per-connection memory reservation.
    pub per_conn_reservation: Option<crate::backpressure::MemoryReservation>,
}

/// Reassembly errors.
#[derive(Debug)]
pub enum ReassemblyError {
    BufferOverflow { buffered: usize, max: usize },
    DigestMismatch { stream_id: u8 },
    DecryptFailed { stream_id: u8, chunk_seq: u32 },
}

impl std::fmt::Display for ReassemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferOverflow { buffered, max } => {
                write!(f, "reassembly overflow: {buffered}/{max}")
            }
            Self::DigestMismatch { stream_id } => {
                write!(f, "digest mismatch on stream {stream_id}")
            }
            Self::DecryptFailed { stream_id, chunk_seq } => {
                write!(f, "decrypt failed: stream {stream_id} chunk {chunk_seq}")
            }
        }
    }
}

impl std::error::Error for ReassemblyError {}

/// Ordered reassembly of decrypted chunks with Merkle verification.
///
/// Uses `chunk_seq` (per-stream, 0-indexed per transfer) for ordering.
/// NOT the AEAD nonce — those are separate concerns.
pub struct Reassembler {
    next_expected: u32,
    buffer: BTreeMap<u32, DecryptedChunk>,
    digest: MerkleDigest,
    max_buffered: usize,
    algorithm: DigestAlgorithm,
}

impl Reassembler {
    pub fn new(max_buffered: usize) -> Self {
        Self::with_algorithm(max_buffered, DigestAlgorithm::default())
    }

    pub fn with_algorithm(max_buffered: usize, algo: DigestAlgorithm) -> Self {
        Self {
            next_expected: 0,
            buffer: BTreeMap::new(),
            digest: MerkleDigest::with_algorithm(algo),
            max_buffered,
            algorithm: algo,
        }
    }

    /// Process a decrypted chunk. Returns in-order chunks ready for delivery.
    ///
    /// Uses `chunk.chunk_seq` (per-stream, 0-indexed) for ordering.
    /// May return 0 (buffered), 1 (next expected), or N (filled a gap).
    pub fn process(
        &mut self,
        chunk: DecryptedChunk,
    ) -> Result<Vec<ReassembledChunk>, ReassemblyError> {
        let seq = chunk.chunk_seq;

        // Stale/duplicate — silently drop.
        if seq < self.next_expected {
            return Ok(Vec::new());
        }

        // Fail-fast on decrypt failure.
        if chunk.decrypt_failed {
            return Err(ReassemblyError::DecryptFailed {
                stream_id: chunk.stream_id,
                chunk_seq: chunk.chunk_seq,
            });
        }

        self.buffer.insert(seq, chunk);

        if self.buffer.len() > self.max_buffered {
            return Err(ReassemblyError::BufferOverflow {
                buffered: self.buffer.len(),
                max: self.max_buffered,
            });
        }

        // Drain consecutive chunks starting from next_expected.
        let mut output = Vec::new();

        while let Some(chunk) = self.buffer.remove(&self.next_expected) {
            let is_last = chunk.is_last;
            let fin_digest = chunk.fin_digest;
            let stream_id = chunk.stream_id;

            if is_last {
                // BulkFin: carries only the merkle root, no data payload.
                // Do NOT feed its chunk_digest into the merkle tree —
                // only data chunks contribute leaves.
                output.push(ReassembledChunk {
                    stream_id,
                    chunk_seq: chunk.chunk_seq,
                    plaintext: chunk.plaintext,
                    is_last: true,
                    reservation: chunk.reservation,
                    per_conn_reservation: chunk.per_conn_reservation,
                });

                self.next_expected += 1;

                // Verify the accumulated merkle root against the fin digest.
                if let Some(expected) = fin_digest {
                    let computed = std::mem::replace(
                        &mut self.digest,
                        MerkleDigest::with_algorithm(self.algorithm),
                    );
                    if !computed.verify(&expected) {
                        return Err(ReassemblyError::DigestMismatch { stream_id });
                    }
                }
            } else {
                // BulkData: feed chunk digest into merkle tree.
                self.digest.feed_chunk_digest(&chunk.chunk_digest);

                output.push(ReassembledChunk {
                    stream_id,
                    chunk_seq: chunk.chunk_seq,
                    plaintext: chunk.plaintext,
                    is_last: false,
                    reservation: chunk.reservation,
                    per_conn_reservation: chunk.per_conn_reservation,
                });

                self.next_expected += 1;
            }
        }

        Ok(output)
    }

    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::verify::{digest_oneshot, merkle_root_with_algorithm};

    fn chunk(algo: DigestAlgorithm, seq: u32, data: &[u8], is_last: bool) -> DecryptedChunk {
        DecryptedChunk {
            stream_id: 0,
            chunk_seq: seq,
            plaintext: ZeroizingBuf::new(data.to_vec()),
            is_last,
            fin_digest: None,
            chunk_digest: digest_oneshot(algo, data),
            decrypt_failed: false,
            reservation: None,
            per_conn_reservation: None,
        }
    }

    /// Create a BulkFin chunk: empty plaintext, carries only the merkle root.
    fn fin_chunk(seq: u32, merkle_root: [u8; 32]) -> DecryptedChunk {
        DecryptedChunk {
            stream_id: 0,
            chunk_seq: seq,
            plaintext: ZeroizingBuf::new(Vec::new()),
            is_last: true,
            fin_digest: Some(merkle_root),
            chunk_digest: [0u8; 32],
            decrypt_failed: false,
            reservation: None,
            per_conn_reservation: None,
        }
    }

    #[test]
    fn in_order_delivery() {
        let mut r = Reassembler::new(1024);
        let out = r.process(chunk(DigestAlgorithm::Blake3, 0, b"a", false)).unwrap();
        assert_eq!(out.len(), 1);
        let out = r.process(chunk(DigestAlgorithm::Blake3, 1, b"b", false)).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn out_of_order_buffering() {
        let mut r = Reassembler::new(1024);
        assert_eq!(r.process(chunk(DigestAlgorithm::Blake3, 1, b"b", false)).unwrap().len(), 0);
        let out = r.process(chunk(DigestAlgorithm::Blake3, 0, b"a", false)).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(&*out[0].plaintext, b"a");
        assert_eq!(&*out[1].plaintext, b"b");
    }

    /// Correct protocol: data chunk(s) followed by a separate fin chunk
    /// carrying only the merkle root. The fin's empty plaintext is not
    /// included in the merkle tree.
    #[test]
    fn fin_with_correct_digest() {
        let algo = DigestAlgorithm::Blake3;
        let mut r = Reassembler::with_algorithm(1024, algo);
        let data = b"hello world";
        let root = merkle_root_with_algorithm(&[&data[..]], algo);

        // Data chunk first.
        let out = r.process(chunk(algo, 0, data, false)).unwrap();
        assert_eq!(out.len(), 1);
        assert!(!out[0].is_last);
        assert_eq!(&*out[0].plaintext, data);

        // Fin chunk with merkle root, empty plaintext.
        let out = r.process(fin_chunk(1, root)).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].is_last);
        assert!(out[0].plaintext.is_empty());
    }

    /// Multiple data chunks then fin — exercises the full pipeline.
    #[test]
    fn multi_chunk_then_fin() {
        let algo = DigestAlgorithm::Blake3;
        let mut r = Reassembler::with_algorithm(1024, algo);
        let c0 = b"chunk zero data";
        let c1 = b"chunk one data!";
        let c2 = b"chunk two data!";
        let root = merkle_root_with_algorithm(&[&c0[..], &c1[..], &c2[..]], algo);

        r.process(chunk(algo, 0, c0, false)).unwrap();
        r.process(chunk(algo, 1, c1, false)).unwrap();
        r.process(chunk(algo, 2, c2, false)).unwrap();
        let out = r.process(fin_chunk(3, root)).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].is_last);
    }

    #[test]
    fn fin_with_wrong_digest() {
        let algo = DigestAlgorithm::Blake3;
        let mut r = Reassembler::with_algorithm(1024, algo);

        // Send a data chunk, then a fin with wrong root.
        r.process(chunk(algo, 0, b"hello", false)).unwrap();
        let result = r.process(fin_chunk(1, [0xFF; 32]));
        assert!(matches!(result, Err(ReassemblyError::DigestMismatch { .. })));
    }

    #[test]
    fn buffer_overflow() {
        let mut r = Reassembler::new(2);
        r.process(chunk(DigestAlgorithm::Blake3, 1, b"a", false)).unwrap();
        r.process(chunk(DigestAlgorithm::Blake3, 2, b"b", false)).unwrap();
        let result = r.process(chunk(DigestAlgorithm::Blake3, 3, b"c", false));
        assert!(matches!(result, Err(ReassemblyError::BufferOverflow { .. })));
    }
}
