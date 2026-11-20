//! The community control-payload enum — every non-chat operation on
//! the mesh. The four largest variants carry their fields as named
//! payload structs (see `payloads.rs`); the serde wire is IDENTICAL to
//! the former inline struct-variant form (proven by `wire_tests.rs`,
//! whose literals were captured against the pre-conversion enum).

use rekindle_types::id::RoleId;
use rekindle_types::video::{Codec, ScalabilityMode};
use serde::{Deserialize, Serialize};

use super::payloads::{
    MekTransferAckPayload, MekTransferPayload, VideoFragmentPayload, VideoParityFragmentPayload,
};
use super::{MediaSessionKey, OnboardingAnswer, VoiceRosterEntry};

/// Control payload covering all non-chat operations actually used by the
/// forward communities mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ControlPayload {
    // ── Member lifecycle ──
    /// Member voluntarily leaving.
    MemberLeave { pseudonym_key: String },
    /// Broadcast: a member joined.
    MemberJoined {
        pseudonym_key: String,
        display_name: String,
        role_ids: Vec<RoleId>,
        status: String,
        /// Route blob so receivers can immediately add the joiner to their gossip overlay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route_blob: Option<Vec<u8>>,
    },
    // ── Moderation ──
    /// Kick a member.
    Kick { target_pseudonym: String },
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
        role_ids: Vec<RoleId>,
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
    /// Broadcast: a thread reply exists (plan E3.3): its body rides the
    /// author's channel outbox, so the notice carries none.
    ThreadMessageReceived {
        thread_id: String,
        message_id: String,
        sender_pseudonym: String,
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

    // ── AutoMod / Raid notifications ──
    /// Raid alert broadcast to all members (owners/admins should act).
    RaidAlert { active: bool },
    /// Channel lockdown broadcast: non-admins should restrict sending.
    ChannelLockdown { locked: bool },
    /// System message broadcast (join/leave/kick/ban events posted to chat feed).
    SystemMessage { body: String, timestamp: u64 },

    // ── Bootstrap protocol ──
    /// The joiner's bootstrap request (plan E3.2): a bare app_call to the
    /// inviter's route. It carries no authority of its own: the inviter
    /// verifies the joiner's signed `MemberJoined` (`member_joined_op`,
    /// whose hash `member_joined` names) before answering.
    BootstrapRequest {
        /// Joiner's community pseudonym (hex-encoded Ed25519 public key).
        joiner_pseudonym: String,
        /// The community anchor `A`'s record key.
        anchor: String,
        /// The joiner's op #0, `MemberJoined`.
        member_joined: rekindle_types::governance::OpRef,
        /// The encoded signed op `member_joined` names.
        member_joined_op: Vec<u8>,
        /// The joiner's route, where the inviter's `MekTransfer` of the
        /// community key goes once the op merges (D16).
        joiner_route: Vec<u8>,
    },
    /// The inviter's answer to a `BootstrapRequest` (plan E3.2): where the
    /// joiner's walk of the op DAG continues, constant in N. No keys (D11)
    /// and no messages.
    BootstrapResponseV2 {
        anchor: String,
        /// The inviter's frontier, at most `MAX_FRONTIER_REFS` heads.
        frontier: Vec<rekindle_types::governance::OpRef>,
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
        /// Joiner's session key, which media keys are sealed to.
        #[serde(default)]
        session_key: MediaSessionKey,
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
        /// Acker's session key, which media keys are sealed to.
        #[serde(default)]
        session_key: MediaSessionKey,
    },
    /// Handshake leg 3 — "confirmed": the joiner is transport-ready
    /// (routes ingested) and asks members to start media. Receivers
    /// force a video keyframe (RFC 5104 FIR semantics — a new member
    /// needs a full intra to start decoding). Directed, ttl = 0.
    VoiceJoinConfirmed { channel_id: String },
    /// A sender's call-media key, sealed to one participant (plans C7.20,
    /// C7.29; RFC 9605 §5.1 sender keys). HPKE Base to `recipient`'s
    /// session key, bound to the channel, recipient and index, and signed
    /// by `sender`. Directed to
    /// the channel roster (ttl = 0); others ignore it.
    VoiceMediaKey {
        community_id: String,
        channel_id: String,
        /// Pseudonym (hex) whose key this is; it sealed it.
        sender: String,
        /// Pseudonym (hex) the key is sealed to.
        recipient: String,
        key_index: u64,
        sealed: Vec<u8>,
        /// `sender`'s pseudonym signature over channel, recipient, index
        /// and `sealed` (plan C7.29).
        #[serde(default)]
        signature: Vec<u8>,
    },
    /// The gossip fast path for a signed governance op (plan E3.2,
    /// `governanceOp @74`): the encoded op (`governance_op::encode_op`) a
    /// member published to its op log, verified as a fetched op is.
    GovernanceOp { op: Vec<u8> },
    /// A ring watcher's `HeadAdvanced` hint, encoded (plan E3.2 S7).
    HeadAdvanced { hint: Vec<u8> },
    /// An encoded, signed `CheckpointStateRequest` (plan E3.2 S7c.3).
    CheckpointStateRequest { request: Vec<u8> },
    /// An encoded `CheckpointStateResponse` (plan E3.2 S7c.3).
    CheckpointStateResponse { response: Vec<u8> },
    /// An encoded, signed `CheckpointHintRequest` (plan E3.2 S7c.5).
    CheckpointHintRequest { request: Vec<u8> },
    /// An encoded `CheckpointHintResponse` (plan E3.2 S7c.5).
    CheckpointHintResponse { response: Vec<u8> },
    /// A partial responder's answer to a `BootstrapRequest` for `anchor`
    /// (plan E3.2 S7c.8): ask another member, not a refusal.
    BootstrapPartial { anchor: String },
    /// The inviter's refusal of a `BootstrapRequest` for `anchor`, and why
    /// (plan E3.2 join step 5): the joiner withdraws its join.
    BootstrapRefused {
        anchor: String,
        reason: rekindle_types::join_refusal::JoinRefusal,
    },
    /// Reply to a [`Self::VoiceMediaKey`] call: `recipient` opened and
    /// installed `sender`'s key at `key_index` (plan C7.22).
    VoiceMediaKeyAck {
        community_id: String,
        channel_id: String,
        sender: String,
        recipient: String,
        key_index: u64,
    },
    /// Ask `sender` for its call-media key at `key_index`: a frame arrived
    /// under it before (or without) its push. Directed, ttl = 0.
    VoiceMediaKeyRequest {
        community_id: String,
        channel_id: String,
        /// Pseudonym (hex) asking; the key is sealed to it.
        requester: String,
        /// Pseudonym (hex) whose key is asked for.
        sender: String,
        key_index: u64,
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

    /// One transport-sized piece of an encoded video frame (see
    /// [`VideoFragmentPayload`]).
    VideoFragment(VideoFragmentPayload),
    /// Reed-Solomon parity for the matching `VideoFragment` stream
    /// (see [`VideoParityFragmentPayload`]).
    VideoParityFragment(VideoParityFragmentPayload),
    /// Sent by a receiver who has lost too many inter-frames to keep
    /// rendering. Senders treat this as a request to encode the next
    /// frame as a keyframe (architecture §10.6).
    KeyframeRequest {
        channel_id: String,
        stream_id: [u8; 16],
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
        site_name: Option<String>,
        fetched_at: u64,
    },
    // ── Generic responses ──
}
