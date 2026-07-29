//! ReorderRing-based per-stream chunk reassembly with Merkle content hash.
//!
//! Zero per-item allocation in the steady state. The ReorderRing replaces
//! the prior BTreeMap implementation that allocated per out-of-order chunk
//! (the ~50 MB throughput knee).
//!
//! Merkle aggregator: BLAKE3(digest_0 || digest_1 || ... || digest_N).
//! Content hash verification compares the aggregated root against the sender's hash.

use rekindle_transport_buff::ReorderRing;

use crate::v4::io::lane_channels::PlaintextBuf;

/// A chunk payload with its pre-computed BLAKE3 digest.
/// The digest is computed on the rayon worker while data is L1-hot.
pub struct ChunkEntry {
    pub data: PlaintextBuf,
    pub digest: [u8; 32],
}

pub struct Reassembler {
    ring: ReorderRing<ChunkEntry>,
    delivered_bytes: u64,
    /// Merkle aggregator — feeds 32-byte per-chunk digests sequentially.
    /// The final hash is BLAKE3 over the concatenation of all chunk digests
    /// in delivery order. This matches the sender's computation.
    digest_aggregator: blake3::Hasher,
}

impl Reassembler {
    /// Create a reassembler with the given reorder window.
    /// `window` must be a power of two (ReorderRing requirement).
    pub fn new(window: usize) -> Self {
        Self {
            ring: ReorderRing::new(window),
            delivered_bytes: 0,
            digest_aggregator: blake3::Hasher::new(),
        }
    }

    /// Create a reassembler starting at an arbitrary chunk index.
    /// Used for resume-after-reconnect (BULK_RESUME).
    pub fn new_from_offset(window: usize, start_chunk: u32) -> Self {
        Self {
            ring: ReorderRing::new_with_base(window, start_chunk as u64),
            delivered_bytes: 0,
            digest_aggregator: blake3::Hasher::new(),
        }
    }

    /// Insert a chunk with its pre-computed digest.
    ///
    /// Returns delivered chunks as a contiguous prefix in chunk-index order.
    /// Zero per-item allocation: the ReorderRing publishes into a fixed-size
    /// slot array and drains without touching the allocator.
    pub fn insert_with_digest(
        &mut self,
        chunk_index: u32,
        data: PlaintextBuf,
        digest: [u8; 32],
    ) -> Vec<(u32, PlaintextBuf)> {
        let seq = chunk_index as u64;
        let entry = ChunkEntry { data, digest };

        // Fast path: in-order delivery when nothing is buffered.
        match self.ring.try_deliver_direct(seq, entry) {
            Ok(entry) => {
                let mut delivered = Vec::new();
                self.deliver(chunk_index, entry, &mut delivered);
                self.drain_into(&mut delivered);
                return delivered;
            }
            Err(entry) => {
                if self.ring.publish(seq, entry).is_err() {
                    return vec![];
                }
            }
        }

        let mut delivered = Vec::new();
        self.drain_into(&mut delivered);
        delivered
    }

    /// Drain the contiguous prefix from the ring into the output vec.
    fn drain_into(&mut self, out: &mut Vec<(u32, PlaintextBuf)>) {
        self.ring.drain_contiguous(|seq, entry| {
            let idx = seq as u32;
            tracing::debug!(
                chunk_index = idx,
                chunk_len = entry.data.len(),
                digest = %hex::encode(&entry.digest[..8]),
                "reassembler: delivering chunk digest to aggregator"
            );
            self.delivered_bytes += entry.data.len() as u64;
            self.digest_aggregator.update(&entry.digest);
            out.push((idx, entry.data));
        });
    }

    fn deliver(
        &mut self,
        idx: u32,
        entry: ChunkEntry,
        out: &mut Vec<(u32, PlaintextBuf)>,
    ) {
        tracing::debug!(
            chunk_index = idx,
            chunk_len = entry.data.len(),
            digest = %hex::encode(&entry.digest[..8]),
            "reassembler: delivering chunk digest to aggregator"
        );
        self.delivered_bytes += entry.data.len() as u64;
        self.digest_aggregator.update(&entry.digest);
        out.push((idx, entry.data));
    }

    pub fn next_expected(&self) -> u32 {
        self.ring.next_deliver() as u32
    }

    pub fn buffered_count(&self) -> usize {
        self.ring.stored_count()
    }

    pub fn total_bytes(&self) -> u64 {
        self.delivered_bytes
    }

    pub fn is_complete(&self, total_chunks: u32) -> bool {
        self.ring.next_deliver() >= total_chunks as u64 && !self.ring.has_buffered()
    }

    /// Verify the Merkle content hash against the sender's hash.
    /// All-zero hash skips verification (streaming generation, hash unknown).
    pub fn verify_content_hash(&self, expected: &[u8; 32]) -> Result<(), ContentHashError> {
        if expected == &[0u8; 32] {
            return Ok(());
        }
        let computed = *self.digest_aggregator.finalize().as_bytes();
        tracing::debug!(
            expected = %hex::encode(&expected[..8]),
            computed = %hex::encode(&computed[..8]),
            delivered_chunks = self.ring.next_deliver(),
            delivered_bytes = self.delivered_bytes,
            "reassembler: verify_content_hash"
        );
        if &computed == expected {
            Ok(())
        } else {
            tracing::error!(
                expected = %hex::encode(&expected[..8]),
                computed = %hex::encode(&computed[..8]),
                delivered_chunks = self.ring.next_deliver(),
                delivered_bytes = self.delivered_bytes,
                buffered = self.ring.stored_count(),
                "reassembler: CONTENT HASH MISMATCH"
            );
            Err(ContentHashError::Mismatch {
                expected: *expected,
                computed,
            })
        }
    }

    /// The content hash of all chunks delivered so far (partial Merkle root).
    pub fn partial_content_hash(&self) -> [u8; 32] {
        *self.digest_aggregator.finalize().as_bytes()
    }

    pub fn reset(&mut self, window: usize) {
        self.ring = ReorderRing::new(window);
        self.delivered_bytes = 0;
        self.digest_aggregator = blake3::Hasher::new();
    }
}

#[derive(Debug)]
pub enum ContentHashError {
    Mismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },
}
