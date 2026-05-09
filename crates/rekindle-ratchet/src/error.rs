//! Ratchet error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RatchetError {
    // ── AEAD ────────────────────────────────────────────────────
    #[error("AEAD key construction failed")]
    AeadKey,

    #[error("AEAD tag verification failed (forgery, replay, or wrong key)")]
    AeadOpen,

    #[error("AEAD buffer too short for tag")]
    AeadBufferShort,

    // ── KDF ─────────────────────────────────────────────────────
    #[error("HKDF/HMAC derivation failed")]
    Kdf,

    // ── ECDH ────────────────────────────────────────────────────
    #[error("X25519 agreement failed")]
    Ecdh,

    // ── KEM ─────────────────────────────────────────────────────
    #[error("ML-KEM keygen failed")]
    KemKeygen,

    #[error("ML-KEM encapsulation failed")]
    KemEncaps,

    #[error("ML-KEM decapsulation failed")]
    KemDecaps,

    #[error("ML-KEM encapsulation key invalid")]
    KemEkInvalid,

    // ── Signature ───────────────────────────────────────────────
    #[error("Ed25519 signing failed")]
    SignFailed,

    #[error("Ed25519 verification failed")]
    VerifyFailed,

    // ── PQXDH ───────────────────────────────────────────────────
    #[error("PQXDH bundle invalid: {reason}")]
    PqxdhBundleInvalid { reason: String },

    #[error("PQXDH algorithm-byte prefix mismatch (F3 mitigation)")]
    PqxdhAlgPrefixMismatch,

    #[error("PQXDH signature verification failed on prekey")]
    PqxdhSigInvalid,

    #[error("PQXDH replayed bundle (SPK ID reused)")]
    PqxdhReplayDetected,

    #[error("PQXDH no prekey available in bundle")]
    PqxdhNoPrekey,

    #[error("PQXDH last-resort PQ prekey used (informational)")]
    PqxdhLastResortUsed,

    #[error("PQXDH OT/LR domain tag mismatch (F4 mitigation)")]
    PqxdhDomainTagMismatch,

    // ── Double Ratchet ──────────────────────────────────────────
    #[error("DR header decryption failed (HKr and NHKr both rejected)")]
    DrHeaderDecrypt,

    #[error("DR message decryption failed")]
    DrMessageDecrypt,

    #[error("DR skip limit exceeded (max {max})")]
    DrSkipLimit { max: u32 },

    #[error("DR counter overflow — forced ratchet required")]
    DrCounterOverflow,

    #[error("DR duplicate message n={n}")]
    DrDuplicateMessage { n: u32 },

    // ── SPQR / Braid ────────────────────────────────────────────
    #[error("Braid illegal state transition from {from}")]
    BraidIllegalTransition { from: String },

    #[error("Braid epoch mismatch (expected {expected}, got {got})")]
    BraidEpochMismatch { expected: u32, got: u32 },

    #[error("Braid ek_vector hash mismatch (hek verification failed)")]
    BraidHekMismatch,

    #[error("Braid chunk reassembly failed (epoch {epoch})")]
    BraidChunkReassembly { epoch: u32 },

    // ── Erasure coding ──────────────────────────────────────────
    #[error("erasure encode failed: {0}")]
    ErasureEncode(String),

    #[error("erasure decode failed: insufficient shards")]
    ErasureDecode,

    // ── Session ─────────────────────────────────────────────────
    #[error("session not found")]
    SessionNotFound,

    #[error("session state corruption: {0}")]
    SessionCorrupt(String),

    #[error("session promotion pending receipt-ack (Cremers 2023 guard)")]
    PromotionPending,

    // ── RNG ─────────────────────────────────────────────────────
    #[error("RNG failed")]
    Rng,

    // ── aws-lc-rs ───────────────────────────────────────────────
    #[error("aws-lc-rs error")]
    AwsLc(#[from] aws_lc_rs::error::Unspecified),
}
