//! Digest verification for bulk transfer integrity.
//!
//! Three digest algorithms, two verification modes:
//!
//! **Algorithms:**
//! - SHA-256 (~420 MiB/s without SHA-NI, ~1.6 GiB/s with SHA-NI)
//! - BLAKE3 (~3.8 GiB/s with AVX2, ~15 GiB/s with update_rayon)
//!
//! **Modes:**
//! - **`StreamingDigest`**: Linear hash over concatenated plaintext.
//!   Single-threaded. Produces OCI-compatible `sha256:hex` digests.
//! - **`MerkleDigest`**: Parallel per-chunk hash aggregated into
//!   `hash(chunk_digest_0 || ... || chunk_digest_N)`. Per-chunk hash
//!   runs on rayon workers. Default for bulk transfers.
//!
//! **Default:** BLAKE3 Merkle (`blake3-merkle:hex`). 4 cores × 3.8 GiB/s
//! = 15.2 GiB/s aggregate — exceeds any wire speed.
//!
//! **OCI compatibility:** OCI registries require `sha256:hex` over raw
//! blob bytes. Use `StreamingDigest` with `DigestAlgorithm::Sha256` for
//! OCI push. For all other transfers, BLAKE3 Merkle is the default.

use aws_lc_rs::digest;

// ── Algorithm Selection ─────────────────────────────────────────────

/// Digest algorithm for bulk transfer verification.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DigestAlgorithm {
    /// SHA-256 — OCI-compatible, ~420 MiB/s without SHA-NI.
    Sha256,
    /// BLAKE3 — ~3.8 GiB/s with AVX2. Default for node-to-node transfers.
    #[default]
    Blake3,
}

// ── One-Shot Digests ────────────────────────────────────────────────

/// Compute a one-shot SHA-256 digest.
///
/// Delegates to `rekindle_hash::single::sha256_oneshot` which uses
/// aws-lc-rs `digest::digest(&SHA256, data)` internally.
pub fn sha256_oneshot(data: &[u8]) -> [u8; 32] {
    rekindle_hash::single::sha256_oneshot(data)
}

/// Compute a one-shot BLAKE3 digest.
pub fn blake3_oneshot(data: &[u8]) -> [u8; 32] {
    rekindle_hash::single::blake3_oneshot(data)
}

/// Compute a one-shot digest using the specified algorithm.
pub fn digest_oneshot(algorithm: DigestAlgorithm, data: &[u8]) -> [u8; 32] {
    match algorithm {
        DigestAlgorithm::Sha256 => sha256_oneshot(data),
        DigestAlgorithm::Blake3 => blake3_oneshot(data),
    }
}

// ── Streaming Digest (linear, single-threaded) ──────────────────────

/// Linear streaming digest. Single-threaded, order-dependent.
/// Use for OCI blob verification where the spec requires sha256 over raw bytes.
pub struct StreamingDigest {
    ctx: digest::Context,
}

impl StreamingDigest {
    pub fn new() -> Self {
        Self { ctx: digest::Context::new(&digest::SHA256) }
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
    fn default() -> Self { Self::new() }
}

// ── Merkle Digest (parallel, algorithm-configurable) ────────────────

/// Parallel Merkle digest. Per-chunk hashes computed on rayon workers,
/// aggregated sequentially on 32-byte digests.
pub struct MerkleDigest {
    aggregator: MerkleAggregator,
    chunk_count: u64,
    algorithm: DigestAlgorithm,
}

enum MerkleAggregator {
    Sha256(digest::Context),
    Blake3(Box<blake3::Hasher>),
}

impl MerkleDigest {
    pub fn new() -> Self {
        Self::with_algorithm(DigestAlgorithm::default())
    }

    pub fn with_algorithm(algorithm: DigestAlgorithm) -> Self {
        let aggregator = match algorithm {
            DigestAlgorithm::Sha256 => MerkleAggregator::Sha256(digest::Context::new(&digest::SHA256)),
            DigestAlgorithm::Blake3 => MerkleAggregator::Blake3(Box::new(blake3::Hasher::new())),
        };
        Self { aggregator, chunk_count: 0, algorithm }
    }

    pub fn feed_chunk_digest(&mut self, chunk_digest: &[u8; 32]) {
        match &mut self.aggregator {
            MerkleAggregator::Sha256(ctx) => ctx.update(chunk_digest),
            MerkleAggregator::Blake3(hasher) => { hasher.update(chunk_digest); }
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
            MerkleAggregator::Blake3(hasher) => *hasher.finalize().as_bytes(),
        }
    }

    pub fn verify(self, expected: &[u8; 32]) -> bool {
        let computed = self.finalize();
        aws_lc_rs::constant_time::verify_slices_are_equal(&computed, expected).is_ok()
    }

    pub fn chunk_count(&self) -> u64 { self.chunk_count }
    pub fn algorithm(&self) -> DigestAlgorithm { self.algorithm }
}

impl Default for MerkleDigest {
    fn default() -> Self { Self::new() }
}

// ── Convenience Functions ───────────────────────────────────────────

/// Compute Merkle root using default algorithm (BLAKE3).
pub fn merkle_root(chunks: &[&[u8]]) -> [u8; 32] {
    merkle_root_with_algorithm(chunks, DigestAlgorithm::default())
}

/// Compute Merkle root using a specific algorithm.
///
/// For SHA-256 with 4+ chunks, uses `rekindle_hash::sha256_parallel`
/// which dispatches to ISA-L multi-buffer AVX2 (8-way) when the
/// `sha256-mb` feature is enabled. Falls back to sequential oneshot
/// otherwise. BLAKE3 is always sequential here (it has internal
/// 8-way parallelism already).
pub fn merkle_root_with_algorithm(chunks: &[&[u8]], algorithm: DigestAlgorithm) -> [u8; 32] {
    let mut merkle = MerkleDigest::with_algorithm(algorithm);

    match algorithm {
        DigestAlgorithm::Sha256 => {
            // Use rekindle_hash::sha256_parallel which automatically
            // dispatches to ISA-L multi-buffer AVX2 when sha256-mb is
            // enabled and chunk count >= 4. Falls back to sequential
            // single-buffer SHA-256 otherwise.
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

// ── Digest String Verification ──────────────────────────────────────

/// Parse a digest string and compare against a computed 32-byte hash.
///
/// Supported prefixes: `sha256:`, `sha256-merkle:`, `blake3:`, `blake3-merkle:`
pub fn verify_oci_digest(
    computed: &[u8; 32],
    expected_digest: &str,
) -> Result<(), DigestMismatch> {
    let (prefix, expected_hex) = if let Some(hex) = expected_digest.strip_prefix("blake3-merkle:") {
        ("blake3-merkle:", hex)
    } else if let Some(hex) = expected_digest.strip_prefix("blake3:") {
        ("blake3:", hex)
    } else if let Some(hex) = expected_digest.strip_prefix("sha256-merkle:") {
        ("sha256-merkle:", hex)
    } else if let Some(hex) = expected_digest.strip_prefix("sha256:") {
        ("sha256:", hex)
    } else {
        return Err(DigestMismatch {
            expected: expected_digest.to_string(),
            actual: "unsupported digest algorithm".to_string(),
        });
    };

    let computed_hex = hex::encode(computed);
    if computed_hex == expected_hex {
        Ok(())
    } else {
        Err(DigestMismatch {
            expected: expected_digest.to_string(),
            actual: format!("{prefix}{computed_hex}"),
        })
    }
}

/// Digest verification failure.
#[derive(Debug, Clone)]
pub struct DigestMismatch {
    pub expected: String,
    pub actual: String,
}

impl std::fmt::Display for DigestMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "digest mismatch: expected {}, got {}", self.expected, self.actual)
    }
}

impl std::error::Error for DigestMismatch {}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SHA-256 streaming tests ─────────────────────────────────

    #[test]
    fn streaming_matches_oneshot() {
        let data = vec![0xABu8; 65519 * 10];
        let expected = sha256_oneshot(&data);
        let mut sd = StreamingDigest::new();
        for chunk in data.chunks(65519) { sd.update(chunk); }
        assert!(sd.verify(&expected));
    }

    #[test]
    fn empty_input() {
        assert_eq!(StreamingDigest::new().finalize(), sha256_oneshot(&[]));
    }

    #[test]
    fn single_byte_chunks() {
        let data = b"hello world";
        let expected = sha256_oneshot(data);
        let mut sd = StreamingDigest::new();
        for &b in data.iter() { sd.update(&[b]); }
        assert!(sd.verify(&expected));
    }

    // ── One-shot dispatch ───────────────────────────────────────

    #[test]
    fn digest_oneshot_dispatches() {
        let data = b"hello world";
        assert_eq!(digest_oneshot(DigestAlgorithm::Sha256, data), sha256_oneshot(data));
        assert_eq!(digest_oneshot(DigestAlgorithm::Blake3, data), blake3_oneshot(data));
    }

    #[test]
    fn blake3_differs_from_sha256() {
        assert_ne!(sha256_oneshot(b"test"), blake3_oneshot(b"test"));
    }

    // ── SHA-256 Merkle tests ────────────────────────────────────

    #[test]
    fn merkle_sha256_single() {
        let data = b"hello world";
        let cd = sha256_oneshot(data);
        let mut m = MerkleDigest::with_algorithm(DigestAlgorithm::Sha256);
        m.feed_chunk_digest(&cd);
        assert_eq!(m.finalize(), sha256_oneshot(&cd));
    }

    #[test]
    fn merkle_sha256_multiple() {
        let c0 = vec![0xAAu8; 65519];
        let c1 = vec![0xBBu8; 65519];
        let d0 = sha256_oneshot(&c0);
        let d1 = sha256_oneshot(&c1);
        let mut m = MerkleDigest::with_algorithm(DigestAlgorithm::Sha256);
        m.feed_chunk_digest(&d0);
        m.feed_chunk_digest(&d1);
        let mut concat = Vec::new();
        concat.extend_from_slice(&d0);
        concat.extend_from_slice(&d1);
        assert_eq!(m.finalize(), sha256_oneshot(&concat));
    }

    // ── BLAKE3 Merkle tests ─────────────────────────────────────

    #[test]
    fn merkle_blake3_single() {
        let data = b"hello world";
        let cd = blake3_oneshot(data);
        let mut m = MerkleDigest::with_algorithm(DigestAlgorithm::Blake3);
        m.feed_chunk_digest(&cd);
        assert_eq!(m.finalize(), blake3_oneshot(&cd));
    }

    #[test]
    fn merkle_blake3_multiple() {
        let c0 = vec![0xAAu8; 65519];
        let c1 = vec![0xBBu8; 65519];
        let d0 = blake3_oneshot(&c0);
        let d1 = blake3_oneshot(&c1);
        let mut m = MerkleDigest::with_algorithm(DigestAlgorithm::Blake3);
        m.feed_chunk_digest(&d0);
        m.feed_chunk_digest(&d1);
        let mut concat = Vec::new();
        concat.extend_from_slice(&d0);
        concat.extend_from_slice(&d1);
        assert_eq!(m.finalize(), blake3_oneshot(&concat));
    }

    #[test]
    fn merkle_sha256_and_blake3_differ() {
        let chunks = vec![vec![0xAAu8; 1024], vec![0xBBu8; 1024]];
        let refs: Vec<&[u8]> = chunks.iter().map(|c| c.as_slice()).collect();
        assert_ne!(
            merkle_root_with_algorithm(&refs, DigestAlgorithm::Sha256),
            merkle_root_with_algorithm(&refs, DigestAlgorithm::Blake3),
        );
    }

    // ── merkle_root convenience ─────────────────────────────────

    #[test]
    fn merkle_root_default_is_blake3() {
        let c0 = vec![0xAAu8; 1024];
        let c1 = vec![0xBBu8; 1024];
        let root = merkle_root(&[&c0, &c1]);
        let blake3_root = merkle_root_with_algorithm(&[&c0, &c1], DigestAlgorithm::Blake3);
        assert_eq!(root, blake3_root);
    }

    #[test]
    fn merkle_root_with_algorithm_roundtrip() {
        for algo in [DigestAlgorithm::Sha256, DigestAlgorithm::Blake3] {
            let c0 = vec![0xAAu8; 1024];
            let c1 = vec![0xBBu8; 1024];
            let root = merkle_root_with_algorithm(&[&c0, &c1], algo);
            let mut m = MerkleDigest::with_algorithm(algo);
            m.feed_chunk_digest(&digest_oneshot(algo, &c0));
            m.feed_chunk_digest(&digest_oneshot(algo, &c1));
            assert_eq!(root, m.finalize());
        }
    }

    // ── Digest string verification ──────────────────────────────

    #[test]
    fn verify_sha256_prefix() {
        let d = sha256_oneshot(b"test");
        assert!(verify_oci_digest(&d, &format!("sha256:{}", hex::encode(d))).is_ok());
    }

    #[test]
    fn verify_sha256_merkle_prefix() {
        let d = sha256_oneshot(b"test");
        assert!(verify_oci_digest(&d, &format!("sha256-merkle:{}", hex::encode(d))).is_ok());
    }

    #[test]
    fn verify_blake3_prefix() {
        let d = blake3_oneshot(b"test");
        assert!(verify_oci_digest(&d, &format!("blake3:{}", hex::encode(d))).is_ok());
    }

    #[test]
    fn verify_blake3_merkle_prefix() {
        let d = blake3_oneshot(b"test");
        assert!(verify_oci_digest(&d, &format!("blake3-merkle:{}", hex::encode(d))).is_ok());
    }

    #[test]
    fn verify_unsupported_prefix() {
        assert!(verify_oci_digest(&[0u8; 32], "sha512:aaaa").is_err());
    }

    #[test]
    fn verify_mismatch() {
        let d = [0u8; 32];
        let result = verify_oci_digest(&d, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(result.is_err());
    }

    // ── Chunk count ─────────────────────────────────────────────

    #[test]
    fn merkle_chunk_count() {
        let mut m = MerkleDigest::new();
        assert_eq!(m.chunk_count(), 0);
        m.feed_chunk_digest(&[0u8; 32]);
        assert_eq!(m.chunk_count(), 1);
    }

    #[test]
    fn merkle_algorithm_accessor() {
        assert_eq!(MerkleDigest::new().algorithm(), DigestAlgorithm::Blake3);
        assert_eq!(MerkleDigest::with_algorithm(DigestAlgorithm::Sha256).algorithm(), DigestAlgorithm::Sha256);
    }
}
