//! Runtime crypto capability detection and performance probing.
//!
//! Runs at daemon startup after the encrypt pool is built. Measures
//! actual throughput of each crypto primitive and reports:
//! - Which AEAD algorithm is available and at what throughput
//! - Which SHA-256 path ISA-L selected (AVX2, SSE, base C)
//! - BLAKE3 throughput baseline
//! - Whether the measured throughputs meet the design targets
//!
//! Results are logged at startup and exposed via `rekindle status --doctor`.

use std::time::Instant;

/// Results of the crypto capability probe.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CryptoCapabilities {
    /// AES-256-GCM seal throughput in GiB/s (64 KiB chunks).
    pub aes_gcm_seal_gibs: f64,
    /// AES-256-GCM open throughput in GiB/s (64 KiB chunks).
    pub aes_gcm_open_gibs: f64,
    /// AEGIS-128L seal throughput in GiB/s (64 KiB chunks). 0 if feature disabled.
    pub aegis_seal_gibs: f64,
    /// SHA-256 single-buffer throughput in MiB/s (64 KiB).
    pub sha256_single_mibs: f64,
    /// SHA-256 multi-buffer throughput in MiB/s (8×64 KiB). 0 if feature disabled.
    pub sha256_mb_mibs: f64,
    /// BLAKE3 throughput in GiB/s (64 KiB).
    pub blake3_gibs: f64,
    /// SHA-256 multi-buffer speedup ratio (sequential wall time / parallel wall time).
    /// >= 3.0 means SIMD is active. ~1.0 means sequential fallback.
    pub sha256_mb_speedup: f64,
    /// Whether SHA-256 multi-buffer is using SIMD (AVX2/AVX512) vs base C.
    pub sha256_mb_simd_active: bool,
    /// Selected AEAD algorithm name for the bulk cipher.
    pub bulk_aead_algorithm: String,
}

impl CryptoCapabilities {
    /// Whether all throughputs meet design targets.
    pub fn meets_targets(&self) -> bool {
        self.aes_gcm_seal_gibs >= 3.0
            && self.blake3_gibs >= 3.0
            && (!self.sha256_mb_simd_active || self.sha256_mb_mibs >= 800.0)
    }
}

/// Run the full crypto capability probe. Takes ~200ms.
///
/// Call once at daemon startup after encrypt pool construction.
/// Results are logged and stored for the doctor endpoint.
pub fn probe() -> CryptoCapabilities {
    let chunk = vec![0xABu8; 65536];
    let iterations: u32 = 500;

    // ── AES-256-GCM ────────────────────────────────────────────
    let cipher = super::cipher::BulkCipher::new(&[0x42; 32]);
    let aes_gcm_seal_gibs = {
        let mut buf = chunk.clone();
        // Warmup
        for i in 0..10u64 {
            let _ = cipher.seal_in_place(i, b"", &mut buf);
            buf.copy_from_slice(&chunk);
        }
        let start = Instant::now();
        for i in 0..iterations {
            let _ = cipher.seal_in_place(u64::from(100 + i), b"", &mut buf);
            buf.copy_from_slice(&chunk);
        }
        let elapsed = start.elapsed();
        let total_gib = (65536.0 * f64::from(iterations)) / (1024.0 * 1024.0 * 1024.0);
        total_gib / elapsed.as_secs_f64()
    };

    let aes_gcm_open_gibs = {
        let mut buf = chunk.clone();
        let tag = cipher.seal_in_place(0, b"", &mut buf).unwrap();
        let mut ct_and_tag = Vec::with_capacity(buf.len() + 16);
        ct_and_tag.extend_from_slice(&buf);
        ct_and_tag.extend_from_slice(&tag);
        let template = ct_and_tag.clone();
        // Warmup
        for _ in 0..10 {
            ct_and_tag.copy_from_slice(&template);
            let _ = cipher.open_in_place(0, b"", &mut ct_and_tag);
        }
        let start = Instant::now();
        for _ in 0..iterations {
            ct_and_tag.copy_from_slice(&template);
            let _ = cipher.open_in_place(0, b"", &mut ct_and_tag);
        }
        let elapsed = start.elapsed();
        let total_gib = (65536.0 * f64::from(iterations)) / (1024.0 * 1024.0 * 1024.0);
        total_gib / elapsed.as_secs_f64()
    };

    // ── AEGIS-128L ─────────────────────────────────────────────
    #[cfg(feature = "aegis")]
    let aegis_seal_gibs = {
        use rekindle_aead::BulkAead;
        let key = rekindle_aead::aegis128l::Aegis128LKey::new(&[0x42; 16]);
        let nonce = key.build_nonce(0);
        let mut ct = vec![0u8; 65536];
        let mut tag = [0u8; 16];
        // Warmup
        for _ in 0..10 {
            let _ = key.seal_detached(&nonce, b"", &chunk, &mut ct, &mut tag);
        }
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = key.seal_detached(&nonce, b"", &chunk, &mut ct, &mut tag);
        }
        let elapsed = start.elapsed();
        let total_gib = (65536.0 * f64::from(iterations)) / (1024.0 * 1024.0 * 1024.0);
        total_gib / elapsed.as_secs_f64()
    };
    #[cfg(not(feature = "aegis"))]
    let aegis_seal_gibs = 0.0;

    // ── SHA-256 single ─────────────────────────────────────────
    let sha256_single_mibs = {
        // Warmup
        for _ in 0..10 {
            let _ = rekindle_hash::single::sha256_oneshot(&chunk);
        }
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = rekindle_hash::single::sha256_oneshot(&chunk);
        }
        let elapsed = start.elapsed();
        let total_mib = (65536.0 * f64::from(iterations)) / (1024.0 * 1024.0);
        total_mib / elapsed.as_secs_f64()
    };

    // ── SHA-256 multi-buffer ───────────────────────────────────
    let (sha256_mb_mibs, sha256_mb_speedup) = rekindle_hash::probe_sha256_mb_performance();

    // ── BLAKE3 ─────────────────────────────────────────────────
    let blake3_gibs = {
        for _ in 0..10 {
            let _ = rekindle_hash::single::blake3_oneshot(&chunk);
        }
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = rekindle_hash::single::blake3_oneshot(&chunk);
        }
        let elapsed = start.elapsed();
        let total_gib = (65536.0 * f64::from(iterations)) / (1024.0 * 1024.0 * 1024.0);
        total_gib / elapsed.as_secs_f64()
    };

    // ISA-L multi-buffer speedup ratio: wall time of 8 sequential hashes
    // divided by wall time of 8 parallel hashes. If SIMD AVX2 8-way is
    // active, the ratio should be >= 4.0 (8 chunks in ~1 chunk time).
    // A ratio ~1.0 means sequential fallback.
    let sha256_mb_simd_active = sha256_mb_speedup >= 3.0;

    let caps = CryptoCapabilities {
        aes_gcm_seal_gibs,
        aes_gcm_open_gibs,
        aegis_seal_gibs,
        sha256_single_mibs,
        sha256_mb_mibs,
        blake3_gibs,
        sha256_mb_speedup,
        sha256_mb_simd_active,
        bulk_aead_algorithm: format!("{:?}", cipher.algorithm()),
    };

    tracing::info!(
        aes_gcm_seal_gibs = %format_args!("{:.2}", caps.aes_gcm_seal_gibs),
        aes_gcm_open_gibs = %format_args!("{:.2}", caps.aes_gcm_open_gibs),
        aegis_seal_gibs = %format_args!("{:.2}", caps.aegis_seal_gibs),
        sha256_single_mibs = %format_args!("{:.0}", caps.sha256_single_mibs),
        sha256_mb_mibs = %format_args!("{:.0}", caps.sha256_mb_mibs),
        sha256_mb_speedup = %format_args!("{:.1}x", caps.sha256_mb_speedup),
        sha256_mb_simd = caps.sha256_mb_simd_active,
        blake3_gibs = %format_args!("{:.2}", caps.blake3_gibs),
        meets_targets = caps.meets_targets(),
        "crypto capability probe complete"
    );

    if !caps.sha256_mb_simd_active {
        tracing::warn!(
            sha256_mb_speedup = %format_args!("{:.1}x", caps.sha256_mb_speedup),
            sha256_mb_mibs = %format_args!("{:.0}", caps.sha256_mb_mibs),
            sha256_single_mibs = %format_args!("{:.0}", caps.sha256_single_mibs),
            "SHA-256 multi-buffer — BLAKE3 Merkle (default) is unaffected"
        );
    }

    caps
}
