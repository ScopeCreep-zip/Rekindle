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
