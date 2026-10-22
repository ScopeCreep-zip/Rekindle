//! The Host's community model (plan E3.1 A1): a joined community's state,
//! its channels, categories and roles, the gossip overlay and the members
//! the presence poll observed. One model for both modes: the GUI writes it
//! through `AppState.communities` (the Host's `Arc` in Shell mode) and the
//! daemon through [`CommunityStore`](super::store::CommunityStore).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

pub use rekindle_types::presence::OnlineMember;

/// A joined community's state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// adapter's `SegmentAdded` handler — see `rekindle_governance_runtime::segments`.
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

    /// Threads held at receipt because their creator was timed out then
    /// (`rekindle_channel::ingest::hold_timed_out_threads`): kept out of
    /// `governance_state` for the session.
    #[serde(skip)]
    pub held_threads: HashSet<rekindle_types::id::ThreadId>,

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
    pub record_sequences: HashMap<String, Vec<Option<u32>>>,

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

    /// Members' RSVPs per live event: rebuilt from governance on every
    /// merge (`GovernanceEntry::EventRsvp`, plan C7.15), with
    /// `EventRsvpChanged` gossip applied in between.
    #[serde(skip)]
    pub event_rsvps_by_event: HashMap<String, Vec<rekindle_types::event::EventRsvp>>,

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
    /// hex hash of the image, cached under the identity's keyed
    /// `state/{pk}.files/` (plan E3.1).
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

    /// The community's members as the presence poll last observed them,
    /// keyed by hex pseudonym: a validly signed `MemberPresence` whose
    /// author is neither banned nor departed. Runtime-derived (the registry
    /// is a presence directory, not a ledger); replaced wholesale per tick
    /// by [`CommunityStore::set_members`](super::store::CommunityStore::set_members),
    /// which keeps `known_members` equal to its keys.
    #[serde(skip)]
    pub members: HashMap<String, MemberRecord>,

    /// Channels whose initial history catch-up has completed.
    #[serde(skip)]
    pub synced_channels: BTreeSet<String>,
}

impl CommunityState {
    /// An empty community keyed by its governance key, for a store entry
    /// made before its first merge.
    #[must_use]
    pub fn new(governance_key: &str) -> Self {
        Self {
            id: governance_key.to_string(),
            governance_key: Some(governance_key.to_string()),
            ..Self::default()
        }
    }
}

/// One member as last seen in the registry.
#[derive(Debug, Clone, Default)]
pub struct MemberRecord {
    /// Peer-supplied. Untrusted display data — render escaped.
    pub display_name: Option<String>,
    pub subkey_index: u32,
    pub segment_index: u32,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
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

impl ChannelInfo {
    /// The channel as a `channels` row.
    #[must_use]
    pub fn row(&self) -> rekindle_db::repo::channels::StoredChannel<'_> {
        rekindle_db::repo::channels::StoredChannel {
            id: &self.id,
            name: &self.name,
            channel_type: self.channel_type.as_str(),
            category_id: self.category_id.as_deref(),
            topic: &self.topic,
            slowmode_seconds: self.slowmode_seconds.unwrap_or(0),
            nsfw: self.nsfw,
            message_record_key: self.message_record_key.as_deref(),
            mek_generation: self.mek_generation,
            parent_voice_channel_id: self.parent_voice_channel_id.as_deref(),
        }
    }
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

/// The channel kind (Tier 1 `ChannelKind`), under the name the desktop's
/// call sites use. Written to `channels` through `.as_str()` and read back
/// with `.parse()` (the rusqlite traits cannot be implemented here: orphan
/// rule).
pub use rekindle_types::channel::ChannelKind as ChannelType;

/// Gossip overlay state for a community.
///
/// Each member maintains a random peer set of D online members and forwards
/// received messages to them. Adaptive degree:
/// - ≤20 members: D = N-1 (direct mesh)
/// - 21-60: D = 6, 61+: D = 8
#[derive(Debug, Clone)]
pub struct GossipOverlay {
    /// Current gossip peers: pseudonym_key → member info.
    /// These are the D peers we send/forward every message to.
    pub peers: HashMap<String, OnlineMember>,
    /// All online members: pseudonym_key → member info.
    /// Superset of `peers`. Updated on each presence poll.
    pub online_members: HashMap<String, OnlineMember>,
    /// True until the first successful sync after coming online.
    /// Used to trigger a `SyncRequest` to online peers for catch-up.
    pub needs_initial_sync: bool,
    /// A1/P4.1 — broadcasts queued because `peers` was empty at send time.
    /// `send_to_mesh_raw` enqueues here instead of dropping; the next
    /// presence poll that lands online peers drains the queue and re-sends.
    /// Bounded at 100 (oldest dropped) so an offline burst doesn't OOM.
    /// Without this, the first member of a fresh community broadcast all
    /// their join announcements / MEK requests / governance updates to a
    /// zero-peer mesh — silently lost.
    pub pending_mesh_broadcasts: VecDeque<rekindle_codec::community::envelope::SignedEnvelope>,
}

impl Default for GossipOverlay {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            online_members: HashMap::new(),
            needs_initial_sync: true,
            pending_mesh_broadcasts: VecDeque::with_capacity(16),
        }
    }
}
