//! Phase 14 — voice session dependency port.
//!
//! The `VoiceSessionDeps` trait is what the rekindle-voice session
//! orchestration (session lifecycle, shutdown, device hot-swap) calls
//! into for every outside-world operation: voice engine handle access,
//! voice-packet channel staging (W14.1), community state lookups (MEK,
//! member names, stage gate), identity, active call_key lookup
//! (1:1 calls), Tauri emit, background task registration.
//!
//! Unlike `rekindle-calls::CallSignalingDeps`, this crate already has
//! veilid-core direct access (pre-existing VEILID_ALLOWED exception),
//! so DHT operations don't need abstraction. The trait surface focuses
//! on `AppState` + Tauri integration points.
//!
//! Implemented by `src-tauri/services/voice_adapter.rs` (lands in 14.h).

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::VoiceError;

/// One peer in a community voice channel. Materialized by the
/// adapter from `AppState.communities` for handlers / future
/// crate-side flows that need a roster snapshot (e.g. voice_peers
/// trait method below).
#[derive(Debug, Clone)]
pub struct VoicePeer {
    /// Pseudonym hex (32-byte Ed25519 pubkey hex-encoded).
    pub pseudonym: String,
    /// Display name (member display string for UI surfaces).
    pub display_name: String,
    /// Veilid route blob. `None` if we don't yet have a route.
    pub route_blob: Option<Vec<u8>>,
}

/// Snapshot of the call_key + kind for a 1:1 call (used by the receive
/// loop to decrypt audio AEAD frames W13.14).
#[derive(Debug, Clone)]
pub struct CallKeyInfo {
    pub call_key: [u8; 32],
    pub peer_pubkey: String,
}

/// Audio preferences pulled from the Tauri store. The adapter
/// (`voice_adapter.rs`) populates this from `commands::settings::Preferences`
/// at session start. The crate side stays free of Tauri's store types.
///
/// Mirrors the subset of `Preferences` fields that the existing
/// `init_engine` consumes; jitter_buffer_ms etc. come from
/// `VoiceConfig::default()` not from user prefs.
#[derive(Debug, Clone)]
pub struct AudioPrefs {
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub input_volume: f32,
    pub output_volume: f32,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

/// Identity snapshot returned by the adapter for `start_session`
/// (public key + display name in one struct, so the orchestrator
/// doesn't make two trait calls).
#[derive(Debug, Clone)]
pub struct VoiceIdentity {
    pub public_key: String,
    pub display_name: String,
}

/// Return type of `init_voice_session` — the engine handle + transport
/// + shared mute/deafen flags all set up and ready for loop spawn.
pub struct VoiceSessionStartup {
    pub muted_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub deafened_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub transport: std::sync::Arc<tokio::sync::Mutex<crate::transport::VoiceTransport>>,
}

/// Configurable scope for `shutdown_voice`. Three named variants
/// cover the call sites:
///   - `FULL`: stop loops + monitor + devices + clear engine.
///   - `LOOPS_ONLY`: stop send/recv/MCU loops only; keep monitor
///     + engine alive (used by device hot-swap which is *itself*
///     running on the monitor loop).
///   - `KEEP_ENGINE`: stop loops + monitor but keep engine alive
///     (used by device restart paths that respawn loops without
///     re-initialising cpal).
#[derive(Debug, Clone, Copy)]
pub struct VoiceShutdownOpts {
    pub stop_loops: bool,
    pub stop_monitor: bool,
    pub stop_devices: bool,
}

impl VoiceShutdownOpts {
    pub const FULL: Self = Self {
        stop_loops: true,
        stop_monitor: true,
        stop_devices: true,
    };
    pub const LOOPS_ONLY: Self = Self {
        stop_loops: true,
        stop_monitor: false,
        stop_devices: false,
    };
    pub const KEEP_ENGINE: Self = Self {
        stop_loops: true,
        stop_monitor: true,
        stop_devices: false,
    };
}

/// Loop shutdown handles taken from the engine in one batch. The
/// shutdown orchestrator signals each `Sender<()>` and then awaits
/// each `JoinHandle`. Whichever set was opted out of via
/// `VoiceShutdownOpts` arrives as `None`.
pub struct VoiceShutdownHandles {
    pub send_loop_shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    pub send_loop_handle: Option<tokio::task::JoinHandle<()>>,
    pub recv_loop_shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    pub recv_loop_handle: Option<tokio::task::JoinHandle<()>>,
    pub monitor_shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    pub monitor_handle: Option<tokio::task::JoinHandle<()>>,
    pub mcu_shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    pub mcu_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Orchestration port for voice session work.
#[async_trait]
pub trait VoiceSessionDeps: Send + Sync + 'static {
    // --- Identity ---

    /// Current owner key (Ed25519 public key hex). Errors if no
    /// identity is loaded.
    fn owner_key(&self) -> Result<String, VoiceError>;

    /// Identity Ed25519 secret bytes (32 B). Errors if no identity.
    fn identity_secret(&self) -> Result<[u8; 32], VoiceError>;

    // --- Voice engine handle (held on AppState) ---

    /// Returns `true` if a voice engine is currently up (`Some` in the
    /// AppState mutex). Used by call signaling to detect mid-call
    /// teardowns vs ringing-only teardowns.
    fn voice_engine_present(&self) -> bool;

    /// Flip the voice engine's muted state. Sets BOTH the engine's
    /// internal `set_muted()` AND the shared `muted_flag` atomic
    /// that the send loop reads. Used by the local-mute command and
    /// by stage-channel audience auto-mute on join.
    fn set_voice_engine_muted(&self, muted: bool);

    /// Symmetric flip for the deafen state. Sets engine + the
    /// `deafened_flag` atomic the receive loop reads.
    fn set_voice_engine_deafened(&self, deafened: bool);

    // --- Voice packet channels (W14.1 pre-stage pattern) ---

    /// Pre-stage the voice receive channel: create a new mpsc, store
    /// the sender on AppState.voice_packet_tx (so the dispatch path can
    /// route inbound packets), and stash the receiver on
    /// voice_packet_rx_staged for the receive loop to pick up. Called
    /// BEFORE any await points in CallAccept handling so packets
    /// arriving during session setup buffer rather than drop.
    fn pre_stage_voice_channel(&self);

    /// Clear the voice packet channels (W15.5 — voice shutdown).
    fn clear_voice_channels(&self);

    // --- Community state lookups (for community voice channels) ---

    /// Look up the current MEK for a community's voice (for audio AEAD
    /// in community voice). The MEK is keyed by community_id only —
    /// one MEK per community covers all voice channels in it (the
    /// existing `mek_cache` map shape). `None` if no MEK is cached.
    fn community_voice_mek(&self, community_id: &str) -> Option<[u8; 32]>;

    /// Snapshot of peers in a community voice channel (with their
    /// pseudonym, display name, route blob). Used at session start
    /// + on roster updates.
    fn voice_peers(&self, community_id: &str, channel_id: &str) -> Vec<VoicePeer>;

    /// Returns `true` if the given channel is a stage channel (only
    /// designated speakers may transmit). Used by send_loop's stage
    /// gate (§10.7).
    fn channel_is_stage(&self, community_id: &str, channel_id: &str) -> bool;

    /// Returns `true` if `our_pseudonym` is currently a designated
    /// speaker in the stage channel. Used by send_loop's stage gate.
    fn we_are_stage_speaker(
        &self,
        community_id: &str,
        channel_id: &str,
        our_pseudonym: &str,
    ) -> bool;

    /// Returns `true` if the given sender is a designated stage speaker
    /// (used by receive_loop to drop packets from non-speakers).
    fn sender_is_stage_speaker(
        &self,
        community_id: &str,
        channel_id: &str,
        sender_pseudonym: &str,
    ) -> bool;

    // --- 1:1 call_key lookup (W13.14 audio AEAD) ---

    /// Look up the call_key + peer pubkey for an active 1:1 call from a
    /// specific peer. Returns `None` if no matching call. Used by
    /// receive_loop to decrypt audio AEAD frames in 1:1 calls.
    fn call_key_for_peer(&self, peer_pubkey: &str) -> Option<CallKeyInfo>;

    // --- Telemetry ---

    /// Increment the packet-drop counter (W14.4 — exposed via
    /// VoiceEvent::PacketsDropped).
    fn record_packet_drop(&self);

    /// Read the current packet-drop counter (for telemetry emit).
    fn packet_drops(&self) -> u64;

    // --- Frontend emit ---

    /// Push a voice event to the frontend. The adapter maps each
    /// variant to its concrete Tauri `VoiceEvent` payload and emits.
    fn emit_voice_event(&self, event: VoiceSessionEvent);

    // --- Background tasks ---

    /// Register a spawned background task so it can be aborted on app
    /// shutdown.
    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>);

    // --- Session orchestration (Phase 14.l) ---
    //
    // Wired by the upcoming `rekindle_voice::session::start_session`
    // port. Adapter implementations delegate to the existing
    // `services::voice::session::*` helpers; the crate side will
    // own the orchestration sequencing.

    /// Snapshot of (public_key, display_name) for the current
    /// identity. Returns `Err(VoiceError::IdentityNotLoaded)` if no
    /// identity is loaded.
    fn current_identity(&self) -> Result<VoiceIdentity, VoiceError>;

    /// Reject the session start if we're already in a voice call in
    /// a different channel. Idempotent: returns `Ok` if we're already
    /// in `channel_id` (no double-join), Err otherwise.
    fn check_not_in_call(&self, channel_id: &str) -> Result<(), VoiceError>;

    /// Read audio preferences from the Tauri store.
    fn audio_prefs(&self) -> AudioPrefs;

    /// Coarse step 1: build the `VoiceEngine` with `prefs`, install
    /// it on `VoiceEngineHandle` with `channel_id` / `community_id`,
    /// start the cpal capture + playback devices, build the shared
    /// `VoiceTransport` (1:1 calls pass `peer_route_blob`), install
    /// the call_key on the transport if it's a 1:1 call, install the
    /// transport on the engine handle. Returns the (muted_flag,
    /// deafened_flag, transport) tuple the orchestrator threads
    /// through `spawn_voice_loops`.
    ///
    /// One coarse method (rather than the 4 fine-grained operations
    /// it replaces) because the AppState mutation pattern between
    /// them is invariant — the adapter is the right home for that
    /// invariance.
    fn init_voice_session(
        &self,
        prefs: &AudioPrefs,
        channel_id: &str,
        community_id: Option<&str>,
        peer_route_blob: Option<&[u8]>,
    ) -> Result<VoiceSessionStartup, VoiceError>;

    /// W13.12 — resolve a 1:1 peer's route via the fallback chain
    /// (cache → DHT subkey 6 → mailbox). `None` if all three empty
    /// (caller must convert to CallDecline / CallEnd).
    async fn resolve_peer_route(&self, peer_pubkey_hex: &str) -> Option<Vec<u8>>;

    /// Load member display names for a community into a
    /// `pseudonym_hex → display_name` map. Used by the receive loop
    /// to surface friendly names on UserJoined events. Empty map for
    /// 1:1 calls.
    async fn load_member_names(
        &self,
        community_id: Option<&str>,
    ) -> std::collections::HashMap<String, String>;

    /// §10.6 — broadcast our media decode capabilities to a community
    /// voice channel so other senders cap their VP9 bitrate at the
    /// lowest common denominator. Best-effort: errors log but don't
    /// fail the session start.
    fn broadcast_media_capabilities(&self, community_id: &str, channel_id: &str);

    /// Emit the local-join voice event (`VoiceEvent::LocalJoined`)
    /// and any related join-side events. Called at the end of
    /// `start_session` after loops are spawned.
    fn emit_local_joined(
        &self,
        channel_id: &str,
        community_id: Option<&str>,
        public_key: &str,
        display_name: &str,
    );

    /// Coarse step 2: take the staged voice packet receiver (or
    /// create one), pull capture_rx + playback_tx + (noise_supp,
    /// echo_cancel) off the engine handle, spawn the three loops
    /// (send / receive / device_monitor) using the deps + transport
    /// + member_names, store all shutdown senders + JoinHandles
    /// back on the engine handle.
    ///
    /// All AppState mutation lives here. The crate's `start_session`
    /// just calls this after `init_voice_session` returned the
    /// transport.
    fn spawn_voice_loops(
        &self,
        public_key: &str,
        transport: std::sync::Arc<tokio::sync::Mutex<crate::transport::VoiceTransport>>,
        muted_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        deafened_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        member_names: std::collections::HashMap<String, String>,
    ) -> Result<(), VoiceError>;

    // ── Restart-loops (hot-swap path) deps (Phase 14.l-restart) ─────

    /// Re-open cpal capture + playback on the existing engine handle.
    /// Used by device hot-swap after `shutdown_voice(KEEP_ENGINE)`.
    fn restart_audio_devices(&self) -> Result<(), VoiceError>;

    /// Shared transport handle on the active voice engine, or `None`
    /// if no engine is running. Used by `restart_loops` to reuse
    /// peers already added via VoiceJoin gossip.
    fn current_shared_transport(
        &self,
    ) -> Option<std::sync::Arc<tokio::sync::Mutex<crate::transport::VoiceTransport>>>;

    /// Cloned (muted_flag, deafened_flag) atomics from the active
    /// engine handle, or `Err` if no engine is running.
    fn current_voice_flags(
        &self,
    ) -> Result<
        (
            std::sync::Arc<std::sync::atomic::AtomicBool>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
        VoiceError,
    >;

    /// Currently bound community_id (or `None` for 1:1 calls / no
    /// engine). Used by restart_loops to re-fetch member_names.
    fn active_community_id(&self) -> Option<String>;

    /// Snapshot of the active engine's (channel_id, community_id).
    /// Returns `(String::new(), None)` if no engine. Used by the
    /// leave_voice flow to broadcast the right scope.
    fn active_channel_info(&self) -> (String, Option<String>);

    /// Send a community gossip envelope (fire-and-forget). Used by
    /// the leave_voice / join_community_voice flows to fan out
    /// VoiceLeave / VoiceJoin. Adapter delegates to
    /// `services::community::send_to_mesh` (Phase 20 will own).
    fn send_community_envelope(
        &self,
        community_id: &str,
        envelope: &rekindle_protocol::dht::community::envelope::CommunityEnvelope,
    );

    /// DB analytics: log a voice join / leave event into the
    /// per-owner SQLite analytics table. Best-effort.
    /// (Phase 22 sync may consolidate this; for now it stays as a
    /// dedicated trait method.)
    fn log_voice_membership(&self, community_id: &str, channel_id: &str, joined: bool);

    /// Snapshot our Veilid route blob for inclusion in a VoiceJoin
    /// envelope. Returns empty Vec if we have no advertised route.
    fn our_route_blob(&self) -> Vec<u8>;

    // ── MCU lifecycle deps (Phase 14.l-mcu) ─────────────────────────

    /// Create a fresh MCU packet mpsc + install the sender on
    /// `AppState.voice_packet_tx`. Returns the receiver for the MCU
    /// loop to drain.
    fn pre_stage_mcu_channel(&self) -> tokio::sync::mpsc::Receiver<crate::transport::VoicePacket>;

    /// Store the MCU loop's shutdown tx + JoinHandle on the engine
    /// handle so shutdown can abort cleanly.
    fn register_mcu_task(
        &self,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    );

    /// Take + drop the MCU loop's shutdown tx + JoinHandle. Signals
    /// shutdown and awaits the loop's exit. No-op if MCU isn't running.
    async fn stop_active_mcu(&self);

    // ── Shutdown + device-monitor deps (Phase 14.l-shutdown) ────────

    /// Take the requested subset of loop shutdown handles from the
    /// engine in one atomic operation (under the engine lock).
    /// Subsets controlled by `opts` — handles for opted-out loops
    /// stay on the engine, returned as `None` in the bundle.
    fn take_shutdown_handles(&self, opts: VoiceShutdownOpts) -> VoiceShutdownHandles;

    /// Stop cpal capture + playback AND clear the voice engine
    /// handle (set to `None`). Used by `shutdown_voice(FULL)`.
    fn stop_devices_and_clear_engine(&self);

    /// Stop cpal capture + playback WITHOUT clearing the engine
    /// handle. Used by device hot-swap before
    /// `set_voice_engine_devices(None, None)` + restart.
    fn stop_audio_devices(&self);

    /// Update the engine's (input_device, output_device) config.
    /// `None` = system default. Takes effect on the next
    /// `restart_audio_devices()` call.
    fn set_voice_engine_devices(&self, input: Option<String>, output: Option<String>);

    /// Read the currently selected device names from the engine's
    /// config. Returns `(None, None)` if no engine or both defaults.
    fn voice_engine_device_config(&self) -> (Option<String>, Option<String>);

    /// Emit a `VoiceEvent::DeviceChanged` with the given fields.
    /// Distinct from `emit_voice_event(VoiceSessionEvent::DeviceChanged)`
    /// because the existing wire variant carries `device_name` which
    /// the crate event doesn't surface.
    fn emit_device_changed(&self, device_type: String, device_name: String, reason: String);

    /// Emit a `NotificationEvent::SystemAlert` (title + body). Used
    /// by device hot-swap to surface "Audio Device Disconnected".
    fn emit_system_alert(&self, title: String, body: String);

    // --- DB lookups (member-name resolution) ---

    /// Resolve a member's display name from the local DB (best-effort,
    /// returns `None` on missing/error). The adapter wraps the SQLite
    /// query.
    async fn resolve_member_display_name(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Option<String>;
}

/// Voice-side events the session/loop modules emit. Mirrors the
/// existing `VoiceEvent` shape but lives in the crate so the trait
/// surface is self-contained.
#[derive(Debug, Clone)]
pub enum VoiceSessionEvent {
    /// A new participant joined the voice channel.
    UserJoined {
        peer_pubkey: String,
        display_name: String,
    },
    /// A participant left.
    UserLeft { peer_pubkey: String },
    /// A participant started/stopped speaking (VAD edge).
    UserSpeaking {
        peer_pubkey: String,
        speaking: bool,
    },
    /// A participant muted/unmuted themselves.
    UserMuted { peer_pubkey: String, muted: bool },
    /// Audio device changed (hot-swap).
    DeviceChanged { device_type: String, reason: String },
    /// Packet drops counter (telemetry — W14.4).
    PacketsDropped { count: u64 },
    /// Connection quality summary (every 5 s from send_loop). Quality
    /// is `"good"` / `"fair"` / `"poor"` based on packet loss %.
    ConnectionQuality { quality: String },
}

/// Used by the deps trait helper to return owned data; placeholder for
/// any future trait-shared types.
pub type DepsHandle = Arc<dyn VoiceSessionDeps>;
