//! Phase 14 — `CallSignalingDeps` trait.
//!
//! The orchestration port the rekindle-calls signaling logic talks to.
//! Identity, transport (Signal-encrypted `app_message` sends), voice
//! session start/stop, persistence (missed-call log), and frontend
//! emit are all behind this trait — the adapter in `src-tauri/services/
//! calls_adapter.rs` (lands in 14.h) implements it against
//! `AppState` + `tauri::AppHandle` + `message_service::send_to_peer_raw`
//! + `services::voice::*`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::error::CallError;
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::{CallRegistry, GroupCallRegistry};
use crate::state::CallKind;

#[async_trait]
pub trait CallSignalingDeps: Send + Sync + 'static {
    // --- Identity ---

    /// Owner key (current identity's Ed25519 public key, hex-encoded).
    /// Errors if no identity is loaded.
    fn owner_key(&self) -> Result<String, CallError>;

    /// Identity Ed25519 secret bytes (32 B). Errors if no identity is
    /// loaded. Caller is responsible for zeroizing after use (most
    /// callers convert to X25519 then drop).
    fn identity_secret(&self) -> Result<[u8; 32], CallError>;

    // --- Subsystems ---

    /// 1:1 call registry handle.
    fn registry(&self) -> Arc<dyn CallRegistry>;

    /// Group call registry handle.
    fn group_registry(&self) -> Arc<dyn GroupCallRegistry>;

    // --- Mute policy ---

    /// Returns `true` if the peer is temp-muted (W12.12 — `temp_call_muted`
    /// map). Signaling drops their CallInvite payloads silently in that
    /// case (the adapter looks up the map; this trait method abstracts).
    fn is_peer_temp_muted(&self, peer_pubkey_hex: &str) -> bool;

    /// Resolve a peer's display name for UI surfaces (friend list
    /// lookup; falls back to truncated pubkey hex).
    fn friend_display_name(&self, peer_pubkey_hex: &str) -> String;

    // --- Transport ---

    /// Send a signed-but-not-encrypted `app_message` to a peer
    /// (`send_to_peer_raw` in the existing adapter). Used for all
    /// fire-and-forget call signaling envelopes (CallInvite, CallAccept,
    /// CallDecline, CallRinging, CallEnd, CallMediaState, CallReaction,
    /// and all GroupCall* variants).
    async fn send_to_peer(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<(), CallError>;

    // --- Voice integration ---

    /// Start the voice session for an accepted call. The adapter calls
    /// `services::voice::session::start_session` with the derived
    /// `call_key` so audio frames can be AEAD-encrypted.
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer_pubkey_hex: &str,
        call_key: [u8; 32],
        kind: CallKind,
    ) -> Result<(), CallError>;

    /// Tear down the active voice session (CallEnd, decline-while-up,
    /// glare loss, error path). No-op if voice is not currently up.
    async fn shutdown_voice_session(&self);

    /// Returns `true` if voice is currently up. Used by `CallEnd` to
    /// decide whether to emit `voice_was_up: true` (so the UI knows to
    /// dismiss the in-call panel vs. just the ringing panel).
    fn voice_active(&self) -> bool;

    /// W14.1 — pre-create the voice packet mpsc channel and stash both
    /// ends on AppState BEFORE any await in the accept path. The
    /// dispatch loop reads `voice_packet_tx` to forward inbound voice
    /// frames; if it's None when the peer's first packet arrives, the
    /// packet drops at dispatch and the receive jitter buffer never
    /// sees it. Must be called synchronously at the top of the accept
    /// handler (no awaits before this).
    fn pre_stage_voice_channel(&self);

    /// W13.2 — spawn the receiver-side 30 s ring timeout. The adapter
    /// owns the full sequence: sleep → status-check → remove from
    /// registry → persist a `missed_calls` row → emit
    /// `CallSignalEvent::CallMissed`. Lives on the trait (not as a
    /// free helper in `crate::signaling::ring_timer`) because the
    /// timer task needs to call `emit_event` + `persist_missed_call`
    /// across the tokio::spawn boundary, and `&dyn CallSignalingDeps`
    /// is not movable into a `'static` task. The adapter clones its
    /// internal `Arc<AppState>` + `AppHandle` + `DbPool` into the
    /// spawned task and routes through the same emit/persist paths
    /// as the rest of the deps trait.
    fn spawn_incoming_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    );

    /// W13.2 caller-side — symmetric to `spawn_incoming_call_timeout`.
    /// Spawn the 30 s dialing timeout that fires `CallTimedOut` +
    /// persists a missed_calls row + removes the registry entry if
    /// the call is still Outgoing at `expires_at_ms`.
    fn spawn_dialing_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    );

    /// Pure helper exposed via trait so callers don't need to
    /// reach into `OsRng`: generate a fresh hex-encoded call id
    /// (16 random bytes). The crate's `start_dm_call` uses this.
    fn fresh_call_id(&self) -> String {
        let mut buf = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut buf);
        hex::encode(buf)
    }

    // --- Persistence ---

    /// Record a missed call in the local `missed_calls` SQLite table.
    /// Best-effort; signaling logs warnings on failure but does not
    /// fail the call-state transition.
    fn persist_missed_call(
        &self,
        call_id: &str,
        peer_pubkey_hex: &str,
        kind: CallKind,
        expired_at_ms: u64,
    );

    // --- UI ---

    /// Bring the incoming-call window to the front (or open it if
    /// closed). Best-effort; failure does not fail the signaling
    /// transition.
    fn surface_window_for_call(&self, call_id: &str);

    /// Emit a domain event. The adapter maps each `CallSignalEvent`
    /// variant to its concrete `ChatEvent` / `NotificationEvent`
    /// Tauri payload and calls `event_emit::emit_live` or
    /// `emit_journaled` as appropriate.
    fn emit_event(&self, event: CallSignalEvent);

    // --- Background tasks ---

    /// Register a spawned background task (ring-timer) so it can be
    /// aborted on app shutdown. The adapter pushes onto
    /// `state.background_handles`.
    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>);
}
