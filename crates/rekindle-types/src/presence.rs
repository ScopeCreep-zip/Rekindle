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

    /// Currently joined voice channel (None if not in voice).
    pub current_voice_channel: Option<ChannelId>,

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

impl Default for MemberPresence {
    fn default() -> Self {
        Self {
            pseudonym_key: PseudonymKey([0u8; 32]),
            display_name: None,
            status: "online".into(),
            custom_status: None,
            current_voice_channel: None,
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
    }
}
