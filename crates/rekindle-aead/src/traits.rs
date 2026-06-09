//! Unified AEAD trait and shared types.

/// AEAD algorithm identifier for wire-protocol negotiation.
///
/// Transmitted in `BulkTransferStart` so both sides agree on the
/// cipher before data flows. The default is AES-256-GCM for maximum
/// compatibility; AEGIS-128L is opt-in for performance-critical paths.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AeadAlgorithm {
    /// AES-256-GCM — FIPS-validated, ~3.6 GiB/s seal on Coffee Lake.
    #[default]
    Aes256Gcm,
    /// AEGIS-128L — 8-block (1024-bit) state, 32-byte rate per update.
    /// 128-bit key. Requires the `aegis` feature flag.
    Aegis128L,
    /// AEGIS-128X2 — 2×parallel AEGIS-128L, 64-byte rate per update.
    /// Faster than AEGIS-128L even without VAES, faster with AVX2.
    /// 128-bit key. Default when `aegis` feature enabled.
    Aegis128X2,
}

/// Errors from AEAD operations.
#[derive(Debug)]
pub enum AeadError {
    /// AEAD tag verification failed — ciphertext was tampered or wrong key/nonce.
    AuthFailed,
    /// Key initialization failed (invalid key length or algorithm unavailable).
    Init,
    /// Input/output buffer size does not match requirements.
    BufferSize { need: usize, have: usize },
}

impl std::fmt::Display for AeadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "AEAD authentication failed"),
            Self::Init => write!(f, "AEAD key initialization failed"),
            Self::BufferSize { need, have } => {
                write!(f, "buffer size mismatch: need {need}, have {have}")
            }
        }
    }
}

impl std::error::Error for AeadError {}

/// Unified AEAD interface for bulk transport encryption.
///
/// All methods take `&self` — implementations must be internally
/// thread-safe. Share via `Arc<dyn BulkAead>` across rayon workers.
///
/// # Contract
///
/// - `seal_detached` MUST zeroize any intermediate plaintext buffers
/// - `open_detached` MUST zero `plaintext_out` on authentication failure
/// - `build_nonce` MUST produce a nonce of exactly `nonce_len()` bytes
/// - Nonce reuse with the same key is catastrophic for both AES-GCM
///   and AEGIS-128L — callers MUST use monotonic counters
pub trait BulkAead: Send + Sync {
    /// Algorithm identifier for wire-protocol negotiation.
    fn algorithm(&self) -> AeadAlgorithm;

    /// Nonce length in bytes. 12 for AES-GCM, 16 for AEGIS-128L.
    fn nonce_len(&self) -> usize;

    /// Tag length in bytes. 16 for both algorithms.
    fn tag_len(&self) -> usize { 16 }

    /// Encrypt with detached tag.
    ///
    /// Reads `plaintext`, writes ciphertext to `ciphertext_out` (same
    /// length), writes authentication tag to `tag_out` (at least
    /// `tag_len()` bytes).
    fn seal_detached(
        &self,
        nonce: &[u8],
        aad: &[u8],
        plaintext: &[u8],
        ciphertext_out: &mut [u8],
        tag_out: &mut [u8],
    ) -> Result<(), AeadError>;

    /// Decrypt with detached tag.
    ///
    /// Reads `ciphertext`, verifies `tag`, writes plaintext to
    /// `plaintext_out` (same length as `ciphertext`).
    ///
    /// On authentication failure, `plaintext_out` is zeroed by the
    /// underlying implementation (aws-lc for GCM, libaegis for AEGIS).
    fn open_detached(
        &self,
        nonce: &[u8],
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8],
        plaintext_out: &mut [u8],
    ) -> Result<(), AeadError>;

    /// Encrypt in-place with separate tag.
    ///
    /// Reads plaintext from `in_out`, overwrites `in_out` with ciphertext
    /// (same length), returns the 16-byte authentication tag. This is the
    /// zero-copy hot path for the bulk transport pipeline.
    ///
    /// Every implementation MUST provide a true in-place path — no
    /// temporary copies, no allocations.
    fn seal_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
    ) -> Result<[u8; 16], AeadError>;

    /// Decrypt in-place from a contiguous `[ciphertext || tag]` buffer.
    ///
    /// On success, the first `ct_and_tag.len() - tag_len()` bytes contain
    /// plaintext. Returns the plaintext length.
    ///
    /// Every implementation MUST provide a true in-place path — no
    /// temporary copies, no allocations.
    fn open_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        ct_and_tag: &mut [u8],
    ) -> Result<usize, AeadError>;

    /// Build a nonce from a u64 counter into a fixed 16-byte array.
    ///
    /// - AES-GCM:   `[0x00; 4][counter BE; 8][0x00; 4]` — use first 12 bytes
    /// - AEGIS-128L: `[0x00; 8][counter BE; 8]` — use all 16 bytes
    ///
    /// Callers slice to `&nonce[..self.nonce_len()]` when passing to AEAD.
    fn build_nonce(&self, counter: u64) -> [u8; 16];

    /// Maximum safe nonce counter value for this algorithm.
    ///
    /// The `NonceCounter` aborts the process when the counter reaches
    /// this limit. The margin below `u64::MAX` accounts for:
    ///
    /// 1. **Contention headroom**: `fetch_add(1, Relaxed)` is not
    ///    linearizable. Under 8-way contention, multiple threads may
    ///    pass the limit check before any observes the updated value.
    ///    The margin must exceed the maximum concurrent callers.
    ///
    /// 2. **Rekey reservation**: Snow's `Cipher::rekey()` uses
    ///    `u64::MAX` as a sentinel nonce. The limit must exclude it.
    ///
    /// 3. **Abort-trapping runtimes**: In environments where
    ///    `process::abort()` is trapped (WASM, certain test harnesses),
    ///    a thread that passed the check before the abort may continue
    ///    with a nonce near the limit. The margin prevents that thread
    ///    from reaching `u64::MAX` on subsequent increments.
    ///
    /// Default: `u64::MAX - (1 << 20)` (~1M margin). Override in
    /// algorithm implementations if the nonce space is smaller (e.g.,
    /// a 32-bit counter sub-range would use `u32::MAX - (1 << 16)`).
    fn nonce_counter_limit(&self) -> u64 {
        u64::MAX - (1 << 20)
    }
}
