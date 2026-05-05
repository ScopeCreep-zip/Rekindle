use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Wire format envelope wrapping all messages sent over Veilid.
///
/// The payload is E2E encrypted (Signal Protocol for DMs, MEK for channels).
/// This envelope provides sender identification and integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Sender's Ed25519 public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Unique message nonce (for deduplication and ordering).
    pub nonce: Vec<u8>,
    /// Encrypted payload (ciphertext).
    pub payload: Vec<u8>,
    /// Ed25519 signature over (timestamp || nonce || payload).
    pub signature: Vec<u8>,
}

/// The type of message contained in the envelope payload (after decryption).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessagePayload {
    /// Direct 1:1 chat message.
    DirectMessage {
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Channel message (community text channel).
    ChannelMessage {
        channel_id: String,
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Typing indicator.
    TypingIndicator { typing: bool },
    /// Friend request.
    FriendRequest {
        display_name: String,
        message: String,
        prekey_bundle: Vec<u8>,
        /// Sender's private profile DHT key (for presence watching).
        profile_dht_key: String,
        /// Sender's current route blob (for immediate contact).
        route_blob: Vec<u8>,
        /// Sender's mailbox DHT key (for route discovery after reconnect).
        mailbox_dht_key: String,
        /// Correlation token linking this request back to a specific invite.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invite_id: Option<String>,
    },
    /// Friend request acceptance.
    FriendAccept {
        prekey_bundle: Vec<u8>,
        /// Acceptor's private profile DHT key.
        profile_dht_key: String,
        /// Acceptor's current route blob.
        route_blob: Vec<u8>,
        /// Acceptor's mailbox DHT key.
        mailbox_dht_key: String,
        /// Initiator's X25519 ephemeral public key (for responder-side X3DH).
        ephemeral_key: Vec<u8>,
        /// Which of the responder's signed prekeys was used by the initiator.
        signed_prekey_id: u32,
        /// Which of the responder's one-time prekeys was consumed (if any).
        one_time_prekey_id: Option<u32>,
    },
    /// Friend request rejection.
    FriendReject,
    /// Sent to remaining friends after profile key rotation (block/unfriend).
    ProfileKeyRotated { new_profile_dht_key: String },
    /// Lightweight ACK confirming a `FriendRequest` was received and stored.
    /// Does NOT mean acceptance — just delivery confirmation.
    FriendRequestReceived,
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GameInfo>,
    },
    /// Notify the peer that we have removed them as a friend.
    Unfriended,
    /// ACK confirming an `Unfriended` message was received and processed.
    UnfriendedAck,
    /// Strand Relay (architecture §13.2 step 2): a friend offers a dedicated
    /// relay route. The recipient appends the blob to their published relay
    /// pool so other contacts who can't reach them directly can route via
    /// this friend. The friend can revoke later via `RelayWithdraw`.
    RelayOffer {
        /// Opaque Veilid private-route blob created by the relay friend
        /// for forwarding-only use (kept distinct from her personal route).
        relay_route_blob: Vec<u8>,
        /// Hex-encoded Ed25519 public key of the friend volunteering to relay.
        relay_pseudonym: String,
    },
    /// Strand Relay revocation: the relay friend withdraws her offer.
    RelayWithdraw {
        relay_pseudonym: String,
    },
    /// Bob's `app_call` reply to Carol confirming her `RelayOffer` was
    /// persisted into his relay pool (architecture §13.2 step 3).
    RelayOfferAck {
        ok: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Strand Relay forward request (architecture §13.3 step 2): Alice→Carol.
    /// Carol sees a friend (`target_pubkey`) referenced and re-emits
    /// `inner_payload` (an entire opaque MessageEnvelope addressed to Bob)
    /// onto Bob's current route. Carol cannot read the inner content.
    RelayEnvelope {
        /// Hex-encoded Ed25519 public key of the ultimate recipient.
        target_pubkey: String,
        /// Opaque envelope bytes — the relay forwards verbatim. Encrypted
        /// to the target only, so the relay never sees plaintext.
        inner_payload: Vec<u8>,
    },
    /// 2-party DM invite (architecture §27.1): Alice → Bob. Carries the
    /// SMPL record key and slot seed; the MEK is *not* in the payload —
    /// both peers derive it deterministically via X25519 ECDH from
    /// their identity keys.
    DmInvite {
        record_key: String,
        slot_seed: Vec<u8>,
        alice_pseudonym: String,
        alice_subkey: u32,
        bob_subkey: u32,
    },
    /// Bob accepts a DM invite (architecture §27.1 line 2917).
    /// Returned as the `app_call` reply to a `DmInvite` so Alice's
    /// `start_dm` future resolves with confirmation.
    DmAccept {
        record_key: String,
    },
    /// Bob declines a DM invite (architecture §27.1).
    DmDecline {
        record_key: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Group DM invite (architecture §27.2): MEK is wrapped per
    /// recipient with X25519 because ECDH only works pairwise.
    GroupDmInvite {
        record_key: String,
        slot_seed: Vec<u8>,
        initiator_pseudonym: String,
        /// JSON-encoded `Vec<rekindle_dm::GroupDmParticipant>` to keep the
        /// envelope crate dependency-free (`rekindle-dm` lives at Tier 7).
        participants_json: String,
        wrapped_mek: Vec<u8>,
        mek_generation: u32,
    },
    /// One side leaving a DM (graceful close).
    DmLeave {
        record_key: String,
    },
    /// Mobile Push Relay registration (architecture §17.3 Tier 3).
    /// A mobile client asks a headless `veilid-server` push relay to
    /// watch a list of DHT record keys on its behalf and forward
    /// content-free wake signals via FCM/APNs (`{"t":"wake"}`). The
    /// relay never sees ciphertext or metadata about what changed —
    /// only that *some* registered record fired.
    RegisterPushRelay {
        /// Hex-encoded device push token (FCM registration id, APNs
        /// device token, or opaque ID for self-hosted relays).
        device_push_token: String,
        /// Platform identifier ("fcm", "apns", "self") for routing.
        platform: String,
        /// Veilid record keys (string-encoded) the relay should watch.
        record_keys: Vec<String>,
    },
    /// Mobile Push Relay revoke. Sent on logout or when the device
    /// invalidates its push token.
    UnregisterPushRelay {
        device_push_token: String,
    },
    /// Wake signal — relay → mobile via FCM/APNs (out-of-band) or
    /// directly via Veilid `app_message` for desktop testing. The
    /// payload is intentionally empty of metadata: the client
    /// re-fetches the relevant records itself.
    WakeNotify {
        /// Server-side timestamp (seconds) so the client can detect
        /// stale wakes after device sleep.
        ts: u64,
    },
    /// Strand Relay presence caching (architecture §13.5): a peer asks
    /// us "do you know `target_pubkey`'s current status?". We respond
    /// from our own friend-presence state if `target_pubkey` is a
    /// friend we relay for; otherwise we drop. Faster than a DHT
    /// lookup (the social CDN pattern).
    StatusRequest {
        target_pubkey: String,
    },
    /// Direct call offer (architecture §10.10 / Plan §Failure 5).
    /// The initiator generates an ephemeral X25519 keypair, sends the
    /// public key here, and awaits `CallAccept` (with the responder's
    /// public key) so both sides can derive the same `call_key` via
    /// HKDF-SHA256 over the ECDH shared secret. Shipped via
    /// `app_call` so the responder's accept/decline returns inline.
    CallOffer {
        /// Hex-encoded 16-byte random call identifier (32 chars).
        call_id: String,
        /// 0 = audio, 1 = video. Matches `rekindle_calls::CallKind::as_u8`.
        offer_kind: u8,
        /// Initiator's hex-encoded Ed25519 identity key. Used by the
        /// responder to look up display name + avatar.
        initiator_pubkey: String,
        /// Initiator's ephemeral X25519 public key (32 bytes).
        initiator_x25519_pub: Vec<u8>,
        /// Unix milliseconds when the ring should be considered missed.
        /// Initiator sets `now + 30_000`.
        expires_at_ms: u64,
    },
    /// Reply to `CallOffer` carrying the responder's X25519 public key
    /// so the initiator can finish key derivation. Returned as the
    /// `app_call` reply, never sent unsolicited.
    CallAccept {
        call_id: String,
        acceptor_x25519_pub: Vec<u8>,
    },
    /// Reply to `CallOffer` rejecting the call. Same shape as
    /// `DmDecline` so the dispatcher can mirror existing patterns.
    CallDecline {
        call_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Reply to a `StatusRequest`. Empty `status` means "I don't have
    /// data for this peer" so the requester can short-circuit.
    StatusResponse {
        target_pubkey: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        /// Unix timestamp (seconds) of the last presence update we saw
        /// for this peer. Lets the requester reject stale snapshots.
        last_seen: u64,
        /// The peer's most recent route blob, so the requester can
        /// short-circuit DHT route lookup as well.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        route_blob: Vec<u8>,
    },
}

/// Game information for rich presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// Direct server address ("ip:port") for join-game functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

// ---------------------------------------------------------------------------
// Invite blob types
// ---------------------------------------------------------------------------

/// A signed invite blob that contains everything needed for initial contact.
///
/// Encoded as JSON, signed with Ed25519, then base64url-encoded for sharing
/// as a `rekindle://` URL or plain string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteBlob {
    /// Sender's Ed25519 public key (hex).
    pub public_key: String,
    /// Sender's display name.
    pub display_name: String,
    /// Sender's mailbox DHT record key (for reading route blob).
    pub mailbox_dht_key: String,
    /// Sender's private profile DHT record key (for presence watching).
    pub profile_dht_key: String,
    /// Sender's current route blob (for immediate contact, may be stale).
    pub route_blob: Vec<u8>,
    /// Sender's Signal `PreKeyBundle` (serialized JSON).
    pub prekey_bundle: Vec<u8>,
    /// Correlation token linking this invite to tracked outgoing invites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_id: Option<String>,
    /// Ed25519 signature over the JSON of all fields above.
    pub signature: Vec<u8>,
}

/// Create a signed invite blob from identity credentials.
///
/// Signs over a JSON-serialized form of the invite data (excluding the
/// signature field itself) using the Ed25519 secret key.
pub fn create_invite_blob(
    secret_key: &[u8; 32],
    public_key: &str,
    display_name: &str,
    mailbox_dht_key: &str,
    profile_dht_key: &str,
    route_blob: &[u8],
    prekey_bundle: &[u8],
    invite_id: Option<&str>,
) -> InviteBlob {
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(secret_key);

    // Build the signable payload (all fields except signature)
    let signable = serde_json::json!({
        "public_key": public_key,
        "display_name": display_name,
        "mailbox_dht_key": mailbox_dht_key,
        "profile_dht_key": profile_dht_key,
        "route_blob": route_blob,
        "prekey_bundle": prekey_bundle,
        "invite_id": invite_id,
    });
    let signable_bytes = serde_json::to_vec(&signable).unwrap_or_default();
    let signature = signing_key.sign(&signable_bytes);

    InviteBlob {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
        mailbox_dht_key: mailbox_dht_key.to_string(),
        profile_dht_key: profile_dht_key.to_string(),
        route_blob: route_blob.to_vec(),
        prekey_bundle: prekey_bundle.to_vec(),
        invite_id: invite_id.map(str::to_string),
        signature: signature.to_bytes().to_vec(),
    }
}

/// Verify the Ed25519 signature on an invite blob.
///
/// Returns `Ok(())` if the signature is valid, `Err` otherwise.
pub fn verify_invite_blob(blob: &InviteBlob) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let pub_bytes =
        hex::decode(&blob.public_key).map_err(|e| format!("invalid public key hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_array: [u8; 64] = blob
        .signature
        .clone()
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);

    // Reconstruct the signable payload
    let signable = serde_json::json!({
        "public_key": blob.public_key,
        "display_name": blob.display_name,
        "mailbox_dht_key": blob.mailbox_dht_key,
        "profile_dht_key": blob.profile_dht_key,
        "route_blob": blob.route_blob,
        "prekey_bundle": blob.prekey_bundle,
        "invite_id": blob.invite_id,
    });
    let signable_bytes = serde_json::to_vec(&signable).unwrap_or_default();

    verifying_key
        .verify(&signable_bytes, &signature)
        .map_err(|e| format!("invalid invite signature: {e}"))
}

/// Encode an invite blob as a `rekindle://` URL.
pub fn encode_invite_url(blob: &InviteBlob) -> String {
    let json = serde_json::to_vec(blob).unwrap_or_default();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json);
    format!("rekindle://{encoded}")
}

/// Decode an invite blob from a `rekindle://` URL or raw base64 string.
pub fn decode_invite_url(url: &str) -> Result<InviteBlob, String> {
    let data = url.strip_prefix("rekindle://").unwrap_or(url);
    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|e| format!("invalid base64: {e}"))?;
    let blob: InviteBlob =
        serde_json::from_slice(&json_bytes).map_err(|e| format!("invalid invite JSON: {e}"))?;
    Ok(blob)
}

// ---------------------------------------------------------------------------
// Community server RPC DTOs (used for Tauri IPC serialization)
// ---------------------------------------------------------------------------

// NOTE: CommunityRequest, CommunityResponse, and CommunityBroadcast enums
// have been removed. All community protocol now goes through the v2
// coordinator/ControlPayload model (see dht::community::envelope).

/// A role definition as returned by the server over RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    #[serde(default)]
    pub self_assignable: bool,
}

/// A community member as returned by the server in the join/rejoin response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfoDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
}

/// A banned member as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
}

/// A channel message as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessageDto {
    pub message_id: String,
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionGroupDto>,
}

/// Helper for `skip_serializing_if` on `u32` fields that default to 0.
///
/// `serde`'s `skip_serializing_if` always passes by reference, so `&u32` is required.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Channel info as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub slowmode_seconds: u32,
}

/// A channel category as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// A community invite code as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteDto {
    pub code: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
}

/// A pinned message as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageDto {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

/// Aggregated reaction data for a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionGroupDto {
    pub emoji: String,
    pub count: u32,
    pub reactors: Vec<String>,
}

/// An audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntryDto {
    pub action: String,
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub details: Option<String>,
    pub timestamp: u64,
}

/// A community event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub creator_pseudonym: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    /// Lifecycle status: "scheduled", "active", "completed", "canceled".
    pub status: String,
    pub rsvps: Vec<EventRsvpDto>,
}

/// An RSVP entry for an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRsvpDto {
    pub pseudonym_key: String,
    pub status: String,
}

/// A game server favorite in a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerDto {
    pub id: String,
    pub game_id: String,
    pub label: String,
    pub address: String,
    pub added_by: String,
    pub created_at: u64,
}

/// Unread count for a single channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountDto {
    pub channel_id: String,
    pub unread_count: u32,
}

/// A thread (branching conversation from a channel message).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfoDto {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
    pub last_message_at: u64,
    pub message_count: u32,
}
