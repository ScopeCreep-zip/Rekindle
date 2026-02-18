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
    ProfileKeyRotated {
        new_profile_dht_key: String,
    },
    /// Lightweight ACK confirming a `FriendRequest` was received and stored.
    /// Does NOT mean acceptance — just delivery confirmation.
    FriendRequestReceived,
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GameInfo>,
    },
}

/// Game information for rich presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
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
        signature: signature.to_bytes().to_vec(),
    }
}

/// Verify the Ed25519 signature on an invite blob.
///
/// Returns `Ok(())` if the signature is valid, `Err` otherwise.
pub fn verify_invite_blob(blob: &InviteBlob) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let pub_bytes = hex::decode(&blob.public_key)
        .map_err(|e| format!("invalid public key hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let verifying_key = VerifyingKey::from_bytes(&pub_array)
        .map_err(|e| format!("invalid public key: {e}"))?;

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
// Community server RPC types
// ---------------------------------------------------------------------------

/// Request from a member to the community server (sent via `app_call`).
///
/// Wrapped in a `MessageEnvelope` signed by the member's pseudonym key.
/// The server verifies the signature and extracts `sender_key` to identify
/// which pseudonym is making the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityRequest {
    /// Join the community (new member).
    Join {
        pseudonym_pubkey: String,
        invite_code: Option<String>,
        display_name: String,
        prekey_bundle: Vec<u8>,
        /// The member's private route blob so the server can broadcast to them.
        route_blob: Option<Vec<u8>>,
    },
    /// Send a message to a channel.
    SendMessage {
        channel_id: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
    },
    /// Fetch message history for a channel.
    GetMessages {
        channel_id: String,
        before_timestamp: Option<u64>,
        limit: u32,
    },
    /// Request current MEK (e.g., after reconnect).
    RequestMEK,
    /// Leave the community.
    Leave,
    /// Admin: kick a member.
    Kick {
        target_pseudonym: String,
    },
    /// Admin: create a channel.
    CreateChannel {
        name: String,
        channel_type: String,
    },
    /// Admin: delete a channel.
    DeleteChannel {
        channel_id: String,
    },
    /// Admin: force MEK rotation.
    RotateMEK,
    /// Admin: rename a channel.
    RenameChannel {
        channel_id: String,
        new_name: String,
    },
    /// Admin: update community metadata (name, description).
    UpdateCommunity {
        name: Option<String>,
        description: Option<String>,
    },
    /// Admin: ban a member (kick + prevent rejoin).
    Ban {
        target_pseudonym: String,
    },
    /// Admin: unban a member.
    Unban {
        target_pseudonym: String,
    },
    /// Admin: get ban list.
    GetBanList,

    // ── New role & permission management ──

    /// Create a new role.
    CreateRole {
        name: String,
        color: u32,
        permissions: u64,
        hoist: bool,
        mentionable: bool,
    },
    /// Edit an existing role.
    EditRole {
        role_id: u32,
        name: Option<String>,
        color: Option<u32>,
        permissions: Option<u64>,
        position: Option<i32>,
        hoist: Option<bool>,
        mentionable: Option<bool>,
    },
    /// Delete a role.
    DeleteRole {
        role_id: u32,
    },
    /// Assign a role to a member (additive — does not remove other roles).
    AssignRole {
        target_pseudonym: String,
        role_id: u32,
    },
    /// Remove a role from a member.
    UnassignRole {
        target_pseudonym: String,
        role_id: u32,
    },
    /// Set a channel permission overwrite.
    SetChannelOverwrite {
        channel_id: String,
        target_type: String, // "role" or "member"
        target_id: String,
        allow: u64,
        deny: u64,
    },
    /// Delete a channel permission overwrite.
    DeleteChannelOverwrite {
        channel_id: String,
        target_type: String,
        target_id: String,
    },
    /// Timeout a member (prevent sending for a duration).
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        reason: Option<String>,
    },
    /// Remove a member's timeout.
    RemoveTimeout {
        target_pseudonym: String,
    },
    /// Get all role definitions.
    GetRoles,
}

/// Response from the community server to a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityResponse {
    /// Generic success.
    Ok,
    /// Join succeeded — includes encrypted MEK and channel list.
    Joined {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        channels: Vec<ChannelInfoDto>,
        role_ids: Vec<u32>,
        roles: Vec<RoleDto>,
    },
    /// Message history.
    Messages {
        messages: Vec<ChannelMessageDto>,
    },
    /// MEK delivery.
    MEK {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
    },
    /// Channel created.
    ChannelCreated {
        channel_id: String,
    },
    /// Community metadata updated.
    CommunityUpdated,
    /// Ban list response.
    BanList {
        banned: Vec<BannedMemberDto>,
    },
    /// Role created successfully.
    RoleCreated {
        role_id: u32,
    },
    /// List of all roles.
    RolesList {
        roles: Vec<RoleDto>,
    },
    /// Error.
    Error {
        code: u32,
        message: String,
    },
}

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
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: u64,
}

/// Channel info as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
}

/// Broadcast from the community server to members via `app_message`.
///
/// Used for real-time notifications like new messages and MEK rotations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityBroadcast {
    /// A new message was posted in a channel.
    NewMessage {
        community_id: String,
        channel_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
    },
    /// MEK has been rotated — fetch your new copy via `RequestMEK`.
    MEKRotated {
        community_id: String,
        new_generation: u64,
    },
    /// A member joined the community.
    MemberJoined {
        community_id: String,
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
    },
    /// A member left or was kicked.
    MemberRemoved {
        community_id: String,
        pseudonym_key: String,
    },
    /// A role was created, updated, or deleted.
    RolesChanged {
        community_id: String,
        roles: Vec<RoleDto>,
    },
    /// A member's roles were changed.
    MemberRolesChanged {
        community_id: String,
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },
    /// A member was timed out or timeout was removed.
    MemberTimedOut {
        community_id: String,
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },
    /// Channel permission overwrites changed.
    ChannelOverwriteChanged {
        community_id: String,
        channel_id: String,
    },
}
