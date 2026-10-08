//! Error types for the v2.0 community system.

use thiserror::Error;

/// Errors from cryptographic operations (rekindle-secrets).
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key generation failed: {0}")]
    KeyGeneration(String),

    #[error("signing failed: {0}")]
    Signing(String),

    #[error("signature verification failed: {0}")]
    Verification(String),

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("invalid key material: {0}")]
    InvalidKey(String),

    #[error("vault storage error: {0}")]
    Storage(String),
}

/// Errors from DHT record operations (rekindle-records).
#[derive(Debug, Error)]
pub enum DhtError {
    #[error("record creation failed: {0}")]
    CreateFailed(String),

    #[error("record open failed: {0}")]
    OpenFailed(String),

    #[error("read failed: {0}")]
    ReadFailed(String),

    #[error("write failed: {0}")]
    WriteFailed(String),

    #[error("write failed after {attempts} retries: {reason}")]
    WriteExhausted { attempts: u32, reason: String },

    #[error("inspect failed: {0}")]
    InspectFailed(String),

    #[error("watch setup failed: {0}")]
    WatchFailed(String),
}

/// Errors from gossip mesh operations (rekindle-gossip).
#[derive(Debug, Error)]
pub enum GossipError {
    #[error("broadcast failed: {0}")]
    BroadcastFailed(String),

    #[error("rate limited: sender {sender} exceeded {limit} msgs/sec")]
    RateLimited { sender: String, limit: u32 },

    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),

    #[error("signature verification failed")]
    SignatureInvalid,
}

/// Errors from governance CRDT operations (rekindle-governance).
#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("insufficient permission: need {required:#x}, have {actual:#x}")]
    InsufficientPermission { required: u64, actual: u64 },

    #[error("member is banned")]
    Banned,

    #[error("community is full (all {max_slots} slots occupied)")]
    CommunityFull { max_slots: u32 },

    #[error("slot claim collision at index {slot}")]
    SlotCollision { slot: u32 },

    #[error("invalid governance entry: {0}")]
    InvalidEntry(String),
}

/// Top-level community error wrapping all subsystem errors.
#[derive(Debug, Error)]
pub enum CommunityError {
    #[error(transparent)]
    Crypto(#[from] CryptoError),

    #[error(transparent)]
    Dht(#[from] DhtError),

    #[error(transparent)]
    Gossip(#[from] GossipError),

    #[error(transparent)]
    Governance(#[from] GovernanceError),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("{0}")]
    Other(String),
}

impl From<serde_json::Error> for CommunityError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}
