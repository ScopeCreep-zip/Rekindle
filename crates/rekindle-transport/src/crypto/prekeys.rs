//! PreKey bundle for Signal Protocol session establishment.
//!
//! Published to DHT profile subkey 5 so new contacts can establish
//! a Signal session asynchronously.

use serde::{Deserialize, Serialize};

/// A bundle of public keys published to DHT for session establishment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreKeyBundle {
    /// Ed25519 identity public key (or X25519 derived).
    pub identity_key: Vec<u8>,
    /// X25519 signed prekey (public half).
    pub signed_prekey: Vec<u8>,
    /// Ed25519 signature over the signed prekey bytes.
    pub signed_prekey_signature: Vec<u8>,
    /// Optional one-time prekey (consumed on first use).
    pub one_time_prekey: Option<Vec<u8>>,
    /// Registration ID for Signal Protocol.
    pub registration_id: u32,
}

impl PreKeyBundle {
    /// Serialize to bytes for DHT storage.
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    /// Deserialize from bytes read from DHT.
    pub fn from_bytes(data: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(data)
    }
}
