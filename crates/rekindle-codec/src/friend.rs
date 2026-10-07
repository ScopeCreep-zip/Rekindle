//! The friend list's entry type, which `capnp_codec::friend` encodes
//! (moved from `rekindle_protocol::dht::friends`, plan C8).

use serde::{Deserialize, Serialize};

/// A single entry in the friend list DHT record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    /// Friend's Ed25519 public key (hex-encoded).
    pub public_key: String,
    /// Local nickname override.
    pub nickname: Option<String>,
    /// Group assignment (e.g., "Work", "Gaming").
    pub group: Option<String>,
    /// Unix timestamp when added.
    pub added_at: u64,
    /// Their profile DHT record key.
    pub profile_dht_key: Option<String>,
    /// `DhtLog` spine key for the per-peer DM conversation, created
    /// during friend accept. Both peers read and write it.
    #[serde(default)]
    pub dm_log_key: Option<String>,
}
