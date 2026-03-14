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
    /// A chat message in a channel (MEK-encrypted content).
    ChatMessage {
        channel_id: String,
        message_id: String,
        author_pseudonym: String,
        /// MEK-encrypted content (end-to-end encrypted).
        ciphertext: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
        /// Lamport logical timestamp for causal ordering.
        #[serde(default)]
        lamport_ts: u64,
        /// Per-sender, per-channel sequence number for gap detection.
        #[serde(default)]
        sequence: u64,
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

/// Control payload covering all non-chat operations.
///
/// Each variant maps 1:1 to an existing `CommunityRequest` variant.
/// Includes both request-type variants (from members) and response-type
/// variants (from admin peers to members).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ControlPayload {
    // ── Coordinator lifecycle ──
    /// Coordinator heartbeat (written to manifest, also broadcast).
    CoordinatorHeartbeat {
        epoch: u64,
        timestamp: u64,
        route_blob: Vec<u8>,
    },
    /// Election claim broadcast by a candidate.
    ElectionClaim {
        epoch: u64,
        pseudonym_key: String,
        score: Vec<u8>,
        route_blob: Vec<u8>,
    },

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
    MemberLeave {
        pseudonym_key: String,
    },
    /// Response: join accepted by admin peer.
    JoinAccepted {
        mek_encrypted: Vec<u8>,
        mek_generation: u64,
        channels: Vec<serde_json::Value>,
        #[serde(default)]
        categories: Vec<serde_json::Value>,
        role_ids: Vec<u32>,
        roles: Vec<serde_json::Value>,
        #[serde(default)]
        members: Vec<serde_json::Value>,
        /// The member registry DHT record key — needed for elections and presence.
        #[serde(default)]
        member_registry_key: Option<String>,
        /// Wrapped channel log keypairs: `[(channel_id, log_key, wrapped_keypair_bytes)]`.
        /// Each keypair is encrypted for the joining member's pseudonym public key
        /// using the same `wrap_mek()` envelope (X25519 DH + ChaCha20-Poly1305).
        #[serde(default)]
        channel_log_keypairs: Vec<(String, String, Vec<u8>)>,
        /// Slot index for the joiner in the member registry SMPL record.
        #[serde(default)]
        slot_index: Option<u32>,
        /// Wrapped slot seed (ECDH-encrypted) — allows the joiner to derive
        /// their own slot keypair locally via `derive_slot_veilid_keypair(seed, slot_index)`.
        /// This eliminates any coordinator dependency for presence writing.
        #[serde(default)]
        wrapped_slot_seed: Option<Vec<u8>>,
        /// Legacy: wrapped slot keypair. Kept for backward compat with older coordinators.
        #[serde(default)]
        wrapped_slot_keypair: Option<Vec<u8>>,
    },
    /// Response: join rejected by admin peer.
    JoinRejected {
        reason: String,
    },
    /// Broadcast: a member joined.
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<u32>,
    },
    /// Broadcast: a member was removed (left, kicked, or banned).
    MemberRemoved {
        pseudonym_key: String,
    },

    // ── Moderation ──
    /// Kick a member.
    Kick {
        target_pseudonym: String,
    },
    /// Ban a member.
    Ban {
        target_pseudonym: String,
    },
    /// Unban a member.
    Unban {
        target_pseudonym: String,
    },
    /// Get ban list.
    GetBanList,
    /// Timeout a member.
    TimeoutMember {
        target_pseudonym: String,
        duration_seconds: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Remove a member's timeout.
    RemoveTimeout {
        target_pseudonym: String,
    },
    /// Broadcast: member timed out.
    MemberTimedOut {
        pseudonym_key: String,
        timeout_until: Option<u64>,
    },

    // ── Messages ──
    /// Edit a previously sent message.
    EditMessage {
        channel_id: String,
        message_id: String,
        new_ciphertext: Vec<u8>,
        mek_generation: u64,
    },
    /// Delete a message.
    DeleteMessage {
        channel_id: String,
        message_id: String,
    },
    /// Fetch message history.
    GetMessages {
        channel_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_timestamp: Option<u64>,
        limit: u32,
    },
    /// Response: message history.
    Messages {
        messages: Vec<serde_json::Value>,
    },
    /// Response: message sent confirmation.
    MessageSent {
        message_id: String,
        timestamp: u64,
    },
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
    /// Request current MEK.
    RequestMEK,
    /// Request SlotKeypairGrant — sent when a member is missing their slot keypair
    /// (e.g. it was never delivered or lost). Any admin peer responds with SlotKeypairGrant.
    /// Includes the requester's route_blob so the admin can respond directly
    /// (without the route, the admin can't deliver the grant because the
    /// requester can't write DHT presence without the slot keypair — chicken-and-egg).
    RequestSlotKeypair {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
    },
    /// Rotate MEK.
    RotateMEK,
    /// Broadcast: MEK rotated.
    MEKRotated {
        new_generation: u64,
    },
    /// Broadcast: member completed onboarding.
    OnboardingComplete {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel management ──
    /// Create a channel.
    CreateChannel {
        name: String,
        channel_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category_id: Option<String>,
    },
    /// Delete a channel.
    DeleteChannel {
        channel_id: String,
    },
    /// Rename a channel.
    RenameChannel {
        channel_id: String,
        new_name: String,
    },
    /// Set channel topic.
    SetChannelTopic {
        channel_id: String,
        topic: String,
    },
    /// Reorder channels.
    ReorderChannels {
        channel_ids: Vec<String>,
    },
    /// Set slowmode for a channel.
    SetSlowmode {
        channel_id: String,
        seconds: u32,
    },
    /// Move a channel to a different category.
    MoveChannel {
        channel_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category_id: Option<String>,
    },
    /// Broadcast: channel structure updated.
    ChannelsUpdated {
        channels: Vec<serde_json::Value>,
        categories: Vec<serde_json::Value>,
    },

    // ── Category management ──
    /// Create a category.
    CreateCategory {
        name: String,
    },
    /// Delete a category.
    DeleteCategory {
        category_id: String,
    },
    /// Rename a category.
    RenameCategory {
        category_id: String,
        new_name: String,
    },
    /// Reorder categories.
    ReorderCategories {
        category_ids: Vec<String>,
    },

    // ── Role management ──
    /// Create a role.
    CreateRole {
        name: String,
        color: u32,
        permissions: u64,
        hoist: bool,
        mentionable: bool,
    },
    /// Edit a role.
    EditRole {
        role_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hoist: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mentionable: Option<bool>,
    },
    /// Delete a role.
    DeleteRole {
        role_id: u32,
    },
    /// Assign a role to a member.
    AssignRole {
        target_pseudonym: String,
        role_id: u32,
    },
    /// Remove a role from a member.
    UnassignRole {
        target_pseudonym: String,
        role_id: u32,
    },
    /// Get role definitions.
    GetRoles,
    /// Broadcast: roles changed.
    RolesChanged {
        roles: Vec<serde_json::Value>,
    },
    /// Broadcast: member roles changed.
    MemberRolesChanged {
        pseudonym_key: String,
        role_ids: Vec<u32>,
    },

    // ── Channel permission overwrites ──
    /// Set a channel permission overwrite.
    SetChannelOverwrite {
        channel_id: String,
        target_type: String,
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
    /// Broadcast: channel overwrite changed.
    ChannelOverwriteChanged {
        channel_id: String,
    },

    // ── Community metadata ──
    /// Update community metadata.
    UpdateCommunity {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },

    // ── Invite management ──
    /// Create an invite with embedded encrypted secrets for self-service joining.
    CreateInvite {
        /// SHA-256 hash of the invite code (hex). Raw code never sent over gossip.
        code_hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_seconds: Option<u64>,
        /// Encrypted `InviteSecrets` blob (base64). Stored alongside invite metadata.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_secrets: Option<String>,
    },
    /// Revoke an invite by its code hash.
    RevokeInvite {
        code_hash: String,
    },
    /// List invites.
    ListInvites,
    /// Broadcast: invite created.
    InviteCreated {
        code_hash: String,
        created_by: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        uses: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<u64>,
        created_at: u64,
    },
    /// Broadcast: invite revoked.
    InviteRevoked {
        code_hash: String,
    },
    /// Broadcast: invite used.
    InviteUsed {
        code_hash: String,
        new_use_count: u32,
    },

    // ── Reactions ──
    /// Add a reaction.
    AddReaction {
        channel_id: String,
        message_id: String,
        emoji: String,
    },
    /// Remove a reaction.
    RemoveReaction {
        channel_id: String,
        message_id: String,
        emoji: String,
    },
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
    /// Pin a message.
    PinMessage {
        channel_id: String,
        message_id: String,
    },
    /// Unpin a message.
    UnpinMessage {
        channel_id: String,
        message_id: String,
    },
    /// Get pinned messages.
    GetPins {
        channel_id: String,
    },
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

    // ── Audit log ──
    /// Get audit log entries.
    GetAuditLog {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_timestamp: Option<u64>,
        limit: u32,
    },

    // ── Events ──
    /// Create an event.
    CreateEvent {
        title: String,
        description: String,
        start_time: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_time: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_attendees: Option<u32>,
    },
    /// Edit an event.
    EditEvent {
        event_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_time: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_time: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_attendees: Option<u32>,
    },
    /// Delete an event.
    DeleteEvent {
        event_id: String,
    },
    /// RSVP to an event.
    RsvpEvent {
        event_id: String,
        status: String,
    },
    /// Cancel an event.
    CancelEvent {
        event_id: String,
    },
    /// Get events.
    GetEvents,
    /// Broadcast: event created.
    EventCreated {
        event: serde_json::Value,
    },
    /// Broadcast: event updated.
    EventUpdated {
        event: serde_json::Value,
    },
    /// Broadcast: event deleted.
    EventDeleted {
        event_id: String,
    },
    /// Broadcast: event RSVP changed.
    EventRsvpChanged {
        event_id: String,
        pseudonym_key: String,
        status: String,
    },

    // ── Threads ──
    /// Create a thread.
    CreateThread {
        channel_id: String,
        name: String,
        starter_message_id: String,
    },
    /// Get threads in a channel.
    GetChannelThreads {
        channel_id: String,
    },
    /// Send a thread message.
    SendThreadMessage {
        thread_id: String,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to_id: Option<String>,
    },
    /// Get thread messages.
    GetThreadMessages {
        thread_id: String,
        limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_timestamp: Option<u64>,
    },
    /// Archive a thread.
    ArchiveThread {
        thread_id: String,
    },
    /// Unarchive a thread.
    UnarchiveThread {
        thread_id: String,
    },
    /// Broadcast: thread created.
    ThreadCreated {
        thread: serde_json::Value,
    },
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
    ThreadArchived {
        thread_id: String,
        archived: bool,
    },

    // ── Game servers ──
    /// Add a game server.
    AddGameServer {
        game_id: String,
        label: String,
        address: String,
    },
    /// Remove a game server.
    RemoveGameServer {
        server_id: String,
    },
    /// Get game servers.
    GetGameServers,
    /// Broadcast: game server added.
    GameServerAdded {
        server: serde_json::Value,
    },
    /// Broadcast: game server removed.
    GameServerRemoved {
        server_id: String,
    },

    // ── Unread tracking ──
    /// Mark channel as read.
    MarkChannelRead {
        channel_id: String,
        last_message_id: String,
    },
    /// Get unread counts.
    GetUnreadCounts,

    // ── Presence ──
    /// Update member presence.
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
    /// Broadcast: member presence changed.
    MemberPresenceChanged {
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

    // ── Onboarding ──
    /// Response: onboarding questions sent to new member.
    OnboardingQuestions {
        questions: Vec<serde_json::Value>,
    },
    /// Submit onboarding answers.
    SubmitOnboardingAnswers {
        answers: Vec<OnboardingAnswer>,
    },

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
    /// Notification: message blocked by automod.
    AutoModBlocked {
        rule_name: String,
        reason: String,
    },
    /// Raid alert broadcast to all members (owners/admins should act).
    RaidAlert {
        active: bool,
    },
    /// Channel lockdown broadcast: non-admins should restrict sending.
    ChannelLockdown {
        locked: bool,
    },
    /// System message broadcast (join/leave/kick/ban events posted to chat feed).
    SystemMessage {
        body: String,
        timestamp: u64,
    },

    // ── Admin delegation ──
    /// Grant manifest keypair + slot seed to a newly promoted admin.
    AdminKeypairGrant {
        /// Manifest owner keypair encrypted for the target member.
        wrapped_manifest_keypair: Vec<u8>,
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

    // ── Coordinator announcement ──
    /// New coordinator announcement (replaces election claim for gossip mesh).
    CoordinatorAnnounce {
        pseudonym_key: String,
        route_blob: Vec<u8>,
        epoch: u64,
    },

    // ── Voice channel signaling ──
    /// Broadcast: member joined a voice channel.
    VoiceJoin {
        channel_id: String,
        /// Private route blob for receiving voice packets.
        route_blob: Vec<u8>,
    },
    /// Broadcast: member left a voice channel.
    VoiceLeave {
        channel_id: String,
    },
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
    /// Generic success response.
    Ok,
    /// Error response.
    Error {
        code: u32,
        message: String,
    },
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
    let pub_bytes = hex::decode(&signed.sender_pseudonym)
        .map_err(|e| format!("invalid pseudonym hex: {e}"))?;
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

        let signed = sign_envelope(&signing_key, "community_abc", &pseudonym_hex, &envelope_bytes);

        assert!(verify_envelope(&signed).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_data() {
        let secret = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = VerifyingKey::from(&signing_key);
        let pseudonym_hex = hex::encode(verifying_key.to_bytes());

        let envelope_bytes = b"original data";
        let mut signed =
            sign_envelope(&signing_key, "community_abc", &pseudonym_hex, envelope_bytes);

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
        let mut signed = sign_envelope(&signing_key, "community_abc", "placeholder", envelope_bytes);

        // Replace sender pseudonym with wrong key
        signed.sender_pseudonym = wrong_hex;

        assert!(verify_envelope(&signed).is_err());
    }

    #[test]
    fn envelope_chat_message_serde() {
        let envelope = CommunityEnvelope::ChatMessage {
            channel_id: "ch_01".into(),
            message_id: "msg_abc".into(),
            author_pseudonym: "pseudo_123".into(),
            ciphertext: vec![1, 2, 3],
            mek_generation: 1,
            timestamp: 1234567890,
            reply_to_id: None,
            lamport_ts: 42,
            sequence: 7,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let back: CommunityEnvelope = serde_json::from_str(&json).unwrap();
        match back {
            CommunityEnvelope::ChatMessage { channel_id, .. } => {
                assert_eq!(channel_id, "ch_01");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn control_payload_serde() {
        let payload = ControlPayload::CoordinatorHeartbeat {
            epoch: 5,
            timestamp: 1234567890,
            route_blob: vec![10, 20, 30],
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: ControlPayload = serde_json::from_str(&json).unwrap();
        match back {
            ControlPayload::CoordinatorHeartbeat { epoch, .. } => assert_eq!(epoch, 5),
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
