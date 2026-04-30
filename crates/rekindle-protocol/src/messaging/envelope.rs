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
