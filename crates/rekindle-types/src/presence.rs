//! v2.0 Member presence types for the SMPL member registry.
//!
//! Each member writes their own MemberPresence to their subkey in the
//! registry SMPL record. Presence is self-sovereign — no coordinator
//! approval needed to update your own status, display name, or profile.
//!
//! See architecture doc §4.3 Record 2 and §24.2 for profile fields.
//! See rekindle-architecture-v2.md §4.2 for field specifications.

use serde::{Deserialize, Serialize};

use crate::id::{ChannelId, PseudonymKey};

#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod limits;
pub mod session;

// Re-exported so `rekindle_types::presence::SessionLocation` and friends
// keep resolving — the split is a file-layout change, not an API one.
use crate::domains;
pub use session::{
    EncryptedSessionExtras, IdleState, LastSeenPrecision, MemberSession, PresencePolicyField,
    PresenceSharingPolicy, SessionExtras, SessionLocation, SessionStatus, ShareScope,
    INVISIBLE_WIRE_VALUE,
};

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
    ///
    /// This is the GENERAL route (Reliable + PreferOrdered) — the one
    /// chat / governance / gossip need for ordered, reliable delivery.
    ///
    /// The row carries no media route (plan C7.15). Connection data for a
    /// call rides the call's own signaling — `VoiceJoin`, `VoiceJoinAck`
    /// and the re-announce — as Discord's `VOICE_SERVER_UPDATE` and
    /// Jingle's `transport-info` do, and both routes on the row overflowed
    /// its 4112-byte registry slot.
    #[serde(with = "crate::base64_bytes")]
    pub route_blob: Vec<u8>,

    /// Unix timestamp of last heartbeat write.
    pub last_heartbeat: u64,

    /// Set by a departing member on their own row to release the slot.
    ///
    /// `communities-governance.md` promises leaving frees a slot, but
    /// Veilid has no per-subkey delete and `inspect_present_subkeys`
    /// reports *ever written*, so a departed member's slot reads as
    /// occupied forever. This is the tombstone the claim path reads
    /// instead (`join_stages::reclaim`).
    ///
    /// It must be **signed**, which is why it lives in the row rather
    /// than being expressed as zeroed bytes: the slot seed is shared, so
    /// any member can write any slot, and an unsigned empty payload
    /// would let anyone free anyone's slot and collide two members onto
    /// one index. Inside `signing_bytes()` it cannot be flipped by a
    /// third party.
    ///
    /// `skip_serializing_if` is load-bearing for compatibility, not
    /// tidiness. A live row omits the field and stays byte-identical to
    /// the pre-existing format, so old readers still verify it. Only a
    /// departed row carries it — and an old reader, which drops unknown
    /// fields before recomputing `signing_bytes()`, fails that
    /// signature and treats the slot as reclaimable too. Both ends
    /// converge on the same answer without a flag day.
    #[serde(default, skip_serializing_if = "is_not_departed")]
    pub departed: bool,

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

    /// Voice channel this member is CURRENTLY connected to (MatrixRTC
    /// `m.rtc.member` pattern — the durable, heartbeat-renewed roster
    /// membership claim the voice reconcile keys on).
    ///
    /// CLEARTEXT, deliberately: voice discovery must work before the
    /// channel MEK converges, and a member's presence row is already
    /// members-only (W26-signed, in the SMPL registry). It was formerly
    /// inside the MEK-encrypted `SessionExtras`, which coupled discovery
    /// to key health and stalled voice on a split-brained MEK. Not
    /// subject to the location-sharing policy or read-side reciprocity —
    /// being in a voice channel is intrinsically visible to its members,
    /// and leaving the channel is how you stop sharing it.
    ///
    /// `skip_serializing_if` keeps a not-in-voice row byte-identical to
    /// the pre-existing format (same discipline as `departed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_channel_id: Option<String>,

    /// Route blob for an opt-in push relay (Tier 3 notifications).
    pub push_relay_route: Option<crate::base64_bytes::Base64Bytes>,

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
    /// [`MemberPresence::signing_bytes`]. The SMPL slot keypair on `set_dht_value` is
    /// community-shared (every member knows the slot seed), so without
    /// this signature any member could forge a presence write claiming
    /// to be any other member — including impersonating their voice
    /// channel state, RSVPs, custom status, or onboarding answers.
    /// Receivers MUST verify before applying.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        with = "crate::base64_bytes"
    )]
    pub signature: Vec<u8>,
}

/// `skip_serializing_if` predicate for [`MemberPresence::departed`].
///
/// Deliberately not `std::ops::Not::not` — serde hands the predicate a
/// reference, and the indirection is worth naming because omitting this
/// field is what keeps live rows wire-identical for old readers.
fn is_not_departed(departed: &bool) -> bool {
    !*departed
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
        let mut out = Vec::with_capacity(domains::PRESENCE_ROW.len() + json.len());
        out.extend_from_slice(domains::PRESENCE_ROW.as_bytes());
        out.extend_from_slice(&json);
        out
    }
}

/// Game currently being played (populated by rekindle-game-detect).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub game_name: String,
    pub game_id: Option<String>,
    pub elapsed_seconds: Option<u64>,
    pub server_address: Option<String>,
}

/// One registry segment of a community (architecture §15, Plate Gates).
///
/// Tier 1 because both `rekindle-presence` (Tier 5, scanning member
/// rows) and `rekindle-governance-runtime` (Tier 6, expanding segments)
/// need it, and Tier 5 cannot depend on Tier 6. They had a copy each,
/// differing only in `anchor` — which meant the desktop's
/// presence adapter mapped the Tier-6 descriptor into the Tier-5 one
/// field by field and *dropped* the anchor key on the way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentDescriptor {
    /// 0 for the genesis segment; 1.. for each Plate Gate expansion.
    pub segment_index: u32,
    /// SMPL member-registry record for this segment.
    pub registry_key: String,
    /// SMPL governance record for this segment. Empty for readers that
    /// only scan presence rows and never merge governance.
    pub anchor: String,
    /// Inclusive-exclusive global slot range this segment covers
    /// (`segment_index * SLOTS_PER_SEGMENT` .. `+ SLOTS_PER_SEGMENT`).
    /// 0 for readers that only scan presence rows and never merge
    /// governance — the frontend's capacity math only uses
    /// `segment_index`/list length, not these bounds.
    pub slot_range_start: u32,
    pub slot_range_end: u32,
}

/// A community member currently believed online, with what we need to
/// reach them and how fresh that belief is.
///
/// Tier 1 because it is presence vocabulary and both tracks need the
/// same five facts. `rekindle-transport` and src-tauri each declared
/// their own; transport's had only the first three, so the daemon's
/// `to_mesh_members` decoded `location` and `last_active` off the
/// registry row and then dropped them on the floor at the mesh
/// boundary. A daemon client could not show where a member was focused
/// or when they were last active, and nothing said why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlineMember {
    /// Veilid private route blob for reaching this member. May be empty:
    /// liveness (being in `online_members`) is decoupled from
    /// reachability (a route being present), so a routeless-but-live
    /// member still appears here.
    pub route_blob: Vec<u8>,
    /// Last advertised member status from the registry or gossip mesh.
    pub status: String,
    /// Seconds since epoch of the last valid gossip message or presence
    /// update. Drives TTL-based eviction of stale members.
    pub last_seen: u64,
    /// Where this member is focused (text/voice channel), once decoded
    /// from their session. `None` until the roster decode fills it.
    pub location: Option<SessionLocation>,
    /// The member's self-reported last-active, already coarsened per
    /// their own policy; drives last-seen bucketing on the roster.
    pub last_active: u64,
}

/// One submitted answer to an onboarding question.
///
/// This was declared here with `answer_text: String` while every live
/// site — the `SubmitOnboardingAnswers` control payload, the Cap'n Proto
/// codec, `governance-runtime`'s required-answer validation, and the
/// frontend DTO — used `selected_options: Vec<String>`. Tier 1 is where
/// someone looks for the authoritative shape, so the wrong one here was
/// worse than none: adopting it would have silently dropped every
/// multi-select answer, and `MemberPresence::onboarding_answers` was
/// already typed with it.
///
/// One declaration now, in the vocabulary tier;
/// `rekindle_codec::community::envelope` re-exports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingAnswer {
    pub question_id: String,
    /// Option IDs the member selected. Multi-select questions carry
    /// several; single-select carries one.
    pub selected_options: Vec<String>,
}

// Historical note kept for the next reader:
// had **zero users**, while every live site — the control envelope, the
// Cap'n Proto codec, `governance-runtime`'s required-answer validation,
// and the frontend DTO — uses `selected_options: Vec<String>`. A
// Tier-1 type is where someone looks for the authoritative shape, so a
// dead one with the wrong fields is worse than none: adopting it would
// have silently dropped every multi-select answer. The live definition
// is `rekindle_codec::community::envelope::OnboardingAnswer`.

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
    #[serde(with = "crate::base64_bytes")]
    pub ciphertext: Vec<u8>,
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
            // A default row is a live member, never a tombstone: every
            // heartbeat builds from `..Default::default()`, so defaulting
            // this to `true` would release the writer's own slot on every
            // presence write.
            departed: false,
            game_info: None,
            avatar_ref: None,
            banner_ref: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            in_call: false,
            call_type: None,
            voice_channel_id: None,
            push_relay_route: None,
            onboarding_answers: None,
            history_ranges_encrypted: None,
            session: MemberSession::default(),
            session_extras_encrypted: None,
            signature: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "presence_tests.rs"]
mod tests;
