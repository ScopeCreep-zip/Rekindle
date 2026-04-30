//! Wire envelope format for all community P2P traffic.
//!
//! Replaces the request/response model (`CommunityRequest`/`CommunityResponse`/`CommunityBroadcast`)
//! with unidirectional envelopes sent via `app_message` (fire-and-forget).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Wire envelope wrapping all community P2P traffic.
/// Sent via `app_message` (fire-and-forget) -- NOT `app_call` (request/response).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityEnvelope {
    /// Gossip notification that a new message exists in a channel SMPL record.
    ///
    /// Chiral Network model: gossip carries the notification (cargo manifest),
    /// not the cargo (ciphertext). Recipients fetch the actual MEK-encrypted
    /// content from the sender's SMPL subkey via `get_dht_value`.
    ///
    /// This ensures ciphertext exists only on DHT storage nodes (5 replicas),
    /// not across the entire gossip fan-out graph (50-100+ relay nodes).
    MessageNotification {
        channel_id: String,
        message_id: String,
        author_pseudonym: String,
        /// Sender's SMPL subkey index — where to fetch the ciphertext.
        subkey_index: u32,
        /// Lamport logical timestamp for causal ordering.
        lamport_ts: u64,
        /// Per-sender, per-channel sequence number for gap detection.
        sequence: u64,
        /// blake3 hash of the MEK-encrypted ciphertext, for integrity
        /// verification after DHT fetch. Ensures the fetched value matches
        /// what the sender wrote.
        content_hash: String,
        timestamp: u64,
    },
    /// A control operation (channel/role/invite/event management, moderation, etc.).
    Control(ControlPayload),
    /// Presence update from a member.
    PresenceUpdate {
        pseudonym_key: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_info: Option<PresenceGameInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
    },
    /// Typing indicator (ephemeral, not stored).
    TypingIndicator {
        channel_id: String,
        pseudonym_key: String,
    },
}

/// Game information for community presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceGameInfo {
    pub game_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

/// A participant entry in a voice roster broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRosterEntry {
    pub pseudonym_key: String,
    pub route_blob: Vec<u8>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub deafened: bool,
}

/// Signed wrapper: sender_pseudonym + serialized envelope + Ed25519 signature.
///
/// Signature is computed over `envelope_bytes` using the sender's pseudonym
/// signing key (derived via `rekindle_crypto::group::pseudonym::derive_community_pseudonym()`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedEnvelope {
    pub community_id: String,
    pub sender_pseudonym: String,
    pub envelope_bytes: Vec<u8>,
    /// Ed25519 signature over `envelope_bytes`.
    pub signature: Vec<u8>,
    /// Hop TTL for gossip forwarding. Starts at 5, decremented on each forward.
    /// When 0, process locally but don't forward.
    #[serde(default = "default_ttl")]
    pub ttl: u8,
}

fn default_ttl() -> u8 {
    5
}

/// Control payload covering all non-chat operations actually used by the
/// forward communities mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ControlPayload {
    // ── Member lifecycle ──
    /// Request to join the community.
    MemberJoinRequest {
        pseudonym_key: String,
        display_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invite_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
        /// Signal Protocol prekey bundle for MEK delivery.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prekey_bundle: Option<Vec<u8>>,
        /// SMPL subkey index the joiner has already claimed via self-service join.
        /// When present, the admin processing this request should use this index
        /// instead of assigning a new one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        claimed_subkey_index: Option<u32>,
    },
    /// Member voluntarily leaving.
    MemberLeave { pseudonym_key: String },
    /// Response: join accepted by admin peer.
    JoinAccepted {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        #[serde(default)]
        members: Vec<serde_json::Value>,
        /// The member registry DHT record key — needed for elections and presence.
        #[serde(default)]
        member_registry_key: Option<String>,
        /// Slot index for the joiner in the member registry SMPL record.
        #[serde(default)]
        slot_index: Option<u32>,
        /// Wrapped slot seed (ECDH-encrypted) — allows the joiner to derive
        /// their own slot keypair locally via `derive_slot_veilid_keypair(seed, slot_index)`.
        /// This eliminates any coordinator dependency for presence writing.
        #[serde(default)]
        wrapped_slot_seed: Option<Vec<u8>>,
    },
    /// Response: join rejected by admin peer.
    JoinRejected { reason: String },
    /// Broadcast: a member joined.
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
        status: String,
        /// Route blob so receivers can immediately add the joiner to their gossip overlay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
    },
    /// Broadcast: a member was removed (left, kicked, or banned).
    MemberRemoved { pseudonym_key: String },

    // ── Moderation ──
    /// Kick a member.
    Kick { target_pseudonym: String },
    /// Ban a member.
    Ban { target_pseudonym: String },
    /// Unban a member.
    Unban { target_pseudonym: String },
    /// Timeout a member.
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Remove a member's timeout.
    RemoveTimeout { target_pseudonym: String },
    /// Broadcast: member timed out.
    MemberTimedOut {
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },

    // ── Messages ──
    /// Broadcast: message edited.
    MessageEdited {
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
        edited_at: u64,
    },
    /// Broadcast: message deleted.
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },

    // ── MEK management ──
    /// Broadcast: MEK rotated by the deterministic rotator.
    MEKRotated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        new_generation: u64,
        /// Pseudonym of the rotator who performed the rotation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotator_pseudonym: Option<String>,
    },
    /// Request current MEK from the deterministic responder.
    /// Propagated via gossip with standard TTL and dedup.
    /// Only the deterministic responder (computed via `select_mek_responder`)
    /// replies with a wrapped MEK via `app_call`.
    RequestMEK {
        channel_id: String,
        /// The generation the requester needs.
        needed_generation: u64,
        /// Requester's pseudonym for deterministic responder selection.
        requester_pseudonym: String,
    },
    /// Direct app_call delivery of wrapped MEK material to a single peer.
    MekTransfer {
        community_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        generation: u64,
        sender_pseudonym: String,
        wrapped_mek: Vec<u8>,
    },
    /// Broadcast: member completed onboarding.
    OnboardingComplete {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel management ──
    /// Broadcast: member roles changed.
    MemberRolesChanged {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel permission overwrites ──
    /// Broadcast: channel overwrite changed.
    ChannelOverwriteChanged { channel_id: String },

    // ── Reactions ──
    /// Broadcast: reaction added.
    ReactionAdded {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// Broadcast: reaction removed.
    ReactionRemoved {
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },

    // ── Pinning ──
    /// Broadcast: message pinned.
    MessagePinned {
        channel_id: String,
        message_id: String,
        pinned_by: String,
    },
    /// Broadcast: message unpinned.
    MessageUnpinned {
        channel_id: String,
        message_id: String,
    },

    // ── Events ──
    /// Broadcast: event created.
    EventCreated { event: serde_json::Value },
    /// Broadcast: event updated.
    EventUpdated { event: serde_json::Value },
    /// Broadcast: event deleted.
    EventDeleted { event_id: String },
    /// Broadcast: event RSVP changed.
    EventRsvpChanged {
        event_id: String,
        pseudonym_key: String,
        status: String,
    },

    // ── Threads ──
    /// Broadcast: thread created.
    ThreadCreated { thread: serde_json::Value },
    /// Broadcast: thread message received.
    ThreadMessageReceived {
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
    },
    /// Broadcast: thread archived/unarchived.
    ThreadArchived { thread_id: String, archived: bool },

    // ── Game servers ──
    /// Broadcast: game server added.
    GameServerAdded { server: serde_json::Value },
    /// Broadcast: game server removed.
    GameServerRemoved { server_id: String },

    // ── Onboarding ──
    /// Submit onboarding answers.
    SubmitOnboardingAnswers { answers: Vec<OnboardingAnswer> },

    // ── Event reminders ──
    /// Broadcast: event starting soon reminder.
    EventReminder {
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },

    // ── Kicked notification ──
    /// Notification: you were kicked from the community.
    KickedNotification,

    // ── AutoMod / Raid notifications ──
    /// Raid alert broadcast to all members (owners/admins should act).
    RaidAlert { active: bool },
    /// Channel lockdown broadcast: non-admins should restrict sending.
    ChannelLockdown { locked: bool },
    /// System message broadcast (join/leave/kick/ban events posted to chat feed).
    SystemMessage { body: String, timestamp: u64 },

    // ── Admin delegation ──
    /// Grant the governance record writer keypair plus slot seed to a newly promoted admin.
    AdminKeypairGrant {
        /// Governance record writer keypair encrypted for the target member.
        wrapped_owner_keypair: Vec<u8>,
        /// Slot seed encrypted for the target member.
        wrapped_slot_seed: Vec<u8>,
    },
    /// Grant a specific slot keypair to a newly joined member.
    SlotKeypairGrant {
        slot_index: u32,
        segment_index: u32,
        /// Slot keypair encrypted for the target member.
        wrapped_slot_keypair: Vec<u8>,
    },
    // ── Bootstrap protocol ──
    /// Gossip notification that a governance SMPL subkey changed.
    GovernanceUpdated {
        governance_key: String,
        subkey_index: u32,
        lamport_ts: u64,
    },
    /// Request a BootstrapBundle from the inviter during community join.
    /// Sent via app_call (request-response) to the inviter's route.
    BootstrapRequest {
        /// Joiner's community pseudonym (hex-encoded Ed25519 public key).
        joiner_pseudonym: String,
        /// Governance record key (proves invite validity).
        governance_key: String,
    },
    /// Response with full community state for efficient bootstrapping.
    /// Returned via app_call reply. Joiner independently verifies against DHT.
    BootstrapResponse {
        /// All governance entries from all occupied subkeys.
        governance_entries: Vec<serde_json::Value>,
        /// Online members with presence data and route blobs.
        member_list: Vec<serde_json::Value>,
        /// Current MEK per channel, wrapped for the joiner's pseudonym.
        channel_meks: Vec<serde_json::Value>,
        /// Last 50 messages per channel (MEK-encrypted ciphertext).
        recent_messages: Vec<serde_json::Value>,
        /// Owner keypair wrapped for the joiner (shared infrastructure).
        wrapped_owner_keypair: Vec<u8>,
    },

    // ── Sync protocol ──
    /// Request channel history from an archiver node.
    SyncRequest {
        channel_id: String,
        since_timestamp: u64,
    },
    /// Response with channel messages from an archiver's local SQLite.
    SyncResponse {
        channel_id: String,
        messages: Vec<serde_json::Value>,
    },

    // ── Voice channel signaling ──
    /// Broadcast: member joined a voice channel.
    VoiceJoin {
        channel_id: String,
        /// Private route blob for receiving voice packets.
        route_blob: Vec<u8>,
    },
    /// Broadcast: member left a voice channel.
    VoiceLeave { channel_id: String },
    /// Broadcast: voice channel mode switch (mesh ↔ MCU).
    VoiceModeSwitch {
        channel_id: String,
        /// "mesh" or "mcu".
        mode: String,
        /// Pseudonym key of the MCU host (only set when mode = "mcu").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_pseudonym: Option<String>,
    },

    /// Moderator action: server-mute a member in a voice channel.
    VoiceMute {
        channel_id: String,
        target_pseudonym: String,
        muted: bool,
    },
    /// Moderator action: server-deafen a member in a voice channel.
    VoiceDeafen {
        channel_id: String,
        target_pseudonym: String,
        deafened: bool,
    },
    /// Voice roster broadcast: current participants for late joiners.
    VoiceRoster {
        channel_id: String,
        participants: Vec<VoiceRosterEntry>,
    },
    // ── Generic responses ──
}

/// A single onboarding answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingAnswer {
    pub question_id: String,
    pub selected_options: Vec<String>,
}

/// Create a signed envelope from a serialized envelope payload.
///
/// Signs `envelope_bytes` with the pseudonym's Ed25519 signing key.
pub fn sign_envelope(
    signing_key: &SigningKey,
    community_id: &str,
    sender_pseudonym: &str,
    envelope_bytes: &[u8],
) -> SignedEnvelope {
    let signature = signing_key.sign(envelope_bytes);
    SignedEnvelope {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        envelope_bytes: envelope_bytes.to_vec(),
        signature: signature.to_bytes().to_vec(),
        ttl: default_ttl(),
    }
}

/// Verify the Ed25519 signature on a signed envelope.
///
/// The `sender_pseudonym` field is the hex-encoded Ed25519 public key.
/// Returns `Ok(())` if the signature is valid.
pub fn verify_envelope(signed: &SignedEnvelope) -> Result<(), String> {
    let pub_bytes =
        hex::decode(&signed.sender_pseudonym).map_err(|e| format!("invalid pseudonym hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "pseudonym key must be 32 bytes".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_array: [u8; 64] = signed
        .signature
        .clone()
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(&signed.envelope_bytes, &signature)
        .map_err(|e| format!("invalid envelope signature: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = VerifyingKey::from(&signing_key);
        let pseudonym_hex = hex::encode(verifying_key.to_bytes());

        let envelope = CommunityEnvelope::TypingIndicator {
            channel_id: "ch_01".into(),
            pseudonym_key: pseudonym_hex.clone(),
        };
        let envelope_bytes = serde_json::to_vec(&envelope).unwrap();

        let signed = sign_envelope(
            &signing_key,
            "community_abc",
            &pseudonym_hex,
            &envelope_bytes,
        );

        assert!(verify_envelope(&signed).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_data() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = VerifyingKey::from(&signing_key);
        let pseudonym_hex = hex::encode(verifying_key.to_bytes());

        let envelope_bytes = b"original data";
        let mut signed = sign_envelope(
            &signing_key,
            "community_abc",
            &pseudonym_hex,
            envelope_bytes,
        );

        // Tamper with the data
        signed.envelope_bytes = b"tampered data".to_vec();

        assert!(verify_envelope(&signed).is_err());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let secret1 = [42u8; 32];
        let secret2 = [99u8; 32];
        let signing_key = SigningKey::from_bytes(&secret1);
        let wrong_verifying = VerifyingKey::from(&SigningKey::from_bytes(&secret2));
        let wrong_hex = hex::encode(wrong_verifying.to_bytes());

        let envelope_bytes = b"test data";
        let mut signed =
            sign_envelope(&signing_key, "community_abc", "placeholder", envelope_bytes);

        // Replace sender pseudonym with wrong key
        signed.sender_pseudonym = wrong_hex;

        assert!(verify_envelope(&signed).is_err());
    }

    #[test]
    fn envelope_message_notification_serde() {
        let envelope = CommunityEnvelope::MessageNotification {
            channel_id: "ch_01".into(),
            message_id: "msg_abc".into(),
            author_pseudonym: "pseudo_123".into(),
            subkey_index: 7,
            lamport_ts: 42,
            sequence: 7,
            content_hash: "abc123".into(),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let back: CommunityEnvelope = serde_json::from_str(&json).unwrap();
        match back {
            CommunityEnvelope::MessageNotification { channel_id, .. } => {
                assert_eq!(channel_id, "ch_01");
            }
            _ => panic!("wrong variant"),
        }
    }

    /// Regression guard: MessageNotification must NEVER contain a "ciphertext" field.
    /// Gossip carries the cargo manifest (metadata), not the cargo (ciphertext).
    /// Ciphertext exists only on DHT storage nodes (5 replicas), not across the
    /// entire gossip fan-out graph.
    #[test]
    fn message_notification_contains_no_ciphertext() {
        let envelope = CommunityEnvelope::MessageNotification {
            channel_id: "ch_01".into(),
            message_id: "msg_abc".into(),
            author_pseudonym: "pseudo_123".into(),
            subkey_index: 7,
            lamport_ts: 42,
            sequence: 7,
            content_hash: "abc123def456".into(),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(
            !json.contains("ciphertext"),
            "MessageNotification must NOT contain ciphertext — gossip carries \
             notifications only. Got: {json}"
        );
    }

    /// Regression guard: MessageNotification payload stays compact.
    /// The notification is only metadata. With short on-wire identifiers it
    /// should remain comfortably under 200 bytes; if ciphertext sneaks in,
    /// this limit will fail immediately.
    #[test]
    fn message_notification_payload_stays_compact() {
        let envelope = CommunityEnvelope::MessageNotification {
            channel_id: "ch01".into(),
            message_id: "m01".into(),
            author_pseudonym: "p01".into(),
            subkey_index: 7,
            lamport_ts: 42,
            sequence: 3,
            content_hash: "abc123".into(),
            timestamp: 1234567890,
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(
            bytes.len() < 200,
            "MessageNotification should be compact (< 200 bytes), was {} bytes. \
             If this fails, check if ciphertext or large fields were added.",
            bytes.len()
        );
    }

    #[test]
    fn control_payload_serde() {
        let payload = ControlPayload::MemberLeave {
            pseudonym_key: "abc123".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: ControlPayload = serde_json::from_str(&json).unwrap();
        match back {
            ControlPayload::MemberLeave { pseudonym_key } => assert_eq!(pseudonym_key, "abc123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn signed_envelope_serde() {
        let signed = SignedEnvelope {
            community_id: "comm_01".into(),
            sender_pseudonym: "abc123".into(),
            envelope_bytes: vec![1, 2, 3],
            signature: vec![0u8; 64],
            ttl: 5,
        };
        let json = serde_json::to_string(&signed).unwrap();
        let back: SignedEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.community_id, "comm_01");
    }
}
