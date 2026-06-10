//! v2.0 Member presence types for the SMPL member registry.
//!
//! Each member writes their own MemberPresence to their subkey in the
//! registry SMPL record. Presence is self-sovereign — no coordinator
//! approval needed to update your own status, display name, or profile.
//!
//! See architecture doc §4.3 Record 2 and §24.2 for profile fields.
//! See rekindle-architecture-v2.md §4.2 for field specifications.

use serde::{Deserialize, Serialize};

use crate::id::{ChannelId, EventId, PseudonymKey};

/// Member presence data written to the registry SMPL subkey.
///
/// Updated every 15 seconds by the heartbeat loop. Contains both
/// ephemeral state (status, voice channel, route blob) and profile
/// data (display name, bio, avatar).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemberPresence {
    /// This member's community-specific pseudonym public key.
    pub pseudonym_key: PseudonymKey,

    /// Self-sovereign display name (no coordinator approval).
    pub display_name: Option<String>,

    /// "online", "away", "busy", "offline"
    pub status: String,

    /// Custom status text (e.g., "Playing Halo")
    pub custom_status: Option<String>,

    /// Current private route blob for direct messaging.
    /// Refreshed every 120 seconds.
    pub route_blob: Vec<u8>,

    /// Unix timestamp of last heartbeat write.
    pub last_heartbeat: u64,

    /// Currently playing game (from rekindle-game-detect).
    pub game_info: Option<GameInfo>,

    /// Content-addressed avatar reference (BLAKE3 hash).
    pub avatar_ref: Option<String>,

    /// Content-addressed banner reference (BLAKE3 hash). Architecture
    /// §24.2 / §32 Week 15 specifies a per-community banner alongside
    /// the avatar.
    pub banner_ref: Option<String>,

    /// Short bio (max 190 chars).
    pub bio: Option<String>,

    /// Pronouns (max 40 chars).
    pub pronouns: Option<String>,

    /// Profile accent color (ARGB u32).
    pub theme_color: Option<u32>,

    /// Earned/assigned badge IDs.
    pub badges: Vec<String>,

    /// Whether currently in a call.
    pub in_call: bool,

    /// "audio", "video", "screen_share" (if in_call is true).
    pub call_type: Option<String>,

    /// Route blob for an opt-in push relay (Tier 3 notifications).
    pub push_relay_route: Option<Vec<u8>>,

    /// RSVPs for scheduled events.
    pub event_rsvps: Vec<EventRSVP>,

    /// Reader-aggregated onboarding answers submitted by this member.
    pub onboarding_answers: Option<Vec<OnboardingAnswer>>,

    /// W11.2 — advertised message history ranges, encrypted under the
    /// current community MEK (architecture §14.3 mutual aid + §16.3
    /// reader-validates). Plaintext history_ranges leaked metadata to
    /// any reader of the registry record (community members today, but
    /// also banned ex-members who cached the registry key, and any
    /// network observer with access to Veilid's DHT storage nodes).
    /// Encrypting under the rotating MEK means post-ban access is
    /// revoked at the next MEK rotation (already wired) and observers
    /// without membership see only opaque ciphertext.
    ///
    /// `None` for fresh joiners who haven't computed their first
    /// history advertisement yet, OR for members whose MEK is missing /
    /// stale at write time. Receivers without the matching MEK
    /// generation skip the field gracefully and fall back to direct
    /// DHT reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_ranges_encrypted: Option<EncryptedHistoryRanges>,

    /// Typed session state — the single source of truth for "what is
    /// this member doing right now". The plaintext form carries only
    /// `status` + coarsened `last_active`; the identity-revealing
    /// `location` / `activity` ride in [`Self::session_extras_encrypted`]
    /// (MEK-bounded readership). The loose `status` string above is
    /// derived from `session.status` on write so classifier readers (which
    /// match on the plaintext string) keep working.
    #[serde(default)]
    pub session: MemberSession,

    /// MEK-encrypted `SessionExtras` (location + activity). `None` when
    /// the member shares neither signal, or when the MEK is missing at
    /// write time. Receivers without the matching generation skip it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_extras_encrypted: Option<EncryptedSessionExtras>,

    /// Architecture §26 W26 — Ed25519 signature by `pseudonym_key` over
    /// [`signing_bytes`]. The SMPL slot keypair on `set_dht_value` is
    /// community-shared (every member knows the slot seed), so without
    /// this signature any member could forge a presence write claiming
    /// to be any other member — including impersonating their voice
    /// channel state, RSVPs, custom status, or onboarding answers.
    /// Receivers MUST verify before applying.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl MemberPresence {
    /// Canonical bytes the author signs. Domain-tagged so the signature
    /// can't be replayed into a different protocol surface.
    pub fn signing_bytes(&self) -> Vec<u8> {
        // Clear the signature for the canonical form, then serialise the
        // rest of the struct deterministically (serde_json field order is
        // the struct declaration order).
        let mut canonical = self.clone();
        canonical.signature = Vec::new();
        let json = serde_json::to_vec(&canonical).unwrap_or_default();
        let mut out = Vec::with_capacity(b"rekindle-presence-v1".len() + json.len());
        out.extend_from_slice(b"rekindle-presence-v1");
        out.extend_from_slice(&json);
        out
    }
}

/// Game currently being played (populated by rekindle-game-detect).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub game_name: String,
    pub game_id: Option<String>,
    pub elapsed_seconds: Option<u64>,
    pub server_address: Option<String>,
}

/// RSVP for a scheduled community event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventRSVP {
    pub event_id: EventId,
    /// "going", "interested", "declined"
    pub status: String,
}

/// Answer text submitted for an onboarding question.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingAnswer {
    pub question_id: String,
    pub answer_text: String,
}

/// Range of message history this member has cached locally.
/// Used by mutual aid: newcomers can request ranges from peers who have them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRange {
    pub channel_id: ChannelId,
    pub oldest_lamport: u64,
    pub newest_lamport: u64,
}

/// W11.2 — wire format for MEK-encrypted history advertisements.
///
/// `ciphertext` is `nonce(12) || aes256gcm_ciphertext_with_tag` over a
/// JSON-serialized `Vec<HistoryRange>`. `mek_generation` is the
/// generation of the MEK used at encrypt time so receivers know which
/// cached MEK to try (and can skip if they don't have it).
///
/// The whole `EncryptedHistoryRanges` struct is included in
/// `MemberPresence::signing_bytes` so its authenticity is bound to the
/// presence row's signature — an attacker who substitutes the
/// ciphertext invalidates the signature and the receiver drops the row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedHistoryRanges {
    pub mek_generation: u64,
    pub ciphertext: Vec<u8>,
}

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

impl SessionStatus {
    /// Wire string used by the transitional `MemberPresence.status`
    /// field (Invisible folds to "offline" — peers must not see it).
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Offline | Self::Invisible => "offline",
        }
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
    /// Voice channel this member is CURRENTLY connected to (MatrixRTC
    /// `m.rtc.member` pattern — the durable, heartbeat-renewed roster
    /// membership claim). Unlike `location`, this is NOT subject to the
    /// location-sharing policy or read-side reciprocity: participating
    /// in a voice channel is intrinsically visible to its members, and
    /// leaving the channel is how you stop sharing it. MEK-encrypted
    /// like the rest of the extras, so it stays members-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_channel_id: Option<String>,
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

impl Default for MemberPresence {
    fn default() -> Self {
        Self {
            pseudonym_key: PseudonymKey([0u8; 32]),
            display_name: None,
            status: "online".into(),
            custom_status: None,
            route_blob: Vec::new(),
            last_heartbeat: 0,
            game_info: None,
            avatar_ref: None,
            banner_ref: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            in_call: false,
            call_type: None,
            push_relay_route: None,
            event_rsvps: Vec::new(),
            onboarding_answers: None,
            history_ranges_encrypted: None,
            session: MemberSession::default(),
            session_extras_encrypted: None,
            signature: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_serde_roundtrip() {
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey([0xAB; 32]),
            display_name: Some("FireStarter92".into()),
            status: "online".into(),
            route_blob: vec![1, 2, 3],
            last_heartbeat: 1710000000,
            ..Default::default()
        };
        let json = serde_json::to_string(&presence).unwrap();
        let back: MemberPresence = serde_json::from_str(&json).unwrap();
        assert_eq!(presence, back);
    }

    #[test]
    fn encrypted_history_ranges_roundtrip() {
        // W11.2 — wire format must survive serde and the signature
        // path. Build a MemberPresence with the encrypted field
        // populated and verify serialize → deserialize produces an
        // identical struct (signing_bytes consistency).
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey([0xCD; 32]),
            display_name: Some("Tester".into()),
            history_ranges_encrypted: Some(EncryptedHistoryRanges {
                mek_generation: 7,
                ciphertext: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x10, 0x20, 0x30],
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&presence).unwrap();
        let back: MemberPresence = serde_json::from_str(&json).unwrap();
        assert_eq!(presence, back);
        // The same input produces identical signing bytes → signature
        // verification is stable across encode/decode.
        assert_eq!(presence.signing_bytes(), back.signing_bytes());
    }

    #[test]
    fn encrypted_history_ranges_omitted_when_none() {
        // `skip_serializing_if = "Option::is_none"` keeps the JSON
        // small for the common case (most presence rows don't carry
        // history ads). Confirm the field literally doesn't appear in
        // the wire form when None.
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey([0; 32]),
            ..Default::default()
        };
        let json = serde_json::to_string(&presence).unwrap();
        assert!(
            !json.contains("historyRangesEncrypted"),
            "absent field must not serialize"
        );
    }

    #[test]
    fn default_presence_is_online() {
        let p = MemberPresence::default();
        assert_eq!(p.status, "online");
        assert!(!p.in_call);
        assert!(p.route_blob.is_empty());
        assert_eq!(p.session.status, SessionStatus::Online);
        assert!(p.session.location.is_none());
        assert!(p.session_extras_encrypted.is_none());
    }

    #[test]
    fn session_status_wire_strings() {
        assert_eq!(SessionStatus::Online.as_wire_str(), "online");
        assert_eq!(SessionStatus::Away.as_wire_str(), "away");
        assert_eq!(SessionStatus::Busy.as_wire_str(), "busy");
        assert_eq!(SessionStatus::Offline.as_wire_str(), "offline");
        // Invisible must never leak as a distinct wire status.
        assert_eq!(SessionStatus::Invisible.as_wire_str(), "offline");
    }

    #[test]
    fn session_serde_roundtrip_and_plaintext_strips_extras() {
        // The plaintext `session` field must not serialise location /
        // activity — they ride encrypted. With None they're omitted.
        let session = MemberSession {
            status: SessionStatus::Away,
            location: None,
            activity: None,
            last_active: 42,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(!json.contains("location"));
        assert!(!json.contains("activity"));
        let back: MemberSession = serde_json::from_str(&json).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn session_location_tagged_roundtrip() {
        let loc = SessionLocation::Voice {
            channel_id: "lounge".into(),
        };
        let json = serde_json::to_string(&loc).unwrap();
        assert!(json.contains("\"kind\":\"voice\""));
        let back: SessionLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(loc, back);
        assert_eq!(back.channel_id(), "lounge");
    }

    #[test]
    fn session_extras_roundtrip() {
        let extras = SessionExtras {
            location: Some(SessionLocation::Text {
                channel_id: "general".into(),
            }),
            activity: Some("Playing Halo".into()),
            voice_channel_id: Some("lounge-voice".into()),
        };
        let json = serde_json::to_vec(&extras).unwrap();
        let back: SessionExtras = serde_json::from_slice(&json).unwrap();
        assert_eq!(extras, back);
    }

    #[test]
    fn default_sharing_policy_is_deny() {
        let p = PresenceSharingPolicy::default();
        assert!(p.share_online);
        assert_eq!(p.share_location, ShareScope::None);
        assert_eq!(p.share_activity, ShareScope::None);
        assert_eq!(p.last_seen, LastSeenPrecision::Hidden);
    }
}
