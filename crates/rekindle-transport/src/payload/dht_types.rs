//! DHT record value types — structured data stored in DHT subkeys.
//!
//! Every DHT subkey value is frame-encoded (version + type + length + payload)
//! before writing, ensuring all stored data is self-describing and versioned.

// ── Profile record ──────────────────────────────────────────────────

// The profile layout lives in `rekindle_types::dht_layout::profile` —
// the desktop track indexes the same records, and keeping a private copy
// here is how the two ended up disagreeing about subkey 8 (relay pool
// vs the old friend-inbox key). These are aliases so existing call sites keep
// reading naturally.
pub use rekindle_types::dht_layout::profile::{
    AVATAR as PROFILE_SUBKEY_AVATAR, DISPLAY_NAME as PROFILE_SUBKEY_DISPLAY_NAME,
    GAME_INFO as PROFILE_SUBKEY_GAME_INFO, METADATA as PROFILE_SUBKEY_METADATA,
    ROUTE_BLOB as PROFILE_SUBKEY_ROUTE_BLOB, STATUS as PROFILE_SUBKEY_STATUS, STATUS_AWAY,
    STATUS_BUSY, STATUS_MESSAGE as PROFILE_SUBKEY_STATUS_MESSAGE, STATUS_OFFLINE, STATUS_ONLINE,
    SUBKEY_COUNT as PROFILE_SUBKEY_COUNT,
};
pub const STATUS_INVISIBLE: u8 = 4;

// ── Friend list record (DFLT, 1 subkey) ─────────────────────────────
//
// Re-exported from `rekindle-protocol` rather than declared here. The
// two crates had their own `FriendEntry`/`FriendList` for the *same*
// DHT record, and they had already diverged. Worse, the two encoded that one
// record differently — protocol in Cap'n Proto, this crate in postcard
// — so neither track could read the other's friend list, and whichever
// wrote last destroyed it. Cap'n Proto is the declared serialization
// (CLAUDE.md, `schemas/friend.capnp`), so that is the one that stayed.
pub use rekindle_codec::friend::FriendEntry;
pub use rekindle_protocol::dht::friends::FriendList;

// ── Mailbox record (DFLT, 1 subkey) ──────────────────────────────────

// Mailbox / manifest / registry subkey layout.
//
// Declared once in `rekindle_types::dht_layout`; this crate previously
// kept its own copy of each table. Eight v1.0 owner-block constants
// (REGISTRY_POLICY, _SCHEMA_VERSION, _OPS_LOG and the five RESERVED_*)
// went with the o_cnt:0 migration — they had no readers.
pub use rekindle_types::dht_layout::channel::{
    MEMBER_SUBKEY_COUNT as CHANNEL_MEMBER_SUBKEY_COUNT,
    OWNER_SUBKEY_COUNT as CHANNEL_OWNER_SUBKEY_COUNT,
};
pub use rekindle_types::dht_layout::mailbox::{
    ROUTE_BLOB as MAILBOX_SUBKEY_ROUTE_BLOB, SUBKEY_COUNT as MAILBOX_SUBKEY_COUNT,
};
pub use rekindle_types::dht_layout::manifest::{
    AUDIT_LOG_KEY as MANIFEST_AUDIT_LOG_KEY, AUTOMOD as MANIFEST_AUTOMOD, BANS as MANIFEST_BANS,
    CATEGORIES as MANIFEST_CATEGORIES, CHANNELS as MANIFEST_CHANNELS,
    COORDINATOR as MANIFEST_COORDINATOR, INVITES as MANIFEST_INVITES,
    METADATA as MANIFEST_METADATA, ONBOARDING as MANIFEST_ONBOARDING,
    POLICIES as MANIFEST_POLICIES, REGISTRY_SPINE_V1 as MANIFEST_REGISTRY_SPINE,
    ROLES as MANIFEST_ROLES, SUBKEY_COUNT as MANIFEST_SUBKEY_COUNT, WELCOME as MANIFEST_WELCOME,
};

/// Maximum member slots per registry segment.
///
/// Re-exported from the desktop track so both agree by construction.
/// This was locally defined as 245 ("256 total - 11 owner subkeys"),
/// the v1.0 derivation — under `o_cnt: 0` there are no owner subkeys
/// and the value is 255, which is what every record is actually built
/// with.
pub use rekindle_types::dht_layout::SLOTS_PER_SEGMENT;
pub use rekindle_types::dht_layout::SLOTS_PER_SEGMENT as REGISTRY_MAX_MEMBERS;

// ── Community metadata ──────────────────────────────────────────────
//
// `JoinPolicy` + `CommunityMetadata` lived here: the v1.0
// governance-manifest metadata subkey, carrying `owner_pseudonym`,
// `operator_pseudonyms`, `community_mailbox_key`, `join_inbox_key`
// and its published keypair. Every one of those is a coordinator
// concept flat governance removed, and the struct had no reader left
// on either track — `GovernanceState.metadata` is the community's
// name and description now, merged from `CommunityMeta` entries that
// each peer validates for itself.

// ── Channel types ───────────────────────────────────────────────────
//
// `ChannelEntry`, `ChannelKind`, `CategoryEntry`, `BanEntry` and
// `RoleEntry` lived here: the v1.0 governance-manifest payload types.
// Nothing has written that manifest since channels, roles, categories
// and bans became `ChannelCreated` / `RoleDefinition` / `CategoryAdded`
// / `MemberBanned` governance entries, and the caller builds display
// types from merged CRDT state. They were also
// five of the eighteen type names this crate duplicated against
// rekindle-protocol.

// ── Role types ──────────────────────────────────────────────────────

// ── Member types ────────────────────────────────────────────────────

// `MemberPresence` lived here with no reader — the third copy of that
// name this migration has removed. The live type is
// `rekindle_types::presence::MemberPresence`, which this crate's own
// `operations/community/leave.rs` already imports directly.

// ── Ban types ───────────────────────────────────────────────────────

// ── Invite types ────────────────────────────────────────────────────

// ── Channel message record ──────────────────────────────────────────

/// A message entry written to a channel record subkey.
///
/// Re-exported from the desktop track, which owns the definition. This
/// crate declared a second one that diverged in two ways, both of which
/// broke cross-track reads outright:
///
///   * `ciphertext` had no `#[serde(with = "base64_bytes")]`, so it
///     serialized as a JSON number array where the desktop track writes
///     a base64 string — a daemon-written message failed to parse there
///     with `invalid type: sequence, expected a string`.
///   * it lacked `attachment`, `flags`, `mentioned_pseudonyms` and
///     `mentioned_roles`, silently dropping file offers, voice-message
///     and @everyone/@here flags, and mention routing on any round trip.
pub use rekindle_codec::community::channel_record::ChannelMessage;
