//! The community control-payload enum — every non-chat operation on
//! the mesh. The four largest variants carry their fields as named
//! payload structs (see `payloads.rs`); the serde wire is IDENTICAL to
//! the former inline struct-variant form (proven by `wire_tests.rs`,
//! whose literals were captured against the pre-conversion enum).

use rekindle_types::video::{Codec, ScalabilityMode};
use serde::{Deserialize, Serialize};

use super::payloads::{
    MekTransferAckPayload, MekTransferPayload, VideoFragmentPayload, VideoParityFragmentPayload,
};
use super::{OnboardingAnswer, VoiceRosterEntry};

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
    /// Peer-to-peer MEK delivery (see [`MekTransferPayload`]).
    MekTransfer(MekTransferPayload),
    /// P1.3 — requester → responder ack confirming successful
    /// `MekTransfer` ingestion (see [`MekTransferAckPayload`]).
    MekTransferAck(MekTransferAckPayload),
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
    EventCreated {
        event: rekindle_types::event::EventInfo,
    },
    /// Broadcast: event updated.
    EventUpdated {
        event: rekindle_types::event::EventInfo,
    },
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
    /// Broadcast: member joined a voice channel (handshake leg 1 —
    /// "reach out").
    VoiceJoin {
        channel_id: String,
        /// Private route blob for receiving voice packets.
        route_blob: Vec<u8>,
        /// Joiner's self-sovereign display name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
    },
    /// Handshake leg 2 — "seen": a present member acks the joiner's
    /// VoiceJoin, carrying its own identity + route so the ack alone
    /// lets the joiner add the acker to its media roster. Directed to
    /// the channel roster (ttl = 0), never relayed.
    VoiceJoinAck {
        channel_id: String,
        /// Pseudonym of the joiner being acked.
        joiner_pseudonym: String,
        /// Acker's display name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        /// Acker's private route blob.
        route_blob: Vec<u8>,
    },
    /// Handshake leg 3 — "confirmed": the joiner is transport-ready
    /// (routes ingested) and asks members to start media. Receivers
    /// force a video keyframe (RFC 5104 FIR semantics — a new member
    /// needs a full intra to start decoding). Directed, ttl = 0.
    VoiceJoinConfirmed { channel_id: String },
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

    /// Architecture §10.6 — receiver acknowledges fragments roughly
    /// every 500 ms and reports their measured downstream bandwidth.
    /// Senders adjust VP9 bitrate to match the slowest receiver.
    /// One transport-sized piece of an encoded video frame (see
    /// [`VideoFragmentPayload`]).
    VideoFragment(VideoFragmentPayload),
    /// Reed-Solomon parity for the matching `VideoFragment` stream
    /// (see [`VideoParityFragmentPayload`]).
    VideoParityFragment(VideoParityFragmentPayload),
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

    /// Capability negotiation broadcast on join. Senders intersect
    /// every receiver's decode set against their own encode set and
    /// pick their per-node encoder codec (architecture §10.6 line 4084).
    ///
    /// Pre-release schema break (memory rule: `feedback_no_legacy_compat`):
    /// the single symmetric `codecs` list split into `encode_codecs` +
    /// `decode_codecs` — WebView engines are direction-asymmetric
    /// (Apple WebKit: H.264 hw encode, broader decode). No version
    /// compat shim — the wire layer drops the old shape entirely.
    MediaCapabilities {
        channel_id: String,
        max_pixel_count: u32,
        max_fps: u8,
        encode_codecs: Vec<Codec>,
        decode_codecs: Vec<Codec>,
        supports_optimize_for_latency: bool,
        supported_scalability_modes: Vec<ScalabilityMode>,
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
