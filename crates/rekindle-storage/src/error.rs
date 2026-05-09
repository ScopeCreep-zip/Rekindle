//! Storage error types.
//!
//! Every variant is either terminal (operation failed, caller handles) or
//! informational (caller retries with a different path). No variant requires
//! the caller to guess what happened.

use thiserror::Error;

/// Alias for `Result<T, StorageError>`.
pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Error, Debug)]
pub enum StorageError {
    // ── Vault lifecycle ─────────────────────────────────────────
    #[error("vault not open")]
    VaultNotOpen,

    #[error("vault already exists at path")]
    VaultAlreadyExists,

    #[error("vault creation failed: {reason}")]
    VaultCreationFailed { reason: String },

    #[error("vault open failed (wrong key or corrupt): {reason}")]
    VaultOpenFailed { reason: String },

    #[error("vault corrupt: {reason}")]
    VaultCorrupt { reason: String },

    #[error("schema migration failed from v{from} to v{to}: {reason}")]
    SchemaMigration { from: u32, to: u32, reason: String },

    // ── Entry crypto ────────────────────────────────────────────
    #[error("entry encryption failed: {0}")]
    EntryEncrypt(String),

    #[error("entry decryption failed (tampered or wrong key): {0}")]
    EntryDecrypt(String),

    #[error("encrypted entry too short ({len} bytes, minimum 28)")]
    EntryTooShort { len: usize },

    // ── SQL ─────────────────────────────────────────────────────
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    // ── Key operations ──────────────────────────────────────────
    #[error("key not found: {label}")]
    KeyNotFound { label: String },

    #[error("key label invalid: {label}")]
    KeyLabelInvalid { label: String },

    // ── Session operations ──────────────────────────────────────
    #[error("session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("skipped key limit exceeded (max {max})")]
    SkippedKeyLimit { max: u32 },

    #[error("skipped key expired")]
    SkippedKeyExpired,

    // ── Unlock ──────────────────────────────────────────────────
    #[error("no unlock methods enrolled")]
    NoUnlockMethods,

    #[error("unlock method unavailable: {method}")]
    UnlockMethodUnavailable { method: String },

    #[error("passphrase derivation failed: {0}")]
    PassphraseDerivation(String),

    #[error("keyring operation failed: {0}")]
    KeyringFailed(String),

    #[error("master key unwrap failed (wrong password or corrupt keyring)")]
    MasterKeyUnwrapFailed,

    #[error("SSH agent failed: {0}")]
    SshAgentFailed(String),

    #[error("salt file missing or corrupt: {path}")]
    SaltCorrupt { path: String },

    // ── Audit ───────────────────────────────────────────────────
    #[error("audit chain broken at entry {index}")]
    AuditChainBroken { index: u64 },

    // ── Session metadata ────────────────────────────────────────
    #[error("session.json integrity check failed")]
    SessionMetaIntegrity,

    #[error("session.json parse failed: {0}")]
    SessionMetaParse(String),

    // ── Platform ────────────────────────────────────────────────
    #[error("platform keyring not available")]
    KeyringNotAvailable,

    // ── RNG ─────────────────────────────────────────────────────
    #[error("random number generation failed: {0}")]
    RngFailed(String),
}
