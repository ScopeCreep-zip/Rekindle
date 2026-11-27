//! Phase 14 — `CallSignalingDeps` trait.
//!
//! The orchestration port the rekindle-calls signaling logic talks to.
//! Identity, transport (ratchet-sealed `DmFrame` sends), voice
//! session start/stop, persistence (missed-call log), and frontend
//! emit are all behind this trait — the adapter in `src-tauri/services/
//! calls_adapter.rs` (lands in 14.h) implements it against
//! `AppState` + `tauri::AppHandle` + the transport's
//! `operations::dm::send_to_peer` + `services::voice::*`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::capnp_codec::direct::DirectBody;

use rekindle_types::group_dm::GroupCallRoster;
use rekindle_types::key_format::public_key_hex;
use rekindle_types::subscription_events::CallParticipant;

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

    /// What we hold of the group DM `group_id`: its roster (identity
    /// keys), an unanswered invite to it, or nothing. A group-scoped call
    /// is admitted only between members of it
    /// ([`crate::signaling::check_group_scope`]).
    async fn group_roster(&self, group_id: &str) -> Result<GroupCallRoster, CallError>;

    // --- Mute policy ---

    /// Returns `true` if the peer is temp-muted (W12.12 — `temp_call_muted`
    /// map). Signaling drops their CallInvite payloads silently in that
    /// case (the adapter looks up the map; this trait method abstracts).
    fn is_peer_temp_muted(&self, peer_pubkey_hex: &str) -> bool;

    /// The display name held in the Host's friend list.
    fn friend_display_name(&self, peer_pubkey_hex: &str) -> Option<String>;

    // --- Media capabilities ---

    /// Phase 5 — codecs the LOCAL WebView can decode, as
    /// preference-ordered wire strings ("vp9" / "vp8" / "h264").
    /// Rides `CallInvite` / `CallAccept` so the DM peer's video sender
    /// can intersect its encode set against ours. The adapter reads
    /// `video_session::reported_local_caps()` (falling back to the
    /// conservative interim default when the probe hasn't run yet).
    fn local_video_decode_codecs(&self) -> Vec<String>;

    // --- Transport ---

    /// Send one call kind to a peer as a ratchet-sealed `DmFrame` (plan
    /// E2.3, §6 0x0200–0x0214). Used for all fire-and-forget call
    /// signaling (CallInvite, CallAccept, CallDecline, CallRinging,
    /// CallEnd, CallMediaState, CallReaction, and every GroupCall* kind).
    async fn send_to_peer(&self, peer_pubkey_hex: &str, body: DirectBody) -> Result<(), CallError>;

    /// Commit a `CallEnd` (`body`) to `peer_pubkey_hex` through the outbox,
    /// retried until it leaves or `deadline_ms` (ms since the epoch) passes
    /// (plan CE1, decision 22). Returns once the row is stored.
    async fn commit_call_end(
        &self,
        peer_pubkey_hex: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError>;

    /// Commit call `call_id`'s `CallInvite` (`body`) to `peer_pubkey_hex`
    /// through the outbox, due by `deadline_ms` (the invite's `expires_at`):
    /// re-sent until [`Self::settle_call_invite`] or the deadline (plan CE3,
    /// lead decision 30). Returns once the row is stored.
    async fn commit_call_invite(
        &self,
        peer_pubkey_hex: &str,
        call_id: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError>;

    /// Settle call `call_id`'s invite row: `answered` when the callee's
    /// `CallRinging`, accept or decline arrived, else (we cancelled, lost
    /// glare) it is discarded. A no-op when no row waits; a storage
    /// failure is logged (the row then fails at its deadline).
    async fn settle_call_invite(&self, call_id: &str, answered: bool);

    // --- Clock and media liveness (RFC 7675 consent, plan CE1) ---

    /// Now, ms since the epoch, from the host's clock port.
    fn now_ms(&self) -> u64;

    /// The latest media-plane proof of life (a voice packet or a verified
    /// receiver report) of `call_id`'s media, on the clock [`Self::now_ms`]
    /// reads; `None` before any arrived, or when the running media is not
    /// `call_id`'s (plan CE2, lead decision 30: media is per call).
    fn last_inbound_media_ms(&self, call_id: &str) -> Option<u64>;

    /// Watch an Active call's consent: run
    /// [`crate::signaling::lifecycle::check_consent`] with `since_ms`
    /// until it answers other than `Live`, waiting its `recheck_in_ms`
    /// between checks. Spawned by the adapter (the task cannot borrow
    /// `&self`).
    fn spawn_consent_watch(&self, call_id: String, since_ms: u64);

    // --- Voice integration ---

    /// Start the voice session for an accepted call, owned by `call_id`
    /// (plan CE2, lead decision 30): its SFrame keys are `call_id`'s, read
    /// from the registry, and only [`Self::shutdown_voice_session`] for
    /// `call_id` stops it. Media still running for another call is torn
    /// down first, before this call's starts.
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer_pubkey_hex: &str,
        kind: CallKind,
    ) -> Result<(), CallError>;

    /// Tear down `call_id`'s media: its voice session, frame channels,
    /// egress and ingress (CallEnd, decline-while-up, glare loss, error
    /// path). A no-op when the running media is another call's, or none
    /// (plan CE2, lead decision 30: ending one call never stops another's).
    async fn shutdown_voice_session(&self, call_id: &str);

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
    /// internal `Arc<AppState>` + `AppHandle` + `Db` into the
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

    /// Show the controls of a 1:1 call that just connected, on either
    /// side. The desktop opens the call's own window; a TUI would raise
    /// its call pane. Best-effort, like `surface_window_for_call`.
    fn present_active_call(&self, call_id: &str);

    /// Emit a domain event. The adapter maps each `CallSignalEvent`
    /// variant to its concrete `ChatEvent` / `NotificationEvent`
    /// Tauri payload and calls `event_emit::emit_live` or
    /// `emit_journaled` as appropriate.
    fn emit_event(&self, event: CallSignalEvent);
}

/// The name shown for a DM call peer.
///
/// # Errors
/// The public key is malformed.
pub fn friend_shown_name(
    public_key: &str,
    friend_name: Option<String>,
) -> Result<String, CallError> {
    let key = public_key_hex(public_key).map_err(|error| CallError::invalid(error.to_string()))?;
    Ok(friend_name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| rekindle_crypto::identity::short_key_fingerprint(&key.to_bytes())))
}

pub(super) fn peer_shown_name<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    public_key: &str,
) -> Result<String, CallError> {
    friend_shown_name(public_key, deps.friend_display_name(public_key))
}

pub(super) fn named_participants<D: CallSignalingDeps + ?Sized>(
    deps: &D,
    participants: &[String],
) -> Result<Vec<CallParticipant>, CallError> {
    participants
        .iter()
        .map(|key| {
            Ok(CallParticipant {
                display_name: peer_shown_name(deps, key)?,
                public_key: key.clone(),
            })
        })
        .collect()
}
