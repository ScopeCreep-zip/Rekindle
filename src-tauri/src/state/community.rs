//! Phase 23.B — extracted from `state.rs`. Per-community state +
//! sub-DTOs (channels, categories, roles, RSVPs, profile snapshots).

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::gossip::GossipOverlay;

/// A joined community's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityState {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfo>,
    pub categories: Vec<CategoryInfo>,
    /// Our role IDs in this community (multi-role, bitmask-based).
    pub my_role_ids: Vec<u32>,
    /// Cached role definitions from merged governance state.
    pub roles: Vec<RoleDefinition>,
    /// Owner keypair for the community DHT record (Veilid `KeyPair::to_string()` format).
    /// Required to open the record with write access.
    pub dht_owner_keypair: Option<String>,
    /// Our pseudonym pubkey hex for this community.
    pub my_pseudonym_key: Option<String>,
    /// Current MEK generation we have.
    pub mek_generation: u64,
    /// DHT member registry record key (SMPL, multi-writer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_registry_key: Option<String>,
    /// Our subkey index in the member registry SMPL record.
    /// **Local** to our segment's record (0..255). The corresponding global
    /// slot index used for slot-keypair derivation is
    /// `my_segment_index * 255 + my_subkey_index` (architecture §15.2 +
    /// §8.3). For segment 0 these are equal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_subkey_index: Option<u32>,
    /// Plate Gate (architecture §15): which segment hosts our slot. `None`
    /// for legacy / unknown; 0 for the genesis segment; 1..=MAX_SEGMENTS
    /// for each expansion. Persisted to SQLite, restored on login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_segment_index: Option<u32>,
    /// Plate Gate (architecture §15): every expansion segment merged from
    /// governance state (segment 0 is implicit, not included here).
    /// Populated on community load and kept current by the governance
    /// adapter's `SegmentAdded` handler — see `governance_adapter::segments`.
    #[serde(default)]
    pub segments: Vec<rekindle_types::presence::SegmentDescriptor>,
    // ── v2.0 flat governance fields ──
    /// DHT key of the SMPL governance record (o_cnt:0).
    /// This is the canonical community identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_key: Option<String>,

    /// Cached CRDT-merged governance state.
    /// Computed by `rekindle_governance::merge::merge()`.
    #[serde(skip)]
    pub governance_state: Option<rekindle_governance::state::GovernanceState>,

    /// Lamport clock for channel and control messages (text, polls,
    /// stage, files, video topology). Ticked on every send, merged
    /// (drift-clamped) on every received message. Loaded at login from
    /// `MAX(messages.lamport_ts)`.
    #[serde(skip)]
    pub message_clock: u64,
    /// Lamport clock for the `GovernanceEntry`s we write. Ticked per
    /// entry, raised by governance rebuilds to what the accepted entries
    /// justify. Persisted as `communities.lamport_clock`.
    #[serde(skip)]
    pub governance_clock: u64,

    // ── Gossip mesh fields (Phase 2) ──
    /// Gossip overlay state (peer set, online members).
    /// `None` until the presence poll loop initializes it.
    #[serde(skip)]
    pub gossip: Option<GossipOverlay>,

    /// Our slot keypair string for writing presence to the SMPL registry.
    /// Veilid `KeyPair::to_string()` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_keypair: Option<String>,

    /// Channel DHTLog record keys: channel_id → log spine key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_log_keys: HashMap<String, String>,

    /// Registry segment owner keypair for writing member index/MEK vault.
    /// Veilid `KeyPair::to_string()` format. Required to add/remove members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_owner_keypair: Option<String>,

    /// Slot seed for deriving member SMPL keypairs.
    /// Distributed to ALL members via JoinAccepted (same trust level as MEK).
    /// 32 bytes, hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_seed: Option<String>,

    /// In-memory cache of known member pseudonym keys.
    /// Populated from SQLite on login, updated on MemberJoined/MemberRemoved/MemberLeave.
    /// Used for fast membership checks on incoming community gossip envelopes.
    #[serde(skip)]
    pub known_members: HashSet<String>,

    /// Cached member role_ids: pseudonym_key → role_ids.
    /// Populated during presence_poll_tick from the member index.
    /// Used for permission checks on incoming gossip moderation payloads.
    #[serde(skip)]
    pub member_roles: HashMap<String, Vec<u32>>,

    /// Per-channel message sequence counter (channel_id → next sequence).
    /// Incremented for each message we send in a channel. Persisted to SQLite.
    #[serde(skip)]
    pub channel_sequences: HashMap<String, u64>,

    /// Pending sync requests: channel_id → (request_timestamp, attempt_count).
    /// Cleared when SyncResponse arrives. Retried in presence_poll_tick if stale.
    #[serde(skip)]
    pub pending_syncs: HashMap<String, (u64, u32)>,

    /// Record keys with an active Veilid watch in this session.
    /// Watches are an optimization; inspect polling still runs for all records.
    #[serde(skip)]
    pub watched_records: HashSet<String>,

    /// Last known network sequence numbers per record key, used by the 60-second
    /// inspect loop to detect changed subkeys without fetching the entire record.
    #[serde(skip)]
    pub record_sequences: HashMap<String, Vec<veilid_core::ValueSeqNum>>,

    /// Per-sender per-channel sequence tracking for gap detection (Briar-inspired).
    /// Key: (sender_pseudonym, channel_id), Value: last received sequence number.
    #[serde(skip)]
    pub peer_sequences: HashMap<(String, String), u64>,

    /// Architecture §28.7 slowmode: per-channel timestamp (ms) of our
    /// last successful send. Compared against the channel's
    /// `slowmode_seconds` to gate further writes. In-memory only —
    /// slowmode applies to the active session, not across restarts.
    #[serde(skip)]
    pub channel_last_send_at: HashMap<String, i64>,

    /// Mutual Aid topology metrics (architecture §14.5): per-peer
    /// (success_count, failure_count) over the lifetime of this session.
    /// Used to weight gossip fan-out selection — the highest-reliability
    /// peers ("ziplines") emerge organically from usage. Pure in-memory;
    /// not persisted across restarts.
    #[serde(skip)]
    pub peer_reliability: HashMap<String, (u32, u32)>,

    /// This community's tasks (presence poll, keepalive, inspect,
    /// history catch-up): a child of the login scope, shut down when the
    /// member leaves or the session ends (plan C4).
    #[serde(skip)]
    pub tasks: Option<std::sync::Arc<rekindle_lifecycle::SessionScope>>,

    /// Tracks all DHT records opened for this community (VeilidChat-inspired lifecycle).
    /// Records are opened once during join and kept open until leave/logout.
    /// Prevents "record not open" errors and ensures proper cleanup.
    #[serde(skip)]
    pub open_community_records: CommunityRecords,

    /// The records this community holds for the session, in the record
    /// pool (plan C7.5): released on leave, closed by logout's pool
    /// shutdown. See `services::community::leases`.
    #[serde(skip)]
    pub leases: rekindle_records::lease::CommunityLeases,

    /// Whether the community's inspect, keepalive and presence loops have
    /// started (once per session, from `leases::records_ready`).
    #[serde(skip)]
    pub loops_started: bool,

    /// Our locally persisted RSVPs for scheduled events.
    #[serde(skip)]
    pub my_event_rsvps: HashMap<String, String>,

    /// Reader-aggregated RSVPs discovered from member presence records.
    #[serde(skip)]
    pub event_rsvps_by_event: HashMap<String, Vec<EventRsvpEntry>>,

    /// Whether our local member record has completed onboarding for this community.
    #[serde(skip)]
    pub onboarding_complete: bool,

    /// Per-community profile bio (≤190 chars). Same identity, different
    /// persona per community — the value is propagated to peers via the next
    /// presence write. Local-only state; on restart it resets to None and
    /// the user re-edits it from the popup.
    #[serde(skip)]
    pub my_bio: Option<String>,

    /// Per-community profile pronouns (≤40 chars per architecture §24.2).
    /// See `my_bio`.
    #[serde(skip)]
    pub my_pronouns: Option<String>,

    /// Per-community profile theme color (0xRRGGBB). See `my_bio`.
    #[serde(skip)]
    pub my_theme_color: Option<u32>,

    /// Per-community profile badges (≤8 entries, each ≤32 chars). See `my_bio`.
    #[serde(skip)]
    pub my_badges: Vec<String>,

    /// Per-community avatar content reference (BLAKE3 hex hash of the
    /// image stored as a Lost Cargo expression asset). Architecture
    /// §24.2 + §32 Week 15.
    #[serde(skip)]
    pub my_avatar_ref: Option<String>,

    /// Per-community banner content reference (BLAKE3 hex hash). Same
    /// caching model as `my_avatar_ref`.
    #[serde(skip)]
    pub my_banner_ref: Option<String>,

    /// Architecture §32 Phase 5 Week 15 — community-level icon (the
    /// avatar shown in the buddy list / community switcher). BLAKE3
    /// hex hash of the WebP-compressed image cached at
    /// `<app_data>/community_avatars/<community_id>/<hash>.webp`.
    /// Synced from `governance.metadata.icon_hash` when CRDT state
    /// is rebuilt; persisted to the `communities.icon_hash` column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_hash: Option<String>,

    /// Architecture §32 Phase 5 Week 15 — community-level banner.
    /// Same caching model as `icon_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_hash: Option<String>,

    /// Reader-aggregated profile snapshots (peer pseudonym → profile fields).
    /// Populated by `presence_poll_tick` from peers' presence subkeys.
    #[serde(skip)]
    pub member_profiles: HashMap<String, MemberProfileSnapshot>,

    /// Architecture §20.6 raid detector — sliding window of recent
    /// (timestamp_secs, pseudonym_hex) join observations. Bounded to the
    /// policy window length when entries are inserted.
    #[serde(skip)]
    pub recent_member_joins: VecDeque<(u64, String)>,

    /// Where the local user is currently focused in this community
    /// (text or voice channel). Drives `MemberSession.location` on the
    /// next presence write. Local-only and reset on restart — the
    /// frontend re-asserts it on channel open, like `my_bio`.
    #[serde(skip)]
    pub my_session_location: Option<rekindle_types::presence::SessionLocation>,

    /// Per-community presence sharing policy (default-deny). Local-only
    /// consent state — **never** published to the registry. Applied at
    /// write time (redact what isn't shared) and read time (drop peer
    /// signals we don't reciprocate). Persisted to SQLite alongside the
    /// other per-community prefs.
    #[serde(default)]
    pub presence_policy: rekindle_types::presence::PresenceSharingPolicy,
}

/// Tracks DHT records opened for a single community.
///
/// Every DHT record open for one community.
///
/// Declared by `rekindle-records`, whose stated job is "DHT record
/// lifecycle" (CLAUDE.md, Tier 3). This file used to carry a second
/// `CommunityRecords` that had grown the fields the lifecycle actually
/// needs while the crate's sat unused with a speculative
/// `HashMap<channel_id, record_key>`. The fields came here; the type
/// went there.
pub use rekindle_records::lifecycle::CommunityRecords;

/// Aggregated RSVP entry for a single member and event.
//
// Declared by `rekindle-presence`, which owns RSVP aggregation
// (`community::rsvp_aggregate`). The copy here was byte-identical.
pub use rekindle_presence::EventRsvpEntry;

/// Per-community profile snapshot aggregated from a peer's presence
/// subkey.
///
/// Declared by `rekindle-presence`, which owns the diffing that produces
/// it (`community::profile_diff`). The copy here had the same eight
/// fields and the same derives — `get_community_members` joins the map
/// with SQLite membership rows so the popup can render `bio` /
/// `pronouns` / `theme_color` / `badges`.
pub use rekindle_presence::MemberProfileSnapshot;

/// A role definition cached from merged governance state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDefinition {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub self_assignable: bool,
    /// Architecture §19.4 — when set, the member can hold at most one
    /// role per group (CRDT-enforced; the higher-Lamport assignment
    /// wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
}

impl RoleDefinition {
    /// Convert from the protocol's `RoleDto`.
    pub fn from_dto(dto: &rekindle_codec::message::RoleDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name.clone(),
            color: dto.color,
            permissions: dto.permissions,
            position: dto.position,
            hoist: dto.hoist,
            mentionable: dto.mentionable,
            self_assignable: dto.self_assignable,
            exclusion_group: None,
        }
    }
}

/// Compute the display name for the highest-positioned role from a set of role IDs.
pub fn display_role_name(role_ids: &[u32], roles: &[RoleDefinition]) -> String {
    match role_ids
        .iter()
        .filter_map(|id| roles.iter().find(|r| r.id == *id))
        .max_by_key(|r| r.position)
    {
        Some(r) => r.name.clone(),
        None => "member".to_string(),
    }
}

/// Channel info within a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub unread_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forum_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_speakers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_moderator: Option<String>,
    #[serde(default)]
    pub slowmode_seconds: Option<u32>,
    /// Whether this channel is marked NSFW.
    #[serde(default)]
    pub nsfw: bool,
    /// DHT record key for this channel's per-channel message record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_record_key: Option<String>,
    /// Current MEK generation for this channel.
    #[serde(default)]
    pub mek_generation: u64,
    /// Local notification preference for this channel.
    #[serde(default = "default_notification_level")]
    pub notification_level: String,
    /// Architecture §32 Phase 7 Week 25 — channel-level notification
    /// sound override (BLAKE3 content hash of a soundboard expression).
    /// `None` means inherit from the community default; the resolver
    /// in `services/community/notifications.rs::resolve_notification_sound`
    /// performs the channel → community-default → app-default cascade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_sound_ref: Option<String>,
    /// Architecture §10.8 — text-in-voice. Hex channel id of the
    /// parent voice channel when this channel is the text companion of
    /// a voice channel; `None` for normal channels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_voice_channel_id: Option<String>,
}

fn default_notification_level() -> String {
    "all".to_string()
}

/// Category info within a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// Canonical definition moved to Tier 1
/// (`rekindle_types::channel::ChannelKind`) — this was a byte-identical
/// second enum (same variants, same lowercase serde form) plus a
/// `From<ChannelKind> for ChannelType` conversion that existed only to
/// bridge the two and had no callers (confirmed by grep before
/// deleting it: nothing in src-tauri ever constructed a `ChannelKind`
/// to convert from). Aliased under this name so the ~12 existing call
/// sites in this crate don't need to change.
pub use rekindle_types::channel::ChannelKind as ChannelType;
// `Display`/`FromStr` live on the canonical type now (same behavior:
// this crate's old `FromStr` was infallible, silently mapping any
// unrecognized string to `Text`; the canonical one is fallible, and
// every caller here already does `.parse().unwrap_or(ChannelType::Text)`
// — same net result). `ToSql`/`FromSql` can't move: implementing a
// foreign trait (`rusqlite`'s) for a foreign type (`ChannelKind`, now
// owned by `rekindle-types`, which the DB-storage traits don't belong
// in anyway per the Tier 1 dependency rules) is an orphan-rule
// violation. The two write sites (`channel_repo.rs`) go through
// `.as_str()` and the one read site (`community_loader/rows.rs`) reads
// a `String` and `.parse()`s it instead.
