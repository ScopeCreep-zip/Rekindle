//! Phase 14 — voice signaling dependency port.
//!
//! `VoiceSignalingDeps` is the abstraction the rekindle-voice signaling
//! handlers (voice_join / voice_leave / stage_update / speak_request /
//! speak_response / voice_mute / voice_deafen / voice_roster /
//! soundboard_play) talk to for every outside-world operation:
//! identity, community state lookups, voice engine control, mesh
//! broadcast, MEK rotation, MCU loop lifecycle, persistence, and
//! frontend emit.
//!
//! The src-tauri adapter implements this trait against `AppState` +
//! `tauri::AppHandle` + `DbPool` + `services::community::*` (where
//! `rotate_voice_mek_for_membership`, `send_to_mesh`, and
//! `persist_hand_raise` still live until Phases 17/19/20 take
//! ownership of MEK rotation, gossip mesh, and channel persistence
//! respectively). The trait lets the crate be free of `AppState`,
//! `tauri::AppHandle`, and `services::community::*` references — exactly
//! the same shape used for `DmDeps`, `CallSignalingDeps`, and
//! `VoiceSessionDeps`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use crate::transport::VoiceTransport;

/// Stage channel state used by the join/leave/stage_update handlers
/// to decide stage-host election + whether to apply the mode-switch
/// auto-elect at the 5+ member threshold.
#[derive(Debug, Clone)]
pub struct StageChannelInfo {
    pub is_stage: bool,
    pub speakers: Vec<String>,
    pub moderator: Option<String>,
}

/// Frontend events the signaling handlers emit. The adapter maps each
/// variant to its concrete src-tauri `CommunityEvent` payload and
/// calls `app.emit("community-event", _)`.
#[derive(Debug, Clone)]
pub enum CommunityVoiceEvent {
    VoiceJoin {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
        route_blob: Vec<u8>,
    },
    VoiceLeave {
        community_id: String,
        channel_id: String,
        pseudonym_key: String,
    },
    VoiceModeSwitch {
        community_id: String,
        channel_id: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    StageUpdate {
        community_id: String,
        channel_id: String,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator_pseudonym: String,
    },
    SpeakRequest {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
    },
    SpeakResponse {
        community_id: String,
        channel_id: String,
        requester_pseudonym: String,
        granted: bool,
        moderator_pseudonym: String,
    },
    SoundboardPlay {
        community_id: String,
        channel_id: String,
        expression_id: String,
        actor_pseudonym: String,
    },
    /// Local voice engine state mute event. Distinct from
    /// `CommunityEvent` — emits to "voice-event" channel.
    UserMuted {
        target_pseudonym: String,
        muted: bool,
    },
}

/// Permission bit mask values needed by signaling. Mirrors
/// `rekindle_types::permissions::*` constants so the trait method
/// callers don't need to depend on `rekindle-types` directly.
pub mod perms {
    pub const MANAGE_MESSAGES: u64 = rekindle_types::permissions::MANAGE_MESSAGES;
    pub const ADMINISTRATOR: u64 = rekindle_types::permissions::ADMINISTRATOR;
    pub const USE_SOUNDBOARD: u64 = rekindle_types::permissions::USE_SOUNDBOARD;
}

#[async_trait]
pub trait VoiceSignalingDeps: Send + Sync + 'static {
    // ── Identity / community state lookups ─────────────────────

    /// Our pseudonym in the given community, or `None` if we're not
    /// a member.
    fn my_pseudonym(&self, community_id: &str) -> Option<String>;

    /// Snapshot of stage-channel state. `None` if community/channel
    /// not found. `is_stage = false` for non-stage channels.
    fn stage_channel_info(&self, community_id: &str, channel_id: &str) -> Option<StageChannelInfo>;

    /// Write back stage-channel state (topic / speakers / moderator).
    /// Best-effort; no-ops on missing community/channel.
    fn update_stage_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator: String,
    );

    /// Snapshot of online-member route blobs for a community (for
    /// voice roster broadcast). Returns `(pseudonym, route_blob)`.
    fn online_voice_members(&self, community_id: &str) -> Vec<(String, Vec<u8>)>;

    // ── Permission checks (Phase 18 governance) ────────────────

    /// Decode a 16-byte channel ID from its hex form. Returns `None`
    /// on malformed input. Used by speak_request permission lookup.
    fn decode_channel_id(&self, channel_id: &str) -> Option<[u8; 16]>;

    /// Compute our own permission bitmask in the given community,
    /// optionally scoped to a channel. Returns 0 if not a member.
    fn my_permissions(&self, community_id: &str, channel_id: Option<[u8; 16]>) -> u64;

    /// Returns `true` if the sender holds ALL bits in `perm_mask`
    /// at the community level (used by SoundboardPlay's
    /// USE_SOUNDBOARD gate). Implementation reads
    /// `community.governance_state` and computes permissions for
    /// `sender_pseudonym_hex`.
    fn sender_has_perm(
        &self,
        community_id: &str,
        sender_pseudonym_hex: &str,
        perm_mask: u64,
    ) -> bool;

    // ── Voice engine handle (cpal-bound, src-tauri-side) ───────

    /// Get the shared voice transport handle. `None` if no voice
    /// engine is active. Returned as `Arc<tokio::sync::Mutex<...>>`
    /// so handlers can `.lock().await` to mutate (add_peer / remove_peer
    /// / set_mode / peer_keys).
    fn transport_handle(&self) -> Option<Arc<tokio::sync::Mutex<VoiceTransport>>>;

    /// Currently-active voice channel ID on the voice engine handle.
    /// `None` if no engine active. Used by the receive-side stage gate
    /// to validate which channel the active speakers list applies to.
    fn voice_engine_channel_id(&self) -> Option<String>;

    /// Whether the voice engine is currently bound to `community_id` /
    /// `channel_id`. Used to gate self-mute updates on stage-speaker
    /// changes — only mute ourselves if we're actually IN that channel.
    fn voice_engine_bound_to(&self, community_id: &str, channel_id: &str) -> bool;

    /// Flip the engine's muted state. Sets BOTH the engine's internal
    /// `set_muted()` AND the shared `muted_flag` atomic that the send
    /// loop reads.
    fn set_voice_engine_muted(&self, muted: bool);

    /// Flip the engine's deafened state. Sets engine + `deafened_flag`.
    fn set_voice_engine_deafened(&self, deafened: bool);

    // ── Cross-subsystem ops (deferred to Phase 17 / 19 / 20) ────

    /// W11.2 — rotate the channel MEK on membership change. Phase 17
    /// (rekindle-mek-rotation) eventually owns this; today the
    /// adapter delegates to `services::community::rotate_voice_mek_for_membership`.
    /// Fire-and-forget: failures log but don't propagate.
    async fn rotate_voice_mek_for_membership(
        &self,
        community_id: String,
        channel_id: String,
        member_pseudonym: String,
        joined: bool,
    );

    /// Send a gossip envelope to the community mesh. Phase 20
    /// (rekindle-gossip) eventually owns this; today the adapter
    /// delegates to `services::community::send_to_mesh`. Sync because
    /// the existing src-tauri function is sync (fire-and-forget).
    fn send_to_mesh(&self, community_id: &str, envelope: &CommunityEnvelope);

    /// Persist our own hand-raise state on a SpeakResponse. Phase 19
    /// (rekindle-channel) eventually owns this; today the adapter
    /// delegates to `services::community::persist_hand_raise`.
    /// Fire-and-forget: failures log.
    async fn persist_hand_raise(&self, community_id: String, channel_id: String, raised: bool);

    /// Increment + return the per-community Lamport counter. Phase
    /// 20 (rekindle-gossip) eventually owns this; today the adapter
    /// delegates to `state_helpers::increment_lamport`. Used to tag
    /// gossip envelopes (SpeakRequest, SpeakResponse, StageUpdate).
    fn next_lamport(&self, community_id: &str) -> u64;

    /// Look up a channel's current stage_speakers list. Used by
    /// `respond_to_speak_request` to compose the StageUpdate that
    /// adds the newly-granted speaker. Returns empty Vec if the
    /// channel doesn't exist.
    fn stage_speakers(&self, community_id: &str, channel_id: &str) -> Vec<String>;

    // ── MCU loop lifecycle ─────────────────────────────────────

    /// Start the MCU mixing loop (we became the elected voice host).
    /// Currently delegates to `services::voice::session::start_mcu_loop`;
    /// when 14.g-session lands the orchestration moves into the crate.
    fn start_mcu_loop(&self);

    /// Stop the MCU mixing loop (we lost host election or left the
    /// channel). Delegates to `services::voice::session::stop_mcu_loop`.
    async fn stop_mcu_loop(&self);

    // ── Frontend emit ──────────────────────────────────────────

    /// Push a community / voice signaling event to the frontend. The
    /// adapter maps to the concrete src-tauri `CommunityEvent` (or
    /// `VoiceEvent::UserMuted` for the `UserMuted` variant) and emits.
    fn emit_event(&self, event: CommunityVoiceEvent);

    // ── Background tasks ───────────────────────────────────────

    /// Register a spawned background task so it can be aborted on
    /// app shutdown.
    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>);
}
