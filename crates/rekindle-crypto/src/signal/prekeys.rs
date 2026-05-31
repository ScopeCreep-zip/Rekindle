//! `PreKey` bundle wire type for Signal Protocol session establishment.
//!
//! Pre-key bundles are published to DHT profile subkey 5 so that new
//! contacts can establish a Signal session asynchronously. Generation,
//! bundle creation, and reuse-on-restart logic all live in
//! [`crate::signal::session::SignalSessionManager`]:
//! - `generate_prekey_bundle` — mint fresh keys + persist via the
//!   PreKeyStore + return a bundle for DHT publication
//! - `load_existing_prekey_bundle` (P1.2) — reconstruct a bundle from
//!   already-persisted keys without overwriting them
//!
//! Rotation is driven from `auth.rs::initialize_signal_manager`, which
//! prefers the existing bundle (so peers' cached PreKeyBundles stay
//! valid across restarts) and only mints fresh keys when the vault is
//! empty.
//!
//! Phase 3b of the decomposed-harvest plan augmented this struct with
//! PQXDH ML-KEM-768 fields. Wire format breaks for any peer expecting
//! the classical-only shape — pre-ship hard break per invariant #5.

use serde::{Deserialize, Serialize};

/// A bundle of public keys published to DHT for session establishment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreKeyBundle {
    // ── Classical X3DH layer ────────────────────────────────────
    /// Ed25519 identity public key.
    pub identity_key: Vec<u8>,
    /// X25519 signed prekey (public).
    pub signed_prekey: Vec<u8>,
    /// Ed25519 signature over `0x01 || signed_prekey`.
    pub signed_prekey_signature: Vec<u8>,
    /// Optional X25519 one-time prekey (consumed on first use).
    pub one_time_prekey: Option<Vec<u8>>,
    /// Identifier for the one-time X25519 prekey (matched on use).
    pub one_time_prekey_id: Option<u32>,
    /// Registration ID for Signal Protocol.
    pub registration_id: u32,

    // ── PQXDH layer ─────────────────────────────────────────────
    /// ML-KEM-768 last-resort public key (1184 bytes).
    pub pqpk_lr: Vec<u8>,
    /// Ed25519 signature over `0x02 || "LR" || pqpk_lr`.
    pub pqpk_lr_signature: Vec<u8>,
    /// Optional one-time ML-KEM-768 public key (1184 bytes).
    pub pqpk_ot: Option<Vec<u8>>,
    /// Ed25519 signature over `0x02 || "OT" || pqpk_ot`.
    pub pqpk_ot_signature: Option<Vec<u8>>,
    /// Identifier for the one-time PQ prekey (matched on use).
    pub pqpk_ot_id: Option<u32>,
}

impl PreKeyBundle {
    /// Serialize to bytes for DHT storage. Uses postcard so the encoding
    /// matches the daemon-track transport crate's wire format.
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    /// Deserialize from bytes read from DHT.
    pub fn from_bytes(data: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(data)
    }
}
