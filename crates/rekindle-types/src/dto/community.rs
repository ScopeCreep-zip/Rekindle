//! Community DTOs (plan E3.1 D1): the invites a community window lists and
//! the one it has just minted; the communities it lists and their members
//! (D1b).

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

/// A community this identity just created (plan E3.1 D1b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityCreated {
    pub governance_key: String,
    pub registry_key: String,
    pub name: String,
}

/// A community this identity just joined from an invite (plan E3.1 D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityJoined {
    pub governance_key: String,
    pub name: String,
    /// The registry segment our claimed slot is in.
    pub segment: u32,
    /// Our slot in that segment.
    pub slot: u32,
    /// The members the join's registry scan found.
    pub known_members: u32,
}

/// A joined community as its window renders it: channels, categories,
/// roles and our own standing in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// BLAKE3 hex of the cached icon image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_hash: Option<String>,
    pub channels: Vec<CommunityChannelView>,
    pub categories: Vec<CommunityCategoryView>,
    pub my_role: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub roles: Vec<CommunityRoleView>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub member_registry_key: Option<String>,
    pub governance_key: Option<String>,
    pub onboarding_complete: bool,
    /// Our per-community profile, for prefilling the edit form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_pronouns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub my_badges: Vec<String>,
}

/// A channel of a [`CommunityView`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityChannelView {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub unread_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forum_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_speakers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_moderator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowmode_seconds: Option<u32>,
    pub notification_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_sound_ref: Option<String>,
}

/// A channel category of a [`CommunityView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityCategoryView {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// A role of a [`CommunityView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityRoleView {
    pub id: u32,
    pub name: String,
    pub color: u32,
    /// The permission bits in decimal: a JSON number loses the bits above
    /// 2^53.
    pub permissions: String,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
}

/// A member of a community as its member list renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityMemberView {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub display_role: String,
    /// `online`, `away`, `busy` or `offline`.
    pub status: String,
    /// Unix seconds.
    pub timeout_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_ref: Option<String>,
    /// Where the member is focused, if they share it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<crate::presence::SessionLocation>,
    /// Last-active unix seconds, coarsened by the member's policy (0 =
    /// not shared).
    #[serde(default)]
    pub last_active: u64,
}
