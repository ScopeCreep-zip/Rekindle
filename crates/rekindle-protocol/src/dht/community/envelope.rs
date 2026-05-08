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
    /// Watch Relay (architecture §14.3 / §11.7): a member with an active
    /// Veilid `watch_dht_values` slot relays a `ValueChange` notification
    /// to peers via gossip, so members without watch slots still learn
    /// when a record's subkey changes. The receiver is expected to
    /// `get_dht_value` to fetch the new value (we deliberately do NOT
    /// carry ciphertext over gossip — same Chiral Network principle as
    /// `MessageNotification`).
    WatchRelay {
        /// Hex-encoded Veilid record key whose subkey changed.
        record_key: String,
        /// Subkey index that changed.
        subkey: u32,
        /// blake3 hash of the new value (integrity check after fetch).
        content_hash: String,
        /// Sender's pseudonym (for permission/audit).
        observer_pseudonym: String,
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
        members: Vec<crate::dht::community::types::MemberSummary>,
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
        /// A3/P1.3 — cascade index for responder fall-through. `0` means the
        /// deterministic top-rank responder; receivers compare against
        /// `cascade_candidates(requester, members)[cascade_index]` and only
        /// reply if they're the candidate at that rank. Requester increments
        /// after each 5-second timeout so the next-best candidate takes over
        /// when the elected responder is offline. Default `0` keeps the wire
        /// format backward-compatible with peers that pre-date this field.
        #[serde(default)]
        cascade_index: u32,
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
    /// P1.3 — requester → responder ack confirming successful
    /// `MekTransfer` ingestion. Sent as the `app_call` reply by the
    /// MekTransfer receiver after MEK unwrap succeeds. Lets the
    /// responder distinguish app-layer success (decryption worked)
    /// from network-layer success (packet arrived) so a misrouted or
    /// generation-mismatched transfer surfaces in observability
    /// rather than disappearing into a synchronous `ACK`.
    ///
    /// Wire format: Cap'n Proto `MekTransferAckPayload` (ordinal 67 in
    /// `community_envelope.capnp`). Forward-compatible: peers built
    /// before this variant existed see `Which::NotInSchema` and fall
    /// back to treating the reply as a bare ACK.
    MekTransferAck {
        community_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        generation: u64,
        /// Hex pseudonym of the receiver (us, the requester) — lets
        /// the responder match the ack against in-flight transfers
        /// indexed by `(community_id, channel_id, generation,
        /// requester_pseudonym)`.
        requester_pseudonym: String,
    },
    /// P4.3 — joiner → admins gossip request to expand the community's
    /// Plate Gate segments. Issued by `claim_registry_slot` when every
    /// slot in the highest existing segment is occupied AND the joiner
    /// lacks `MANAGE_COMMUNITY`. Admins receiving this gossip
    /// invoke `expand_community_segment` which emits a `SegmentAdded`
    /// governance entry; the joiner watches for the new segment via
    /// the merged governance state and retries slot claim.
    ///
    /// Wire format: Cap'n Proto `RequestSegmentExpansionPayload`
    /// (ordinal 68 in `community_envelope.capnp`).
    RequestSegmentExpansion {
        community_id: String,
        requester_pseudonym: String,
        /// Highest segment_index the joiner saw as full when the
        /// request was issued. Admins use this to dedup concurrent
        /// expansion requests — only the first admin's
        /// `expand_community_segment` call lands; later receivers see
        /// `next_segment_index > full_segment_index + 1` in their
        /// merged governance state and no-op.
        full_segment_index: u32,
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
    EventCreated { event: rekindle_types::event::EventInfo },
    /// Broadcast: event updated.
    EventUpdated { event: rekindle_types::event::EventInfo },
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
    ThreadCreated {
        thread: rekindle_types::thread::ThreadInfo,
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
    ThreadArchived { thread_id: String, archived: bool },

    // ── Game servers ──
    /// Broadcast: game server added.
    GameServerAdded {
        server: rekindle_types::game_server::GameServerInfo,
    },
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
        governance_entries: Vec<rekindle_types::governance::GovernanceEntry>,
        /// Online members with presence data and route blobs.
        member_list: Vec<rekindle_types::member::MemberInfo>,
        /// Current MEK per channel, wrapped for the joiner's pseudonym.
        channel_meks: Vec<rekindle_types::mek::ChannelMekDelivery>,
        /// Last 50 messages per channel (MEK-encrypted ciphertext),
        /// grouped by channel id (architecture §13.4 line 2068).
        recent_messages: Vec<rekindle_types::message::BootstrapChannelMessages>,
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
        messages: Vec<rekindle_types::message::SyncedMessage>,
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
    /// Broadcast: stage speaker list/topic update.
    StageUpdate {
        channel_id: String,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator_pseudonym: String,
        lamport: u64,
    },
    /// Audience member requests permission to speak in a stage channel.
    SpeakRequest {
        channel_id: String,
        requester_pseudonym: String,
        lamport: u64,
    },
    /// Moderator response to a stage speak request.
    SpeakResponse {
        channel_id: String,
        requester_pseudonym: String,
        granted: bool,
        moderator_pseudonym: String,
        lamport: u64,
    },

    /// Lost Cargo file request (architecture §28.9 line 3252-3256).
    /// Sent via gossip to broadcast that a peer wants specific chunks of an
    /// attachment. Sources reply via `app_call` with `AttachmentChunk`
    /// payloads (one chunk per call).
    RequestAttachment {
        channel_id: String,
        attachment_id: [u8; 16],
        /// Chunk indices the requester needs. Empty list = "I'll take any
        /// chunks you have that I don't" (responder consults bitmap
        /// intersection).
        requested_chunks: Vec<u32>,
        requester_pseudonym: String,
    },

    /// Lost Cargo chunk delivery (architecture §28.9 line 3257-3265). The
    /// payload of the `app_call` reply when responding to
    /// `RequestAttachment`. Receiver verifies the SHA-256 plaintext hash
    /// after FEK decryption (plan §1.J2).
    AttachmentChunk {
        attachment_id: [u8; 16],
        chunk_index: u32,
        /// Chunk ciphertext (FEK-encrypted). Receiver decrypts with the
        /// FEK that was wrapped under the channel MEK in the original offer.
        data: Vec<u8>,
        /// SHA-256 of the *plaintext* chunk for tamper detection on the
        /// transport path (defense-in-depth alongside the AES-GCM tag).
        plaintext_hash: [u8; 32],
    },

    /// Batched chunk-delivery reply: one or more `AttachmentChunk` entries
    /// returned in a single `app_call` response. The responder collects
    /// every chunk it has from the requested set, packs them here, and
    /// replies once — saves N round trips when one peer holds many chunks.
    MultiAttachmentChunk { chunks: Vec<ControlPayload> },

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
    /// Soundboard playback trigger (architecture §10.9). Broadcast in
    /// the voice channel's gossip mesh so every participant plays the
    /// expression's audio locally — the audio bytes themselves are
    /// already cached as inline expression assets per §18.4.
    SoundboardPlay {
        channel_id: String,
        /// Expression ID (16-byte UUID hex) of the sound asset.
        expression_id: String,
        /// Sender's pseudonym so receivers can attribute or mute per peer.
        actor_pseudonym: String,
    },

    /// Architecture §10.6 video / screen-share fragment. Frames are
    /// VP9-encoded, MEK-encrypted, then split into ≤28 KB chunks so
    /// they fit inside Veilid `app_message`. The 16-byte `stream_id`
    /// is `blake3(channel_id || sender_pseudonym)[..16]` so concurrent
    /// streams in the same channel never collide. Reassembly happens
    /// in `rekindle-video::Reassembler` on the receive side.
    VideoFragment {
        channel_id: String,
        stream_id: [u8; 16],
        frame_seq: u32,
        frag_index: u8,
        frag_total: u8,
        keyframe: bool,
        timestamp: u32,
        payload: Vec<u8>,
        signature: Vec<u8>,
    },

    /// Forward-error-correction parity packet for the matching
    /// `VideoFragment` data stream (architecture §10.6 line 4080).
    /// Modelled on RFC 5109 / FlexFEC: a separate packet variant
    /// carrying Reed-Solomon parity shards so a few dropped data
    /// fragments don't force a full keyframe re-request. Receivers
    /// without FEC support ignore these.
    VideoParityFragment {
        channel_id: String,
        stream_id: [u8; 16],
        frame_seq: u32,
        parity_index: u8,
        parity_total: u8,
        data_count: u8,
        frame_len: u32,
        timestamp: u32,
        payload: Vec<u8>,
        signature: Vec<u8>,
    },

    /// Architecture §10.6 — receiver acknowledges fragments roughly
    /// every 500 ms and reports their measured downstream bandwidth.
    /// Senders adjust VP9 bitrate to match the slowest receiver.
    FrameAck {
        channel_id: String,
        stream_id: [u8; 16],
        last_frame_seq: u32,
        kbps: u32,
        loss_q8: u8,
    },

    /// Sent by a receiver who has lost too many inter-frames to keep
    /// rendering. Senders treat this as a request to encode the next
    /// frame as a keyframe (architecture §10.6).
    KeyframeRequest {
        channel_id: String,
        stream_id: [u8; 16],
    },

    /// Bandwidth advertisement decoupled from `FrameAck` — used when
    /// network conditions change without a frame in flight (e.g.
    /// Wi-Fi → cellular hand-off).
    BandwidthEstimate {
        channel_id: String,
        kbps: u32,
        window_secs: u8,
        loss_q8: u8,
    },

    /// Capability negotiation broadcast on join. Senders union the
    /// reported caps and pick a resolution + framerate every receiver
    /// can decode (architecture §10.6 line 4084).
    MediaCapabilities {
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        codecs: Vec<String>,
    },

    /// Architecture §10.6 + Phase 6 Week 22 — broadcast that the
    /// active video relay for a `(channel_id, stream_id)` pair has
    /// changed. Receivers switch their fetch source to
    /// `relay_host_pseudonym` and reset their reassembler for that
    /// stream so they don't pile up partial frames from the old relay.
    /// `relay_host_pseudonym = None` signals a transition to direct
    /// mesh delivery (used when participant count drops below the
    /// relay threshold or the previous relay went offline with no
    /// successor yet).
    TopologyChange {
        channel_id: String,
        stream_id: [u8; 16],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relay_host_pseudonym: Option<String>,
        /// Free-form reason: `"initial"`, `"relay_left"`,
        /// `"relay_overloaded"`, `"explicit_request"`. Receivers
        /// surface this in logs only — behaviour is the same.
        reason: String,
        lamport: u64,
    },

    // ── Link Previews (architecture §28.8) ──
    /// Sender's pre-fetched OpenGraph metadata for a URL embedded in
    /// `message_id`. Receivers display inline; gated reader-side by
    /// the sender's `EMBED_LINKS` permission.
    LinkPreview {
        channel_id: String,
        message_id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site_name: Option<String>,
        fetched_at: u64,
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
    use crate::capnp_envelope::{encode_community_envelope, encode_signed_envelope};

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
        // Wire-format encoding matches the live gossip path (Cap'n
        // Proto, not JSON) so the signature applies to bytes that the
        // production code actually sends.
        let envelope_bytes = encode_community_envelope(&envelope).unwrap();

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
    fn envelope_message_notification_capnp_roundtrip() {
        let envelope = CommunityEnvelope::MessageNotification {
            channel_id: "ch_01".into(),
            message_id: "msg_abc".into(),
            author_pseudonym: "pseudo_123".into(),
            subkey_index: 7,
            lamport_ts: 42,
            sequence: 7,
            content_hash: "abc123".into(),
            timestamp: 1_234_567_890,
        };
        let bytes = encode_community_envelope(&envelope).unwrap();
        let back = crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::MessageNotification { channel_id, .. } => {
                assert_eq!(channel_id, "ch_01");
            }
            _ => panic!("wrong variant"),
        }
    }

    /// Regression guard: MessageNotification must NEVER contain a "ciphertext" field
    /// in the typed Rust enum. Gossip carries the cargo manifest (metadata), not the
    /// cargo (ciphertext). Ciphertext exists only on DHT storage nodes (5 replicas),
    /// not across the entire gossip fan-out graph.
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
            timestamp: 1_234_567_890,
        };
        // Debug-format inspection: `Debug` derives the field names the
        // type really has. If a `ciphertext` field is ever added to
        // `MessageNotification`, this regression catches it.
        let debug = format!("{envelope:?}");
        assert!(
            !debug.contains("ciphertext"),
            "MessageNotification must NOT contain ciphertext — gossip carries \
             notifications only. Got: {debug}"
        );
    }

    /// Regression guard: `MessageNotification` packs compactly on the
    /// wire. Cap'n Proto packed format keeps this notification well
    /// under the Veilid 32 KB `app_message` limit; if ciphertext
    /// sneaks in, this limit fails immediately.
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
            timestamp: 1_234_567_890,
        };
        let bytes = encode_community_envelope(&envelope).unwrap();
        assert!(
            bytes.len() < 200,
            "MessageNotification should be compact (< 200 bytes packed), was {} bytes. \
             If this fails, check if ciphertext or large fields were added.",
            bytes.len()
        );
    }

    #[test]
    fn control_payload_capnp_roundtrip() {
        let payload = ControlPayload::MemberLeave {
            pseudonym_key: "abc123".into(),
        };
        let bytes =
            encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
        let back =
            crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::Control(ControlPayload::MemberLeave { pseudonym_key }) => {
                assert_eq!(pseudonym_key, "abc123");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mek_transfer_ack_capnp_roundtrip() {
        // P1.3 — verify the new MekTransferAck variant survives Cap'n
        // Proto encode/decode with all fields preserved including the
        // optional channel_id (hasChannelId boolean toggle).
        let payload = ControlPayload::MekTransferAck {
            community_id: "veilid:abc".into(),
            channel_id: Some("ch_42".into()),
            generation: 17,
            requester_pseudonym: "deadbeef".repeat(4), // 32-hex-char pseudonym
        };
        let bytes =
            encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
        let back =
            crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::Control(ControlPayload::MekTransferAck {
                community_id,
                channel_id,
                generation,
                requester_pseudonym,
            }) => {
                assert_eq!(community_id, "veilid:abc");
                assert_eq!(channel_id.as_deref(), Some("ch_42"));
                assert_eq!(generation, 17);
                assert_eq!(requester_pseudonym, "deadbeef".repeat(4));
            }
            _ => panic!("wrong variant"),
        }

        // Also verify the channel_id == None path round-trips cleanly
        // — this is the community-wide MEK case.
        let payload_none = ControlPayload::MekTransferAck {
            community_id: "veilid:xyz".into(),
            channel_id: None,
            generation: 99,
            requester_pseudonym: "cafebabe".repeat(4),
        };
        let bytes =
            encode_community_envelope(&CommunityEnvelope::Control(payload_none)).unwrap();
        let back =
            crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::Control(ControlPayload::MekTransferAck {
                channel_id,
                generation,
                ..
            }) => {
                assert!(channel_id.is_none(), "None channel_id must round-trip");
                assert_eq!(generation, 99);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn request_segment_expansion_capnp_roundtrip() {
        // P4.3 — RequestSegmentExpansion variant must round-trip
        // cleanly through the Cap'n Proto codec.
        let payload = ControlPayload::RequestSegmentExpansion {
            community_id: "veilid:plate".into(),
            requester_pseudonym: "feedface".repeat(4),
            full_segment_index: 0,
        };
        let bytes =
            encode_community_envelope(&CommunityEnvelope::Control(payload)).unwrap();
        let back =
            crate::capnp_envelope::decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::Control(ControlPayload::RequestSegmentExpansion {
                community_id,
                requester_pseudonym,
                full_segment_index,
            }) => {
                assert_eq!(community_id, "veilid:plate");
                assert_eq!(requester_pseudonym, "feedface".repeat(4));
                assert_eq!(full_segment_index, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn signed_envelope_capnp_roundtrip() {
        let signed = SignedEnvelope {
            community_id: "comm_01".into(),
            sender_pseudonym: "abc123".into(),
            envelope_bytes: vec![1, 2, 3],
            signature: vec![0u8; 64],
            ttl: 5,
        };
        let bytes = encode_signed_envelope(&signed);
        let back = crate::capnp_envelope::decode_signed_envelope(&bytes).unwrap();
        assert_eq!(back.community_id, "comm_01");
    }
}
