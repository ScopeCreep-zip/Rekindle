//! Session state carried inside a `MemberPresence` row.
//!
//! Split out of `presence.rs` when that file crossed the 600-line
//! ceiling. The seam is real rather than arbitrary: everything here
//! describes *this heartbeat* — status, focus, activity, and the
//! sharing policy that decides which of those a peer is allowed to see
//! — while the parent module describes the registry row that carries
//! them.

use serde::{Deserialize, Serialize};

/// Typed session state. Decoupled from routing — a session is "live"
/// on a fresh heartbeat regardless of `route_blob`. This is the local
/// working form; on the wire `location`/`activity` are stripped from
/// the plaintext `session` field and carried encrypted instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemberSession {
    /// Presence status. Typed, not free-string.
    pub status: SessionStatus,
    /// Where the member is focused right now (text or voice channel).
    /// Stripped from the plaintext wire form (→ `SessionExtras`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SessionLocation>,
    /// Coarse activity label ("Playing Halo"), policy-gated at write
    /// time. Stripped from the plaintext wire form (→ `SessionExtras`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Unix seconds of the last user interaction. Drives last-seen
    /// bucketing on the read side; already coarsened per policy.
    #[serde(default)]
    pub last_active: u64,
}

/// Presence status — mirrors the identity-level `UserStatus` vocabulary
/// so friends and community presence speak one language. Mapped at the
/// src-tauri adapter boundary (this crate has no `UserStatus`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    #[default]
    Online,
    Away,
    Busy,
    Offline,
    /// Appear offline to peers but stay functionally connected.
    Invisible,
}

/// Architecture §13.4 — "invisible" appears offline to others, so the
/// wire payload uses `"offline"` for both Invisible and Offline.
///
/// Named rather than inlined because the fold is a *privacy* rule, not a
/// formatting choice: anything that writes a status onto the wire has to
/// collapse Invisible, and a bare `"offline"` literal at a new callsite
/// is indistinguishable from a genuine offline.
pub const INVISIBLE_WIRE_VALUE: &str = "offline";

impl SessionStatus {
    /// Wire string used by the transitional `MemberPresence.status`
    /// field (Invisible folds to "offline" — peers must not see it).
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Offline | Self::Invisible => INVISIBLE_WIRE_VALUE,
        }
    }

    /// `true` when peers should consider this status "available"
    /// (receive routing decisions + mention escalation).
    #[must_use]
    pub fn is_visible_online(self) -> bool {
        matches!(self, Self::Online | Self::Away | Self::Busy)
    }

    /// `true` when the user is actively present at the keyboard
    /// (presence indicators flash, notifications can wake them).
    #[must_use]
    pub fn is_actively_engaged(self) -> bool {
        matches!(self, Self::Online | Self::Busy)
    }
}

/// Where a member is focused right now. One field for both text and
/// voice so the roster aggregates location uniformly. `channel_id` is
/// the string id the frontend + community wire envelopes already use
/// (hex of the 16-byte UUID) — no byte/string conversion at the hops.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SessionLocation {
    Text { channel_id: String },
    Voice { channel_id: String },
}

impl SessionLocation {
    /// The channel id regardless of text/voice kind.
    #[must_use]
    pub fn channel_id(&self) -> &str {
        match self {
            Self::Text { channel_id } | Self::Voice { channel_id } => channel_id,
        }
    }
}

/// The identity-revealing slice of a session — encrypted under the MEK
/// so only current members can read it (ex-members lose access at the
/// next rotation, DHT observers see opaque ciphertext).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionExtras {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SessionLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
}

/// MEK-encrypted wire form for [`SessionExtras`]. Same shape as
/// [`EncryptedHistoryRanges`]; `ciphertext` is over a JSON `SessionExtras`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedSessionExtras {
    pub mek_generation: u64,
    pub ciphertext: Vec<u8>,
}

/// Per-signal presence consent. Default-deny: a member is online by
/// default (the baseline community signal) but shares no location,
/// activity, or last-seen until they opt in. Local-only — NEVER written
/// to the registry (it's the user's private consent, not peer-visible).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceSharingPolicy {
    /// Master switch: appear online at all. Off ⇒ Invisible.
    pub share_online: bool,
    /// Share which channel you're in (text and/or voice).
    pub share_location: ShareScope,
    /// Share activity / game string.
    pub share_activity: ShareScope,
    /// Last-seen granularity exposed to peers.
    pub last_seen: LastSeenPrecision,
}

/// Audience for an opt-in presence signal. `Members` = everyone in this
/// community; future scopes (friends-only, per-recipient) extend here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShareScope {
    /// Default-deny — share with nobody.
    #[default]
    None,
    /// Share with everyone in this community.
    Members,
}

/// Granularity of the last-seen signal exposed to peers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LastSeenPrecision {
    /// No last-seen at all.
    #[default]
    Hidden,
    /// Hour-granularity buckets ("recently" / "a while ago").
    Coarse,
    /// Precise timestamp (opt-in).
    Exact,
}

impl Default for PresenceSharingPolicy {
    fn default() -> Self {
        Self {
            share_online: true,
            share_location: ShareScope::None,
            share_activity: ShareScope::None,
            last_seen: LastSeenPrecision::Hidden,
        }
    }
}
