//! 8-way parallel SHA-256 via ISA-L's AVX2 scheduler-level API.
//!
//! Bypasses the ISA-L context manager (`sha256_ctx_mgr_submit`) which
//! serializes `HASH_ENTIRE` jobs. Instead calls the scheduler directly:
//! `_sha256_mb_mgr_init_avx2`, `_sha256_mb_mgr_submit_avx2`,
//! `_sha256_mb_mgr_flush_avx2`. These operate on `ISAL_SHA256_MB_JOB_MGR`
//! and `ISAL_SHA256_JOB` — the low-level lane manager that batches
//! 8 jobs simultaneously via transposed AVX2 YMM registers.
//!
//! # Why bypass the context manager?
//!
//! The context manager's `sha256_ctx_mgr_submit` with `HASH_ENTIRE` enters
//! a `resubmit` loop that processes ALL blocks of a single job before
//! returning. The 8 lanes are never simultaneously occupied. The scheduler
//! API adds a job to a lane and only runs the kernel when all 8 lanes
//! have work — true 8-way parallelism.
//!
//! # Safety
//!
//! We use opaque Rust structs sized from build-time probes (2× margin).
//! The `ISAL_SHA256_JOB` struct layout is stable (ABI contract for the
//! NASM assembly kernel). The `ISAL_SHA256_MB_JOB_MGR` is zeroed via
//! `_sha256_mb_mgr_init_avx2` which sets `unused_lanes` and clears all
//! lane data.

use std::mem::MaybeUninit;

/// Check for AVX2 support at runtime via CPUID. Cached in a `OnceLock`.
///
/// The `_sha256_mb_mgr_*_avx2` FFI functions execute `vmovdqu ymm`
/// instructions. On CPUs without AVX2 (pre-Haswell, VMs with AVX2
/// disabled), calling them produces SIGILL. This gate must be checked
/// before any call into `sha256_mb_parallel`.
pub fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *HAS.get_or_init(|| is_x86_feature_detected!("avx2"))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

mod sizes {
    include!(concat!(env!("OUT_DIR"), "/isal_sizes.rs"));
}

// ── Opaque scheduler-level structs ─────────────────────────────────

/// Opaque `ISAL_SHA256_MB_JOB_MGR`. 2× probed CTX_MGR size for margin.
/// The CTX_MGR contains the JOB_MGR as its first field, so CTX_MGR size >= JOB_MGR size.
#[repr(C, align(64))]
struct Sha256MbJobMgr {
    _opaque: [u8; 8192],
}

/// `ISAL_SHA256_JOB` layout from sha256_job.asm:
///
/// ```text
/// offset  0: buffer         (8 bytes, *const u8)
/// offset  8: len            (8 bytes, u64 — block count)
/// offset 16: <padding>      (48 bytes to reach align-64)
/// offset 64: result_digest  (32 bytes, [u32; 8])
/// offset 96: status         (4 bytes, u32)
/// offset100: <padding>      (4 bytes to reach align-8)
/// offset104: user_data      (8 bytes, *mut c_void)
/// offset112: <padding to struct align 64 = 128 bytes total>
/// ```
///
/// Verified by `sha256_mb_mgr_submit_avx2.asm`:
///   line 123: `mov DWORD(len), [job + _len]`       → offset 8
///   line 132: `vmovdqu xmm0, [job + _result_digest]` → offset 64
///   line 144: `mov p, [job + _buffer]`             → offset 0
#[repr(C, align(64))]
struct Sha256Job {
    buffer: *const u8,          // offset 0
    len: u64,                   // offset 8, block count
    _pad0: [u8; 48],           // offset 16..64, padding to align result_digest
    result_digest: [u32; 8],    // offset 64, SHA-256 state
    status: u32,                // offset 96
    _pad1: u32,                // offset 100, alignment padding
    user_data: u64,             // offset 104 (using u64 instead of *mut to avoid Send issues)
    _pad2: [u8; 16],          // offset 112..128, pad to 128 = 2×64 alignment
}

/// SHA-256 initial digest state (H0..H7).
const SHA256_INIT: [u32; 8] = [
    0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
    0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
];

const BLOCK_SIZE: usize = 64;

// Compile-time proof that our Rust structs match the C ABI.
static_assertions::const_assert!(std::mem::size_of::<Sha256MbJobMgr>() >= sizes::PROBED_JOB_MGR_SIZE);
static_assertions::const_assert!(std::mem::size_of::<Sha256Job>() >= sizes::PROBED_JOB_SIZE);

// Verify field offsets match the assembly's expectations.
// These offsets are hardcoded in sha256_mb_mgr_submit_avx2.asm.
static_assertions::const_assert!(std::mem::offset_of!(Sha256Job, buffer) == sizes::PROBED_JOB_BUFFER_OFFSET);
static_assertions::const_assert!(std::mem::offset_of!(Sha256Job, len) == sizes::PROBED_JOB_LEN_OFFSET);
static_assertions::const_assert!(std::mem::offset_of!(Sha256Job, result_digest) == sizes::PROBED_JOB_DIGEST_OFFSET);
static_assertions::const_assert!(std::mem::offset_of!(Sha256Job, status) == sizes::PROBED_JOB_STATUS_OFFSET);

extern "C" {
    fn _sha256_mb_mgr_init_avx2(state: *mut Sha256MbJobMgr);
    fn _sha256_mb_mgr_submit_avx2(state: *mut Sha256MbJobMgr, job: *mut Sha256Job)
        -> *mut Sha256Job;
    fn _sha256_mb_mgr_flush_avx2(state: *mut Sha256MbJobMgr) -> *mut Sha256Job;
}

/// Hash N chunks in parallel using ISA-L's AVX2 scheduler directly.
///
/// For N <= 8, all chunks are processed in one batch with true 8-way
/// parallelism. For N > 8, chunks are processed in batches of 8.
///
/// # Panics
///
/// Panics if `chunks.len() != digests_out.len()`.
pub fn sha256_mb_parallel(chunks: &[&[u8]], digests_out: &mut [[u8; 32]]) {
    assert_eq!(chunks.len(), digests_out.len());
    if chunks.is_empty() {
        return;
    }

    let mut offset = 0;
    while offset < chunks.len() {
        let end = (offset + 8).min(chunks.len());
        sha256_mb_batch(&chunks[offset..end], &mut digests_out[offset..end]);
        offset = end;
    }
}

/// Hash up to 8 chunks using the AVX2 scheduler with true lane parallelism.
///
/// Each chunk is submitted as all-blocks-at-once. The scheduler queues
/// each job in a lane. When 8 lanes are full, the AVX2 kernel fires
/// across all 8 simultaneously. For < 8 jobs, flush processes remaining.
fn sha256_mb_batch(chunks: &[&[u8]], digests_out: &mut [[u8; 32]]) {
    let n = chunks.len();
    assert!(n <= 8);
    assert_eq!(n, digests_out.len());

    // SHA-256 padding: the scheduler expects the buffer to contain the
    // raw message bytes. We need to handle padding ourselves since we're
    // bypassing the context manager. Each chunk needs:
    // 1. The message bytes
    // 2. A 0x80 byte
    // 3. Zero padding to reach 56 mod 64
    // 4. The 64-bit big-endian bit length
    //
    // We create padded buffers for each chunk.
    let mut padded: Vec<Vec<u8>> = Vec::with_capacity(n);
    for chunk in chunks {
        let msg_len = chunk.len();
        let bit_len = (msg_len as u64) * 8;

        // Number of blocks after padding
        let padded_len = if (msg_len % BLOCK_SIZE) < 56 {
            // Padding fits in the last block
            ((msg_len / BLOCK_SIZE) + 1) * BLOCK_SIZE
        } else {
            // Need an extra block for padding
            ((msg_len / BLOCK_SIZE) + 2) * BLOCK_SIZE
        };

        let mut buf = vec![0u8; padded_len];
        buf[..msg_len].copy_from_slice(chunk);
        buf[msg_len] = 0x80;
        // Last 8 bytes = big-endian bit length
        buf[padded_len - 8..].copy_from_slice(&bit_len.to_be_bytes());
        padded.push(buf);
    }

    // SAFETY: _sha256_mb_mgr_init_avx2 zeroes and initializes the manager.
    // Jobs are stack-allocated with known layout. All pointers are valid
    // for the padded buffer lifetimes (which outlive the unsafe block).
    unsafe {
        let mut mgr = MaybeUninit::<Sha256MbJobMgr>::zeroed().assume_init();
        _sha256_mb_mgr_init_avx2(&raw mut mgr);

        // Create jobs with initial digest state and padded buffer pointers.
        let mut jobs: Vec<Sha256Job> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_possible_truncation)]
                let num_blocks = (padded[i].len() / BLOCK_SIZE) as u64;
                Sha256Job {
                    buffer: padded[i].as_ptr(),
                    len: num_blocks,
                    _pad0: [0u8; 48],
                    result_digest: SHA256_INIT,
                    status: 0,
                    _pad1: 0,
                    user_data: 0,
                    _pad2: [0u8; 16],
                }
            })
            .collect();

        // Submit all jobs. The scheduler fills lanes and fires the AVX2
        // kernel when 8 lanes are occupied. For < 8 jobs, some lanes
        // remain empty until flush.
        for job in &mut jobs {
            _sha256_mb_mgr_submit_avx2(&raw mut mgr, std::ptr::from_mut(job));
        }

        // Flush remaining lanes.
        loop {
            let completed = _sha256_mb_mgr_flush_avx2(&raw mut mgr);
            if completed.is_null() {
                break;
            }
        }

        // Extract digests — result_digest is [u32; 8] in host byte order.
        // SHA-256 output is big-endian, so we need to convert each u32.
        for (i, job) in jobs.iter().enumerate() {
            for (j, word) in job.result_digest.iter().enumerate() {
                digests_out[i][j * 4..(j + 1) * 4].copy_from_slice(&word.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mb_matches_single_buffer() {
        let chunks: Vec<Vec<u8>> = (0..16)
            .map(|i| vec![i as u8; 65519])
            .collect();
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();

        let mut mb_digests = vec![[0u8; 32]; 16];
        sha256_mb_parallel(&refs, &mut mb_digests);

        for (i, chunk) in chunks.iter().enumerate() {
            let single = super::super::single::sha256_oneshot(chunk);
            assert_eq!(
                mb_digests[i], single,
                "multi-buffer digest mismatch at chunk {i}"
            );
        }
    }

    #[test]
    fn mb_handles_non_multiple_of_8() {
        let chunks: Vec<Vec<u8>> = (0..11)
            .map(|i| vec![i as u8; 1024])
            .collect();
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();

        let mut digests = vec![[0u8; 32]; 11];
        sha256_mb_parallel(&refs, &mut digests);

        for (i, chunk) in chunks.iter().enumerate() {
            let expected = super::super::single::sha256_oneshot(chunk);
            assert_eq!(digests[i], expected, "mismatch at chunk {i}");
        }
    }

    #[test]
    fn mb_empty_input() {
        let chunks: Vec<&[u8]> = vec![];
        let mut digests: Vec<[u8; 32]> = vec![];
        sha256_mb_parallel(&chunks, &mut digests);
    }

    #[test]
    fn mb_single_chunk() {
        let data = vec![0xABu8; 65519];
        let refs = vec![data.as_slice()];
        let mut digests = vec![[0u8; 32]; 1];
        sha256_mb_parallel(&refs, &mut digests);
        assert_eq!(digests[0], super::super::single::sha256_oneshot(&data));
    }

    #[test]
    fn mb_small_chunks() {
        let chunks: Vec<Vec<u8>> = (0..8)
            .map(|i| vec![i as u8; 13])
            .collect();
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();

        let mut digests = vec![[0u8; 32]; 8];
        sha256_mb_parallel(&refs, &mut digests);

        for (i, chunk) in chunks.iter().enumerate() {
            let expected = super::super::single::sha256_oneshot(chunk);
            assert_eq!(digests[i], expected, "small chunk mismatch at {i}");
        }
    }

    #[test]
    fn mb_exact_block_multiple() {
        let chunks: Vec<Vec<u8>> = (0..8)
            .map(|i| vec![i as u8; 64 * 16])
            .collect();
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();

        let mut digests = vec![[0u8; 32]; 8];
        sha256_mb_parallel(&refs, &mut digests);

        for (i, chunk) in chunks.iter().enumerate() {
            let expected = super::super::single::sha256_oneshot(chunk);
            assert_eq!(digests[i], expected, "exact block mismatch at {i}");
        }
    }

    #[test]
    fn mb_mixed_sizes() {
        let chunks: Vec<Vec<u8>> = vec![
            vec![0xAA; 13],
            vec![0xBB; 64],
            vec![0xCC; 100],
            vec![0xDD; 1024],
            vec![0xEE; 65519],
            vec![0xFF; 65536],
            vec![0x11; 7],
            vec![0x22; 128],
        ];
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();

        let mut digests = vec![[0u8; 32]; 8];
        sha256_mb_parallel(&refs, &mut digests);

        for (i, chunk) in chunks.iter().enumerate() {
            let expected = super::super::single::sha256_oneshot(chunk);
            assert_eq!(digests[i], expected, "mixed size mismatch at {i}");
        }
    }

    #[test]
    fn mb_empty_chunk() {
        let chunks: Vec<Vec<u8>> = vec![vec![]];
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();
        let mut digests = vec![[0u8; 32]; 1];
        sha256_mb_parallel(&refs, &mut digests);
        assert_eq!(digests[0], super::super::single::sha256_oneshot(&[]));
    }
}
