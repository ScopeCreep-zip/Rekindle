//! Identity DTOs (plan E1.1): the login picker's list, the identity a create
//! or an unlock opened, and the unlocked identity's public data.

use serde::{Deserialize, Serialize};

/// One identity on this device, for the login picker (`IdentityList`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummary {
    pub public_key: String,
    pub display_name: String,
    /// Creation time, ms since the epoch.
    pub created_at: i64,
}

/// The identity an `IdentityCreate` or `Unlock` opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityView {
    pub public_key: String,
    pub display_name: String,
}

/// The unlocked identity's public data (`IdentityShow`, which answers
/// `Option<IdentityShown>`: `None` while locked).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityShown {
    pub public_key: String,
    pub display_name: String,
    pub profile_dht_key: String,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
    pub communities: usize,
}

/// This device's prekey record and its upkeep (`PrekeyStatus`, plan E2.4):
/// what the CLI's `keys prekeys status`, the debug `pqxdh_bundle_info` and
/// the status snapshot's `crypto.prekeys` check read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrekeyStatus {
    /// This device's id in the profile's device list (hex).
    pub device_id: Option<String>,
    /// The prekey record's key, once the unlock opened or created it.
    pub record_key: Option<String>,
    /// The signed prekey's id and age, before the first head exists `None`.
    pub spk_id: Option<u32>,
    pub spk_age_secs: Option<u64>,
    pub pq_lr_id: Option<u32>,
    pub lr_age_secs: Option<u64>,
    /// One-time slots holding unused keys, and the consumed one-time keys
    /// the 30-day archive still holds (a consumed slot is refilled in the
    /// commit that consumes it, so this counts uses, not empty slots).
    pub fresh_slots: u32,
    pub consumed_slots: u32,
    /// The published head's component sizes, in bytes.
    pub component_lens: Option<PrekeyComponentLens>,
}

/// The sizes of a published prekey head's parts: an X25519 key is 32 B,
/// an ML-KEM-768 key 1,184 B, a signature 64 B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrekeyComponentLens {
    pub spk: usize,
    pub spk_sig: usize,
    pub pq_lr: usize,
    pub pq_lr_sig: usize,
}
