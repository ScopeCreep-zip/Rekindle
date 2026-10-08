//! Multi-buffer hash implementations for Rekindle Merkle digest.
//!
//! The `sha256-mb` feature enables multi-buffer SHA-256 through ISA-L
//! crypto's scheduler kernels: on x86_64, SSE and AVX (4 lanes), AVX2 (8),
//! AVX-512 (16) and SHA-NI; on AArch64 Linux, the Armv8 crypto-extension
//! kernel (3 lanes). On Coffee Lake (AVX2, no SHA-NI), the AVX2 kernel
//! delivers ~1.0–1.2 GiB/s aggregate vs ~430 MiB/s for single-buffer
//! SHA-256. Every kernel the host supports can be named explicitly
//! ([`multi_buffer::Kernel`], [`multi_buffer::supported`]); ISA-L's own
//! choice is [`multi_buffer::dispatched`], and [`sha256_parallel`] takes the
//! widest kernel a batch fills ([`multi_buffer::for_batch`]).
//!
//! BLAKE3 is always available and runs at ~3.8 GiB/s on AVX2 hardware
//! with its own internal 8-way parallelism.
//!
//! # Usage
//!
//! ```ignore
//! use rekindle_hash::{sha256_parallel, DigestAlgorithm, digest_oneshot};
//!
//! // Hash N independent chunks in parallel (SHA-256 multi-buffer when available)
//! let chunks: Vec<&[u8]> = vec![&data[..65536], &data[65536..131072]];
//! let mut digests = vec![[0u8; 32]; chunks.len()];
//! sha256_parallel(&chunks, &mut digests);
//!
//! // One-shot digest with algorithm selection
//! let d = digest_oneshot(DigestAlgorithm::Blake3, &data);
//!
//! // A specific multi-buffer kernel, when the host supports it
//! use rekindle_hash::multi_buffer::{sha256_mb, Kernel};
//! sha256_mb(Kernel::Avx2, &chunks, &mut digests)?;
//! ```

pub mod single;

#[cfg(feature = "sha256-mb")]
#[allow(unsafe_code)]
pub mod multi_buffer;

/// Digest algorithm selection for Merkle content hash verification.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    Sha256,
    #[default]
    Blake3,
}

/// Compute a one-shot digest using the specified algorithm.
pub fn digest_oneshot(algorithm: DigestAlgorithm, data: &[u8]) -> [u8; 32] {
    match algorithm {
        DigestAlgorithm::Sha256 => single::sha256_oneshot(data),
        DigestAlgorithm::Blake3 => single::blake3_oneshot(data),
    }
}

/// Compute SHA-256 digests of N independent chunks in parallel.
///
/// With the `sha256-mb` feature, when the batch fills the lanes of a kernel
/// this host supports, the chunks are hashed together by the widest such
/// kernel ([`multi_buffer::for_batch`]); otherwise each chunk is hashed by
/// single-buffer SHA-256 (aws-lc). Filling the lanes is the policy, not a
/// measured optimum: the `kernels` and `sha256_parallel` groups of
/// `benches/hash_compare.rs` measure each kernel against single-buffer
/// SHA-256 by batch size, which is where the crossover on a given host is
/// read from.
///
/// # Panics
///
/// When `chunks.len() != digests_out.len()`.
pub fn sha256_parallel(chunks: &[&[u8]], digests_out: &mut [[u8; 32]]) {
    assert_eq!(
        chunks.len(),
        digests_out.len(),
        "chunk count must match digest output count"
    );

    #[cfg(feature = "sha256-mb")]
    if let Some(kernel) = multi_buffer::for_batch(chunks.len()) {
        if let Err(error) = multi_buffer::sha256_mb(kernel, chunks, digests_out) {
            // The kernel is supported and the slot counts match, so the
            // only failures left are a message beyond SHA-256's length
            // limit and a scheduler that loses a job.
            panic!("multi-buffer SHA-256 through {kernel}: {error}");
        }
        return;
    }

    for (chunk, digest) in chunks.iter().zip(digests_out.iter_mut()) {
        *digest = single::sha256_oneshot(chunk);
    }
}
