//! [`Effect`]: what the interpreter does after an input.

use rekindle_types::subscription_events::{CallEvent, FocusReason};

use crate::state::CallKind;

/// Which timer of a call (each fires the matching [`super::CallInput`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallTimer {
    /// The caller's ring → `LocalDialingTimeout`.
    Dialing,
    /// The callee's ring → `LocalIncomingTimeout`.
    Incoming,
    /// The next consent check → `ConsentCheck`.
    Consent,
}

/// Side effects of [`super::CallStateMachine::apply`], in firing order.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Commit the invite as an outbox row, re-sent until settled or
    /// `expires_at_ms` (plan CE3). A failed commit feeds `LocalUnreachable`.
    SendCallInvite {
        recipient: String,
        call_id: String,
        offer_kind: u8,
        initiator_pubkey: String,
        initiator_x25519_pub: [u8; 32],
        expires_at_ms: u64,
        video_decode_codecs: Vec<String>,
    },
    /// The callee holds the invite and rings (it settles the caller's row).
    SendCallRinging { recipient: String, call_id: String },
    SendCallAccept {
        recipient: String,
        call_id: String,
        acceptor_x25519_pub: [u8; 32],
        video_decode_codecs: Vec<String>,
    },
    SendCallDecline {
        recipient: String,
        call_id: String,
        reason: String,
    },
    /// Commit `CallEnd` as an outbox row due by `deadline_ms` (plan CE1):
    /// always after the local end.
    SendCallEnd {
        recipient: String,
        call_id: String,
        reason: String,
        deadline_ms: u64,
    },
    SendCallMediaState {
        recipient: String,
        call_id: String,
        audio: bool,
        video: bool,
        screen: bool,
        timestamp_ms: u64,
    },
    SendCallReaction {
        recipient: String,
        call_id: String,
        emoji: String,
        timestamp_ms: u64,
    },
    /// Settle the invite's outbox row: `answered` (ringing, accept or
    /// decline arrived) or discarded (we cancelled, lost glare).
    SettleInvite { call_id: String, answered: bool },
    /// Start `call_id`'s media; success feeds `VoiceTransportUp`, failure
    /// `VoiceTransportDown`. Another call's media is stopped first.
    StartVoiceSession {
        call_id: String,
        peer: String,
        kind: CallKind,
    },
    /// Stop `call_id`'s media, only if it owns the running media (plan CE2).
    StopVoiceSession { call_id: String, reason: String },
    /// Arm `timer` for `call_id` at `at_ms` (replacing that timer).
    ArmTimer {
        call_id: String,
        timer: CallTimer,
        at_ms: u64,
    },
    /// Disarm every timer of `call_id`.
    CancelTimers { call_id: String },
    /// Write a `missed_calls` row (`rekindle_db::repo::calls::record_missed`).
    PersistMissedCall {
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expired_at_ms: u64,
    },
    /// Deliver an event to subscribers.
    Emit(CallEvent),
    /// Show the controls of a call that just connected (the call window).
    PresentActiveCall { call_id: String },
    /// Bring the incoming-call surface forward.
    SurfaceCallWindow { call_id: String },
    /// Bring the conversation with `peer` forward
    /// (delivered as `CallEvent::FocusConversation`).
    FocusConversation {
        call_id: String,
        peer: String,
        display_name: String,
        reason: FocusReason,
    },
}
