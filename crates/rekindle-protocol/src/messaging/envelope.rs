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
// Community server RPC types
// ---------------------------------------------------------------------------

/// Request from a member to the community server (sent via `app_call`).
///
/// Wrapped in a `MessageEnvelope` signed by the member's pseudonym key.
/// The server verifies the signature and extracts `sender_key` to identify
/// which pseudonym is making the request.
///
/// **Note:** `rename_all = "camelCase"` is intentionally omitted — this enum
/// is serialized for Rust↔Rust RPC over Veilid (JSON via `serde_json`), never
/// sent to JavaScript. Both the client and server deserialize with the same
/// serde config, so PascalCase variant names work correctly on both sides.
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
    },
    /// Edit a previously sent message.
    EditMessage {
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
    },
    /// Delete a message (own messages, or any with MANAGE_MESSAGES permission).
    DeleteMessage {
        channel_id: String,
        message_id: String,
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
    Kick { target_pseudonym: String },
    /// Admin: create a channel.
    CreateChannel {
        name: String,
        channel_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category_id: Option<String>,
    },
    /// Admin: delete a channel.
    DeleteChannel { channel_id: String },
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
    Ban { target_pseudonym: String },
    /// Admin: unban a member.
    Unban { target_pseudonym: String },
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
    DeleteRole { role_id: u32 },
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
    RemoveTimeout { target_pseudonym: String },
    /// Get all role definitions.
    GetRoles,

    // ── Category management ──
    /// Create a channel category.
    CreateCategory { name: String },
    /// Delete a channel category.
    DeleteCategory { category_id: String },
    /// Rename a channel category.
    RenameCategory { category_id: String, new_name: String },
    /// Move a channel to a different category (or out of any category).
    MoveChannel { channel_id: String, category_id: Option<String> },
    /// Reorder categories.
    ReorderCategories { category_ids: Vec<String> },

    // ── Invite management ──
    /// Create a community invite code.
    CreateInvite {
        max_uses: Option<u32>,
        expires_in_seconds: Option<u64>,
    },
    /// Revoke an invite code.
    RevokeInvite { code: String },
    /// List active invite codes.
    ListInvites,

    // ── Reactions ──
    /// Add a reaction to a message.
    AddReaction {
        channel_id: String,
        message_id: String,
        emoji: String,
    },
    /// Remove a reaction from a message.
    RemoveReaction {
        channel_id: String,
        message_id: String,
        emoji: String,
    },

    // ── Pinning ──
    /// Pin a message in a channel.
    PinMessage {
        channel_id: String,
        message_id: String,
    },
    /// Unpin a message from a channel.
    UnpinMessage {
        channel_id: String,
        message_id: String,
    },
    /// Get pinned messages for a channel.
    GetPins {
        channel_id: String,
    },

    // ── Audit log ──
    /// Get audit log entries.
    GetAuditLog {
        before_timestamp: Option<u64>,
        limit: u32,
    },

    // ── Channel topic & reordering ──
    /// Set a channel's topic/description.
    SetChannelTopic {
        channel_id: String,
        topic: String,
    },
    /// Reorder channels within their category.
    ReorderChannels { channel_ids: Vec<String> },

    // ── Slowmode ──
    /// Set slowmode delay for a channel (0 to disable).
    SetSlowmode {
        channel_id: String,
        seconds: u32,
    },

    // ── Typing & Presence ──
    /// Indicate typing in a channel (ephemeral, not stored).
    ChannelTyping { channel_id: String },
    /// Update member presence status.
    UpdatePresence {
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_seconds: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_address: Option<String>,
    },

    // ── Events ──
    /// Create a community event.
    CreateEvent {
        title: String,
        description: String,
        start_time: u64,
        end_time: Option<u64>,
        channel_id: Option<String>,
        max_attendees: Option<u32>,
    },
    /// Edit an existing event.
    ///
    /// All optional fields use `None` = "don't change", `Some(value)` = "set to this".
    /// To clear a nullable field, send its zero/empty sentinel:
    ///   - `end_time: Some(0)` clears the end time
    ///   - `channel_id: Some("")` unlinks the channel
    ///   - `max_attendees: Some(0)` removes the attendee cap
    EditEvent {
        event_id: String,
        title: Option<String>,
        description: Option<String>,
        start_time: Option<u64>,
        end_time: Option<u64>,
        channel_id: Option<String>,
        max_attendees: Option<u32>,
    },
    /// Delete an event.
    DeleteEvent { event_id: String },
    /// RSVP to an event.
    RsvpEvent {
        event_id: String,
        status: String,
    },
    /// Cancel an event (sets status to "canceled").
    CancelEvent { event_id: String },
    /// Get upcoming events.
    GetEvents,

    // ── Threads ──
    /// Create a thread from a message.
    CreateThread {
        channel_id: String,
        name: String,
        starter_message_id: String,
    },
    /// Get threads in a channel.
    GetChannelThreads { channel_id: String },
    /// Send a message to a thread.
    SendThreadMessage {
        thread_id: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        reply_to_id: Option<String>,
    },
    /// Get thread message history.
    GetThreadMessages {
        thread_id: String,
        limit: u32,
        before_timestamp: Option<u64>,
    },
    /// Archive a thread.
    ArchiveThread { thread_id: String },
    /// Unarchive a thread.
    UnarchiveThread { thread_id: String },

    // ── Game Server Favorites ──
    /// Add a game server to the community's favorites list.
    AddGameServer {
        game_id: String,
        label: String,
        address: String,
    },
    /// Remove a game server from the community's favorites list.
    RemoveGameServer { server_id: String },
    /// Get all game servers for this community.
    GetGameServers,

    // ── Unread tracking ──
    /// Mark a channel as read up to a specific message.
    MarkChannelRead {
        channel_id: String,
        last_message_id: String,
    },
    /// Get unread counts for all channels in the community.
    GetUnreadCounts,
}

/// Response from the community server to a member.
///
/// **Note:** `rename_all = "camelCase"` is intentionally omitted — this enum
/// is serialized for Rust↔Rust RPC over Veilid, not Rust→JS Tauri IPC.
/// See [`CommunityRequest`] for details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityResponse {
    /// Generic success.
    Ok,
    /// Join succeeded — includes encrypted MEK, channel list, categories, and members.
    Joined {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        channels: Vec<ChannelInfoDto>,
        #[serde(default)]
        categories: Vec<CategoryDto>,
        role_ids: Vec<u32>,
        roles: Vec<RoleDto>,
        #[serde(default)]
        members: Vec<MemberInfoDto>,
    },
    /// Message history.
    Messages { messages: Vec<ChannelMessageDto> },
    /// MEK delivery.
    Mek {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
    },
    /// Message sent successfully.
    MessageSent { message_id: String, timestamp: u64 },
    /// Channel created.
    ChannelCreated { channel_id: String },
    /// Community metadata updated.
    CommunityUpdated,
    /// Ban list response.
    BanList { banned: Vec<BannedMemberDto> },
    /// Role created successfully.
    RoleCreated { role_id: u32 },
    /// List of all roles.
    RolesList { roles: Vec<RoleDto> },
    /// Category created.
    CategoryCreated { category_id: String },
    /// Invite code created, with server's Ed25519 signature of the code.
    InviteCreated { code: String, signature: String },
    /// List of active invites.
    InviteList { invites: Vec<InviteDto> },
    /// Pinned messages list.
    PinnedMessages { pins: Vec<PinnedMessageDto> },
    /// Audit log entries.
    AuditLog { entries: Vec<AuditLogEntryDto> },
    /// Event created.
    EventCreated { event_id: String },
    /// List of events.
    EventList { events: Vec<EventDto> },
    /// Thread created.
    ThreadCreated { thread_id: String },
    /// List of threads in a channel.
    ThreadList { threads: Vec<ThreadInfoDto> },
    /// Thread message history.
    ThreadMessages { messages: Vec<ChannelMessageDto> },
    /// Game server favorites list.
    GameServerList { servers: Vec<GameServerDto> },
    /// Unread counts for all channels.
    UnreadCounts { counts: Vec<UnreadCountDto> },
    /// Error.
    Error { code: u32, message: String },
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

/// Broadcast from the community server to members via `app_message`.
///
/// Used for real-time notifications like new messages and MEK rotations.
///
/// **Note:** `rename_all = "camelCase"` is intentionally omitted — this enum
/// is serialized for Rust↔Rust communication over Veilid, not Rust→JS Tauri IPC.
/// The Tauri→frontend bridge uses [`CommunityEvent`](crate::channels::community_channel::CommunityEvent),
/// which does have `rename_all = "camelCase"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommunityBroadcast {
    /// A new message was posted in a channel.
    NewMessage {
        community_id: String,
        channel_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
    },
    /// A message was edited.
    MessageEdited {
        community_id: String,
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
        edited_at: u64,
    },
    /// A message was deleted.
    MessageDeleted {
        community_id: String,
        channel_id: String,
        message_id: String,
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
    /// A reaction was added to a message.
    ReactionAdded {
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// A reaction was removed from a message.
    ReactionRemoved {
        community_id: String,
        channel_id: String,
        message_id: String,
        emoji: String,
        reactor_pseudonym: String,
    },
    /// A message was pinned.
    MessagePinned {
        community_id: String,
        channel_id: String,
        message_id: String,
        pinned_by: String,
    },
    /// A message was unpinned.
    MessageUnpinned {
        community_id: String,
        channel_id: String,
        message_id: String,
    },
    /// A member started typing in a channel.
    ChannelTyping {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    /// A member's presence status changed.
    MemberPresenceChanged {
        community_id: String,
        pseudonym_key: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_seconds: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_address: Option<String>,
    },
    /// An event was created.
    EventCreated {
        community_id: String,
        event: EventDto,
    },
    /// An event was updated.
    EventUpdated {
        community_id: String,
        event: EventDto,
    },
    /// An event was deleted.
    EventDeleted {
        community_id: String,
        event_id: String,
    },
    /// Someone RSVPed to an event.
    EventRsvpChanged {
        community_id: String,
        event_id: String,
        pseudonym_key: String,
        status: String,
    },
    /// A thread was created.
    ThreadCreated {
        community_id: String,
        thread: ThreadInfoDto,
    },
    /// A message was sent in a thread.
    ThreadMessageReceived {
        community_id: String,
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        reply_to_id: Option<String>,
    },
    /// A thread was archived or unarchived.
    ThreadArchived {
        community_id: String,
        thread_id: String,
        archived: bool,
    },
    /// A game server was added to the community's favorites.
    GameServerAdded {
        community_id: String,
        server: GameServerDto,
    },
    /// A game server was removed from the community's favorites.
    GameServerRemoved {
        community_id: String,
        server_id: String,
    },
    /// Event starting soon reminder.
    EventReminder {
        community_id: String,
        event_id: String,
        title: String,
        minutes_until_start: u32,
    },
    /// Channel or category structure was modified (create, delete, rename, move, reorder, topic, slowmode).
    /// Carries the full updated channel + category state so receivers can replace their local view.
    ChannelsUpdated {
        community_id: String,
        channels: Vec<ChannelInfoDto>,
        categories: Vec<CategoryDto>,
    },
    /// An invite code was created.
    InviteCreated {
        community_id: String,
        code: String,
        created_by: String,
        max_uses: Option<u32>,
        uses: u32,
        expires_at: Option<u64>,
        created_at: u64,
    },
    /// An invite code was revoked.
    InviteRevoked {
        community_id: String,
        code: String,
    },
    /// An invite was used (uses counter incremented).
    InviteUsed {
        community_id: String,
        code: String,
        new_use_count: u32,
    },
}
