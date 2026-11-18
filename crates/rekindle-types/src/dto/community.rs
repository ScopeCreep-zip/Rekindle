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
    /// Redemptions allowed, `1..=100`: every reader refuses an invite
    /// outside it (plan E3.2 (d)), so none is unlimited.
    pub max_uses: u32,
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
    pub anchor: String,
    pub registry_key: String,
    pub name: String,
}

/// A community this identity just joined from an invite (plan E3.1 D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityJoined {
    pub anchor: String,
    pub name: String,
    /// The registry segment our claimed slot is in.
    pub segment: u32,
    /// Our slot in that segment.
    pub slot: u32,
    /// The members the join's registry scan found.
    pub known_members: u32,
    /// Our `MemberJoined` is applied, gossiped and queued but not yet on
    /// our member record (plan E3.2 S6b, Matrix's local echo): it lands
    /// without a retry, announced by `GovernanceEvent::OpPublication`.
    #[serde(default)]
    pub pending: bool,
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
    pub anchor: Option<String>,
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
    /// Set when we were removed (plan E3.2 S9 R3-5): the community is
    /// read-only history, and the window names who removed us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<crate::display::CommunityRemovalDisplay>,
}

/// One channel's unread messages and mentions after our read marker
/// (plan E3.1, D25; `UnreadCounts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelUnreadView {
    pub channel_id: String,
    pub unread_count: u32,
    pub mention_count: u32,
}

/// A channel of a [`CommunityView`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityChannelView {
    pub id: String,
    pub name: String,
    pub channel_type: String,
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
    pub position: u32,
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
    /// The member-list group this row falls under, decided by the Host
    /// ([`super::member_groups::group_members`]); every frontend renders it.
    pub group: MemberGroup,
}

/// The member-list group a member falls under (Discord's member list): a
/// hoisted role the member holds while not offline, else `Online`, else
/// `Offline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MemberGroup {
    /// The member's highest hoisted role.
    Role { id: u32, name: String },
    /// Present (online, away or busy) with no hoisted role.
    Online,
    /// Offline, whatever their roles.
    Offline,
}

/// A community's policy (architecture §10.7, §20.6): the rules text and the
/// raid thresholds, the defaults until a `CommunityPolicy` entry merges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityPolicyView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

/// A banned member of a community, from merged governance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityBanView {
    /// 64 lowercase hex.
    pub pseudonym_key: String,
    /// The roster's name, or a short form of the key.
    pub display_name: String,
}

/// One report in a community's moderator queue (plan E3.1, D26), from
/// merged governance: a reported message (`channel_id` and `message_id`)
/// or a reported member (`member`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModReportView {
    /// 32 lowercase hex.
    pub report_id: String,
    /// The reporter's pseudonym, 64 lowercase hex.
    pub reporter: String,
    pub channel_id: Option<String>,
    pub message_id: Option<String>,
    pub member: Option<String>,
    pub reason: String,
    pub lamport: u64,
}

/// One row of a community's audit log: a verified governance entry, by its
/// author (plan E3.1 F4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityAuditEntry {
    /// What the entry did, in snake case (`member_banned`, `role_assigned`, …).
    pub action: String,
    /// The author's pseudonym, 64 lowercase hex.
    pub actor_pseudonym: String,
    /// The subject of the entry (a member, channel, role, …), when it has one.
    pub target: Option<String>,
    /// The entry's other fields, when it carries any.
    pub details: Option<String>,
    /// The entry's Lamport time.
    pub timestamp: u64,
}

/// One automod rule, as the moderators' settings list it (plan E3.1 D26).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModRuleView {
    pub rule_id: String,
    pub name: String,
    pub enabled: bool,
    pub keywords: Vec<String>,
    pub regex_patterns: Vec<String>,
    pub action: crate::governance::AutoModAction,
    pub lamport: u64,
}

/// One custom expression of a community (architecture §18, plan E3.1
/// E3c), its picture or sound inline when cached and re-checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionInfo {
    /// 16 bytes of hex.
    pub expression_id: String,
    pub name: String,
    /// `emoji`, `sticker` or `soundboard`.
    pub kind: String,
    pub content_hash: String,
    /// A `data:` URL built from an allowlisted, decoded media type.
    pub inline_data_url: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Present only on a soundboard sound.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sound_meta: Option<crate::expression::SoundboardMeta>,
    /// The uploader's pseudonym (hex).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub creator_pseudonym: Option<String>,
    /// Seconds at upload.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<u64>,
    /// Gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
}

/// One pinned channel message (architecture §16.4, plan E3.1 E3b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageView {
    pub message_id: String,
    pub channel_id: String,
    /// The pinner's pseudonym (hex).
    pub pinned_by: String,
    /// Seconds, as the receiving peer stored the pin.
    pub pinned_at: u64,
}
