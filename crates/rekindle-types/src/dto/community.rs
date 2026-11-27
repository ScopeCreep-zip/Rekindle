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
    /// The fingerprint the joiner's review shows for this invite, so the
    /// two can be compared out of band like a safety number (plan E7.15).
    pub fingerprint: String,
}

/// A community this identity just created (plan E3.1 D1b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityCreated {
    pub anchor: String,
    pub registry_key: String,
    pub name: String,
    /// The co-creators the creation still waits on (plan E3.2 S7s.3):
    /// non-empty while the records are reserved and the genesis is not
    /// written yet (`registry_key` is then empty).
    #[serde(default)]
    pub pending_co_creators: Vec<String>,
}

/// A successor community both sides link, as offered to this member (ADR
/// 0011 "Successor community", plan E3.2 S7s.4): the snapshot of
/// `GovernanceEvent::CommunitySuccessorOffered` a window opened after the
/// event reads (lead decision (33)). `signer` is the op's author and
/// `creators` the new community's creator set, hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessorOffer {
    pub community: String,
    pub new_anchor: String,
    pub signer: String,
    pub reason: crate::governance::SuccessorReason,
    pub creators: Vec<String>,
}

/// A transfer's link request asking this member (lead decision (46)): the
/// snapshot of `GovernanceEvent::SuccessorLinkRequested` a window opened
/// after the event reads. `request` is the op's hash and `recipient` the
/// offer's recipient, hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessorLinkRequested {
    pub community: String,
    pub request: String,
    pub recipient: String,
    pub new_anchor: String,
}

/// A community's successor snapshot (lead decision (33)): the successors
/// offered this session and the link requests asking us now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunitySuccessors {
    pub offers: Vec<SuccessorOffer>,
    pub link_requests: Vec<SuccessorLinkRequested>,
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
    pub my_role_ids: Vec<String>,
    /// What we may do, from the merged governance (plan E3.2 S9,
    /// architecture §9.2 `compute_permissions`): the base permission bits
    /// as a decimal `u64` string (the webview parses it with `BigInt`, exact
    /// past 2^53). The frontend reads these flags and computes no policy;
    /// `EffectivePermissionsChanged` naming us says to re-read them.
    pub my_permissions: String,
    /// Our effective permissions per channel (id hex): the base plus the
    /// channel's overwrites, decimal `u64` strings as `my_permissions`.
    pub my_channel_permissions: std::collections::BTreeMap<String, String>,
    /// Whether we hold a creator seat now (plan E3.2 S7s): in the pinned
    /// cert's creator set and not departed, from the merged governance.
    /// A change arrives as `EffectivePermissionsChanged` naming us, like
    /// the permission flags.
    pub we_are_creator: bool,
    /// The window's controls we may not use, each with the Host's words
    /// naming what grants it (a missing permission, our timeout's end, a
    /// creator seat); a capability we hold is absent (plan E3.2 S9). The
    /// window shows every control and disables these with the reason: it
    /// never hides one and never reads the permission bits for it. A change
    /// arrives on `EffectivePermissionsChanged` naming us, with the new map.
    pub denied: super::community_capability::DeniedCapabilities,
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
    /// Our name in this community as it renders, never empty (plan E3.4
    /// S6): our card's name, else our short pseudonym fingerprint; never
    /// the identity's name. Messages render with it before the member
    /// list loads.
    pub my_display_name: String,
    /// Our card's name as we set it, absent when unset: the set-name
    /// form's prefill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_card_name: Option<String>,
    /// Our card's custom status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_custom_status: Option<String>,
    /// Set when we were removed (plan E3.2 S9 R3-5): the community is
    /// read-only history, and the window names who removed us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<crate::display::CommunityRemovalDisplay>,
    /// Where each of our governance ops not yet landed stands (its hash,
    /// hex): what a window opened after their `Pending` events reads at
    /// load (ADR 0013: a new window hydrates, nothing is replayed); later
    /// changes arrive as `GovernanceEvent::OpPublication`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub op_publications:
        std::collections::BTreeMap<String, crate::subscription_events::OpPublicationState>,
    /// Whether this device may write here (ADR 0022 rule 4, lead decision
    /// 63): `NeedsReenrol` when it holds no key endorsed for our current
    /// member record (a fresh device, or one whose person re-joined from
    /// another device); the window offers `CommunityDeviceEnrol`.
    #[serde(default)]
    pub device_status: DeviceStatus,
    /// The unlock barrier's view while it holds our governance writes
    /// (plan E3.2 S7d-keys.8): what a window opened mid-barrier reads at
    /// load; later changes arrive as `GovernanceEvent::CatchUp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up: Option<crate::display_catch_up::CatchUpView>,
    /// Whether we hold the community key governance names (plan FX3
    /// follow-up KP): `Pending` while it is missing or behind, until a key
    /// holder answers our request; later changes arrive as
    /// `GovernanceEvent::KeyStatus`.
    #[serde(default)]
    pub key_status: KeyStatus,
}

/// This device's standing in a community ([`CommunityView::device_status`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceStatus {
    /// Our device key is endorsed for the current member record.
    #[default]
    Enrolled,
    /// Our writes are refused until this device enrols a fresh key.
    NeedsReenrol,
}

/// Whether a community key is held ([`KeyStatus::state`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyState {
    /// We hold the generation governance names.
    #[default]
    Current,
    /// The key is missing or behind: posts sealed under it stay unread
    /// until a key holder answers.
    Pending,
}

/// Our community key against the generation governance names, as the Host
/// publishes it ([`CommunityView::key_status`],
/// `GovernanceEvent::KeyStatus`). `have` is the generation we hold (none
/// before the first key), `need` the one governance names, `since_ms` when
/// it went `Pending` (unix ms; none while `Current`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub state: KeyState,
    pub have: Option<u64>,
    pub need: Option<u64>,
    pub since_ms: Option<u64>,
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
    /// We cannot read it (our effective permissions lack `VIEW_CHANNELS`):
    /// the window shows it locked with `locked_reason`, never its posts as
    /// undecryptable noise.
    #[serde(default)]
    pub locked: bool,
    /// The Host's words for why, empty when not locked.
    #[serde(default)]
    pub locked_reason: String,
    /// The channel's controls we may not use here, with the Host's words
    /// (plan E3.2 S9); a held one is absent. Re-read on
    /// `EffectivePermissionsChanged` naming us.
    #[serde(default)]
    pub denied: super::community_capability::ChannelDeniedCapabilities,
}

/// A channel category of a [`CommunityView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityCategoryView {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// A role tag's text colour on each skin base (plan E7.3, SC 1.4.3): the
/// role's own colour where it reaches 4.5:1 on the tag over every ground
/// a built-in skin of that base draws it on, else that colour moved in
/// Oklab lightness until it does (`rekindle_skin::role_tag`). `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleTagColours {
    pub dark: u32,
    pub light: u32,
}

/// A role of a [`CommunityView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityRoleView {
    /// The full 16-byte role id, 32 hex characters.
    pub id: String,
    pub name: String,
    pub color: u32,
    /// The name's readable text colour per base.
    pub tag_colours: RoleTagColours,
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
    /// Never empty (plan E3.4 S6): the member's per-community card name,
    /// else [`Self::fingerprint`]. Never an identity's name.
    pub display_name: String,
    /// The member's short pseudonym fingerprint (`"a1b2 c3d4"`), stated by
    /// the Host; a frontend never computes it.
    pub fingerprint: String,
    /// The custom status on the member's verified card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
    pub role_ids: Vec<String>,
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
    /// Each moderation action against this member: `None` when we may
    /// take it, else the Host's reason, as the governance validator decides
    /// it from the merged state (plan E3.2 S9); every one refused on our
    /// own row. The window shows every action and disables the refused.
    pub moderation: MemberModeration,
    /// Whether we outrank this member in the role hierarchy (the
    /// validator's rank rule): role changes and voice moderation need it.
    #[serde(default)]
    pub outranks: bool,
}

/// The moderation actions against one member, each the governance
/// validator's verdict on the entry it writes: `None` (wire `null`) when
/// allowed, else the Host's words for why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberModeration {
    /// `MemberRemoved`.
    pub kick: Option<String>,
    /// `BanEntry`.
    pub ban: Option<String>,
    /// `TimeoutEntry`.
    pub timeout: Option<String>,
    /// `RoleAssignment` / `RoleUnassignment` of a role the member cannot
    /// self-assign.
    pub manage_roles: Option<String>,
    /// Server mute (`MUTE_MEMBERS`, rank-strict); no governance entry.
    pub mute: Option<String>,
    /// Server deafen (`DEAFEN_MEMBERS`, rank-strict); no governance entry.
    pub deafen: Option<String>,
}

impl MemberModeration {
    /// Every action refused for one `reason`.
    #[must_use]
    pub fn refused(reason: &str) -> Self {
        Self {
            kick: Some(reason.to_owned()),
            ban: Some(reason.to_owned()),
            timeout: Some(reason.to_owned()),
            manage_roles: Some(reason.to_owned()),
            mute: Some(reason.to_owned()),
            deafen: Some(reason.to_owned()),
        }
    }
}

/// The member-list group a member falls under (Discord's member list): a
/// hoisted role the member holds while not offline, else `Online`, else
/// `Offline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MemberGroup {
    /// The member's highest hoisted role.
    Role { id: String, name: String },
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
    /// The Host's name for the author (plan E3.4 S6): its verified card's,
    /// else its short pseudonym fingerprint; never empty, never a raw key.
    pub actor_name: String,
    /// The subject of the entry (a member, channel, role, …), when it has one.
    pub target: Option<String>,
    /// The Host's name for the target, named as the author is; absent when
    /// the target is not a member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
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

/// A friend's co-creator invite we hold, unanswered or accepted and
/// waiting for its join link (plan E3.2 S7s.3g, lead decision (33)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldCreatorInvite {
    pub anchor: String,
    /// The creator, by identity key.
    pub peer_key: String,
    pub community_name: String,
}

/// One co-creator's state in a creation of ours, in the words of
/// `FriendEvent::CoCreatorStatus` (`pending`, `accepted`, `declined`,
/// `removed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCreatorState {
    pub peer_key: String,
    pub status: String,
}

/// A creation of ours still waiting on its co-creators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCoCreation {
    pub anchor: String,
    pub name: String,
    /// Every co-creator named, in the order named.
    pub co_creators: Vec<CoCreatorState>,
}

/// The co-creation state a window reads once its event stream is live
/// (plan E3.2 S7s.3g, the VI1 pattern): the held invites and our pending
/// creations. The friend events then keep it current.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCreationSnapshot {
    pub invites: Vec<HeldCreatorInvite>,
    pub creations: Vec<PendingCoCreation>,
}
