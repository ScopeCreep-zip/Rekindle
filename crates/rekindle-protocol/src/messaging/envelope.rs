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
    },
    /// Friend request acceptance.
    FriendAccept { prekey_bundle: Vec<u8> },
    /// Friend request rejection.
    FriendReject,
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
