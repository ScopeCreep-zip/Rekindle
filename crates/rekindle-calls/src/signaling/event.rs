//! Phase 14 — call-side events the signaling logic emits.
//!
//! The src-tauri adapter maps each variant to its concrete
//! `ChatEvent` / `NotificationEvent` Tauri payload and calls
//! `event_emit::emit_live` (or `emit_journaled` for incoming calls
//! that should survive a hard-quit + cursor replay).
//!
//! Variants cover both 1:1 and group call lifecycles. Where a Tauri
//! command needs to know about a call-state transition (e.g., to
//! refresh the call window), the adapter forwards via `app.emit`.

use crate::state::CallKind;

#[derive(Debug, Clone)]
pub enum CallSignalEvent {
    // --- 1:1 call lifecycle ---
    /// Inbound `CallInvite` arrived; UI should surface the incoming-call
    /// window. Adapter emits both `ChatEvent::IncomingCall` and a
    /// journaled `NotificationEvent::CallIncoming` so cursor replay
    /// after a hard-quit re-surfaces the missed incoming call.
    IncomingCall {
        call_id: String,
        from_public_key: String,
        from_display_name: String,
        kind: CallKind,
        expires_at_ms: u64,
    },
    /// Outbound side: peer is now ringing (received our invite,
    /// pending accept/decline). UI flips "Calling…" → "Ringing…".
    CallRinging {
        call_id: String,
        peer_public_key: String,
    },
    /// Peer accepted our invite; voice session has been started.
    /// W14.2: payload carries `kind` so the adapter can emit
    /// `expected_local_camera = matches!(kind, Video)` — Tauri reads
    /// this to start WebCodecs capture on a video call. CLI/TUI
    /// frontends ignore it naturally.
    CallConnected {
        call_id: String,
        peer_public_key: String,
        kind: CallKind,
    },
    /// W15.6 — caller-side: we just sent CallInvite. Frontends seed
    /// outgoing-call store + start ringback. Adapter maps to the
    /// existing `ChatEvent::CallStarted` payload.
    CallStarted {
        call_id: String,
        peer_public_key: String,
        peer_display_name: String,
        kind: CallKind,
        expires_at_ms: u64,
    },
    /// Peer declined our invite (or invite-side decline).
    CallDeclined {
        call_id: String,
        peer_public_key: String,
        reason: String,
    },
    /// Either side ended an active call.
    CallEnded {
        call_id: String,
        peer_public_key: String,
        reason: String,
        voice_was_up: bool,
    },
    /// 30s ring timeout fired without an accept (outgoing side).
    CallTimedOut {
        call_id: String,
        peer_public_key: String,
        kind: CallKind,
    },
    /// 30s ring timeout fired without our accept (incoming side).
    CallMissed {
        call_id: String,
        peer_public_key: String,
        kind: CallKind,
    },
    /// Frontend: focus the conversation window for this peer.
    /// `reason` is "call-started" (caller-side, after CallInvite send)
    /// or "call-accepted" (either side, after CallConnected).
    /// Frontends MAY surface differently; not load-bearing.
    ConversationFocusRequested {
        peer_public_key: String,
        peer_display_name: String,
        reason: String,
    },

    // --- Group call lifecycle ---
    /// Inbound group call invite. UI surfaces the multi-participant
    /// incoming-call panel.
    IncomingGroupCall {
        call_id: String,
        initiator_public_key: String,
        initiator_display_name: String,
        participants: Vec<String>,
        kind: u8,
        expires_at_ms: u64,
    },
    /// Initiator side: first participant accepted; call is Active.
    GroupCallConnected { call_id: String },
    /// A participant joined the call (accepted invite).
    GroupCallParticipantJoined {
        call_id: String,
        peer_public_key: String,
    },
    /// A participant left or declined the call.
    GroupCallParticipantLeft {
        call_id: String,
        peer_public_key: String,
        reason: String,
    },
    /// Local user ended the call (or last participant left). Adapter
    /// emits `ChatEvent::GroupCallEnded` — distinct from
    /// `GroupCallParticipantLeft` which is fan-out gossip about
    /// someone else leaving.
    GroupCallEnded { call_id: String, reason: String },
}
