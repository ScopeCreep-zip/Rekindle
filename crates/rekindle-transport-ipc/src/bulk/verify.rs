//! Digest verification for bulk transfer integrity.
//!
//! Two algorithms, two modes:
//! - SHA-256: OCI-compatible `sha256:hex` digests
//! - BLAKE3: ~8x faster, default for node-to-node transfers
//!
//! StreamingDigest: linear hash (single-threaded, OCI compat)
//! MerkleDigest: parallel per-chunk hash aggregation (default)

use aws_lc_rs::digest;
use serde::{Deserialize, Serialize};

/// Digest algorithm selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigestAlgorithm {
    Sha256,
    #[default]
    Blake3,
}

// ---- One-shot digests ----

pub fn sha256_oneshot(data: &[u8]) -> [u8; 32] {
    rekindle_hash::single::sha256_oneshot(data)
}

pub fn blake3_oneshot(data: &[u8]) -> [u8; 32] {
    rekindle_hash::single::blake3_oneshot(data)
}

pub fn digest_oneshot(algo: DigestAlgorithm, data: &[u8]) -> [u8; 32] {
    match algo {
        DigestAlgorithm::Sha256 => sha256_oneshot(data),
        DigestAlgorithm::Blake3 => blake3_oneshot(data),
    }
}

// ---- Streaming digest (linear, single-threaded, OCI compat) ----

pub struct StreamingDigest {
    ctx: digest::Context,
}

impl StreamingDigest {
    pub fn new() -> Self {
        Self {
            ctx: digest::Context::new(&digest::SHA256),
        }
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.ctx.update(chunk);
    }

    pub fn finalize(self) -> [u8; 32] {
        let d = self.ctx.finish();
        let mut out = [0u8; 32];
        out.copy_from_slice(d.as_ref());
        out
    }

    pub fn verify(self, expected: &[u8; 32]) -> bool {
        let computed = self.finalize();
        aws_lc_rs::constant_time::verify_slices_are_equal(&computed, expected).is_ok()
    }
}

impl Default for StreamingDigest {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Merkle digest (parallel per-chunk, configurable algorithm) ----

enum MerkleAggregator {
    Sha256(digest::Context),
    Blake3(Box<blake3::Hasher>),
}

pub struct MerkleDigest {
    aggregator: MerkleAggregator,
    chunk_count: u64,
    algorithm: DigestAlgorithm,
}

impl MerkleDigest {
    pub fn new() -> Self {
        Self::with_algorithm(DigestAlgorithm::default())
    }

    pub fn with_algorithm(algo: DigestAlgorithm) -> Self {
        let aggregator = match algo {
            DigestAlgorithm::Sha256 => {
                MerkleAggregator::Sha256(digest::Context::new(&digest::SHA256))
            }
            DigestAlgorithm::Blake3 => MerkleAggregator::Blake3(Box::new(blake3::Hasher::new())),
        };
        Self { aggregator, chunk_count: 0, algorithm: algo }
    }

    /// Feed a pre-computed per-chunk digest (32 bytes).
    pub fn feed_chunk_digest(&mut self, chunk_digest: &[u8; 32]) {
        match &mut self.aggregator {
            MerkleAggregator::Sha256(ctx) => ctx.update(chunk_digest),
            MerkleAggregator::Blake3(h) => {
                h.update(chunk_digest);
            }
        }
        self.chunk_count += 1;
    }

    pub fn finalize(self) -> [u8; 32] {
        match self.aggregator {
            MerkleAggregator::Sha256(ctx) => {
                let d = ctx.finish();
                let mut out = [0u8; 32];
                out.copy_from_slice(d.as_ref());
                out
            }
            MerkleAggregator::Blake3(h) => *h.finalize().as_bytes(),
        }
    }

    /// Verify against expected digest (constant-time comparison).
    pub fn verify(self, expected: &[u8; 32]) -> bool {
        let computed = self.finalize();
        aws_lc_rs::constant_time::verify_slices_are_equal(&computed, expected).is_ok()
    }

    pub fn chunk_count(&self) -> u64 {
        self.chunk_count
    }

    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }
}

impl Default for MerkleDigest {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Convenience: compute Merkle root ----

/// Compute Merkle root (default BLAKE3).
pub fn merkle_root(chunks: &[&[u8]]) -> [u8; 32] {
    merkle_root_with_algorithm(chunks, DigestAlgorithm::default())
}

/// Compute Merkle root with a specific algorithm.
///
/// For SHA-256 with 4+ chunks, uses rekindle_hash::sha256_parallel
/// (ISA-L multi-buffer AVX2 when sha256-mb feature is enabled).
pub fn merkle_root_with_algorithm(chunks: &[&[u8]], algo: DigestAlgorithm) -> [u8; 32] {
    let mut merkle = MerkleDigest::with_algorithm(algo);

    match algo {
        DigestAlgorithm::Sha256 => {
            let mut digests = vec![[0u8; 32]; chunks.len()];
            rekindle_hash::sha256_parallel(chunks, &mut digests);
            for d in &digests {
                merkle.feed_chunk_digest(d);
            }
        }
        DigestAlgorithm::Blake3 => {
            for chunk in chunks {
                merkle.feed_chunk_digest(&blake3_oneshot(chunk));
            }
        }
    }

    merkle.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_matches_oneshot() {
        let data = vec![0xABu8; 65519 * 3];
        let expected = sha256_oneshot(&data);
        let mut sd = StreamingDigest::new();
        for chunk in data.chunks(65519) {
            sd.update(chunk);
        }
        assert!(sd.verify(&expected));
    }

    #[test]
    fn merkle_blake3_single() {
        let data = b"hello world";
        let cd = blake3_oneshot(data);
        let mut m = MerkleDigest::new();
        m.feed_chunk_digest(&cd);
        assert_eq!(m.finalize(), blake3_oneshot(&cd));
    }

    #[test]
    fn merkle_root_default_is_blake3() {
        let c0 = vec![0xAAu8; 1024];
        let c1 = vec![0xBBu8; 1024];
        let root = merkle_root(&[&c0, &c1]);
        let blake3_root = merkle_root_with_algorithm(&[&c0, &c1], DigestAlgorithm::Blake3);
        assert_eq!(root, blake3_root);
    }

    #[test]
    fn sha256_and_blake3_differ() {
        let data = &[vec![0xAAu8; 1024]];
        let refs: Vec<&[u8]> = data.iter().map(|c| c.as_slice()).collect();
        assert_ne!(
            merkle_root_with_algorithm(&refs, DigestAlgorithm::Sha256),
            merkle_root_with_algorithm(&refs, DigestAlgorithm::Blake3),
        );
    }
}
