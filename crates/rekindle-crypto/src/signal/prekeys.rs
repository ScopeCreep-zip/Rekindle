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
//! valid across restarts) and only mints fresh keys when Stronghold is
//! empty.

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
