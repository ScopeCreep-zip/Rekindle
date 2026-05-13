//! Multi-buffer hash implementations for Rekindle Merkle digest.
//!
//! The `sha256-mb` feature enables 8-way parallel SHA-256 via ISA-L
//! crypto's AVX2 multi-buffer implementation. On Coffee Lake (no SHA-NI),
//! this delivers ~1.0–1.2 GiB/s vs ~430 MiB/s for single-buffer SHA-256.
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
//! ```

pub mod single;

#[cfg(all(feature = "sha256-mb", target_arch = "x86_64"))]
#[allow(unsafe_code)]
pub mod multi_buffer;

/// Digest algorithm selection — mirrors `rekindle_node::ipc::bulk::verify::DigestAlgorithm`.
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

/// Probe SHA-256 multi-buffer performance to detect which ISA-L
/// dispatch path was selected at runtime. Returns the throughput
/// in MiB/s for a single 8×64KiB batch. If the result is < 800 MiB/s,
/// the dispatcher fell back to base C instead of AVX2.
/// Probe SHA-256 multi-buffer performance. Returns (aggregate_mibs, speedup_ratio).
///
/// `aggregate_mibs`: total MiB/s across all 8 chunks.
/// `speedup_ratio`: wall-time ratio of single-buffer vs multi-buffer for the same
/// total data. A ratio >= 4.0 means AVX2 8-way is active. A ratio ~1.0 means
/// the dispatcher fell back to sequential base C.
pub fn probe_sha256_mb_performance() -> (f64, f64) {
    #[cfg(all(feature = "sha256-mb", target_arch = "x86_64"))]
    {
        if !multi_buffer::has_avx2() {
            return (0.0, 1.0);
        }
    }
    #[cfg(not(all(feature = "sha256-mb", target_arch = "x86_64")))]
    {
        return (0.0, 1.0);
    }

    #[cfg(all(feature = "sha256-mb", target_arch = "x86_64"))]
    {
        let chunk = vec![0xABu8; 65536];
        let chunks: Vec<Vec<u8>> = (0..8u8).map(|i| vec![i; 65536]).collect();
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();
        let mut digests = vec![[0u8; 32]; 8];

        // Warmup — first call triggers the multibinary CPUID dispatch.
        sha256_parallel(&refs, &mut digests);
        for _ in 0..10 {
            let _ = single::sha256_oneshot(&chunk);
        }

        // Time single-buffer: 8 sequential hashes.
        let start = std::time::Instant::now();
        for _ in 0..100 {
            for c in &chunks {
                let _ = single::sha256_oneshot(c);
            }
        }
        let single_elapsed = start.elapsed();

        // Time multi-buffer: 8 parallel hashes.
        let start = std::time::Instant::now();
        for _ in 0..100 {
            sha256_parallel(&refs, &mut digests);
        }
        let mb_elapsed = start.elapsed();

        let total_bytes = 8.0 * 65536.0 * 100.0;
        let aggregate_mibs = total_bytes / (1024.0 * 1024.0) / mb_elapsed.as_secs_f64();
        let speedup = single_elapsed.as_secs_f64() / mb_elapsed.as_secs_f64();

        (aggregate_mibs, speedup)
    }
}

/// Compute SHA-256 digests of N independent chunks in parallel.
///
/// Uses ISA-L multi-buffer AVX2 (8-way) when the `sha256-mb` feature is
/// enabled and chunk count >= 4. Falls back to sequential single-buffer
/// SHA-256 otherwise.
///
/// `chunks.len()` must equal `digests_out.len()`.
pub fn sha256_parallel(chunks: &[&[u8]], digests_out: &mut [[u8; 32]]) {
    assert_eq!(chunks.len(), digests_out.len(),
        "chunk count must match digest output count");

    #[cfg(all(feature = "sha256-mb", target_arch = "x86_64"))]
    {
        if chunks.len() >= 8 && multi_buffer::has_avx2() {
            multi_buffer::sha256_mb_parallel(chunks, digests_out);
            return;
        }
    }

    // Fallback: sequential single-buffer.
    for (chunk, digest) in chunks.iter().zip(digests_out.iter_mut()) {
        *digest = single::sha256_oneshot(chunk);
    }
}
