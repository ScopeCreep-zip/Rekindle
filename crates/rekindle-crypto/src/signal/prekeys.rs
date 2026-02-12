//! `PreKey` generation and bundle creation for Signal Protocol.
//!
//! Pre-key bundles are published to DHT profile subkey 5 so that
//! new contacts can establish a Signal session asynchronously.

use serde::{Deserialize, Serialize};

/// A bundle of public keys published to DHT for session establishment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreKeyBundle {
    /// Ed25519 identity public key.
    pub identity_key: Vec<u8>,
    /// X25519 signed prekey (public).
    pub signed_prekey: Vec<u8>,
    /// Signature over the signed prekey by the identity key.
    pub signed_prekey_signature: Vec<u8>,
    /// Optional one-time prekey (consumed on first use).
    pub one_time_prekey: Option<Vec<u8>>,
    /// Registration ID for Signal Protocol.
    pub registration_id: u32,
}

// TODO: Implement prekey generation, bundle creation, and rotation
