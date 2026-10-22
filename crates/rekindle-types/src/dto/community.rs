//! Community DTOs (plan E3.1 D1): the invites a community window lists and
//! the one it has just minted.

use serde::{Deserialize, Serialize};

/// A freshly minted invite. The raw code rides only inside `url`; the
/// network sees its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInviteCreated {
    /// The governance invite id, 32 lowercase hex: what a revoke names.
    pub invite_id: String,
    pub code_hash: String,
    /// The canonical `rekindle://invite/...` link.
    pub url: String,
    /// Unix seconds, when the invite stops admitting.
    pub expires_at: Option<u64>,
}

/// A live invite of a community, from merged governance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInviteView {
    /// The governance invite id, 32 lowercase hex: what a revoke names.
    pub invite_id: String,
    pub code_hash: String,
    /// The minter's community pseudonym, 64 lowercase hex.
    pub created_by: String,
    /// `None` is unlimited.
    pub max_uses: Option<u32>,
    /// Redemptions we counted; only the minter counts them, so another
    /// member's invite shows 0.
    pub uses: u32,
    /// Unix seconds.
    pub expires_at: Option<u64>,
    /// Milliseconds, when we minted it; `None` for another member's.
    pub created_at: Option<i64>,
    /// The invite link, for invites this identity minted.
    pub url: Option<String>,
}
