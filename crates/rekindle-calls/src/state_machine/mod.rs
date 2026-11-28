//! The pure 1:1 call state machine (plan E4.1 S1).
//!
//! Owns the lifecycle of 1:1 calls: a map of held calls, turned by each
//! [`CallInput`] into [`Effect`]s. No async, no I/O, no clock, no Veilid:
//! every input carries `now_ms`, and an interpreter
//! ([`port::interpret`] over a [`port::CallPort`]) performs the effects:
//! the sends (the invite and `CallEnd` as outbox rows, the rest as sends),
//! the timers, the media start and stop, the events. The Host's call
//! adapter implements the port (E4.1 S7); until then the desktop still runs
//! the live signaling path (`crate::signaling`), whose behaviour this
//! machine carries one for one: the sender checks, temp-mute, glare (the
//! lower identity key keeps its outgoing call; E4.2 changes the rule),
//! re-ringing a repeated invite, busy, superseding a stale call with the
//! same peer ([`CALL_SUPERSEDED`]), settling the invite row, the local end
//! first then a durable `CallEnd`, and RFC 7675 consent
//! ([`CallInput::ConsentCheck`]).
//!
//! Nothing here persists a call: a restart drops the calls it held. E4.2
//! adds its own SQLCipher `media_sessions` row (session id, next seq,
//! per-call secrets, zeroized at the end) so a restarted callee can
//! re-attempt its accept; the old `PersistCallState`/`rehydrate` pair had
//! no interpreter and is gone.
//!
//! # State diagram
//!
//! ```text
//!   LocalStartCall ─▶ Outgoing ──AcceptReceived──▶ Connecting ──VoiceTransportUp──▶ Active
//!     Decline/End/Cancel/DialingTimeout/Unreachable end Outgoing; Down/End/Hangup end
//!     Connecting; End/Hangup/ConsentCheck expiry/EndAll end Active.
//!   InviteReceived ─▶ Incoming ──LocalAccept──▶ Connecting ──Up (sends CallAccept)──▶ Active
//!     LocalDecline/End/IncomingTimeout end Incoming.
//! ```

use std::collections::HashMap;

use crate::state::{CallKind, CallState, CallStatus};

mod effect;
mod incoming;
mod input;
mod media;
mod outgoing;
pub mod port;

pub use crate::signaling::lifecycle::{
    CALL_END_DEADLINE_MS, CALL_SUPERSEDED, CONNECTION_LOST, CONSENT_TIMEOUT_MS,
};
pub use effect::{CallTimer, Effect};
pub use input::{CallInput, InboundInvite, OutgoingCall};

/// The decline reason of a malformed invite.
pub const INVALID_X25519: &str = "invalid x25519 public key";
/// The decline reason a temp-muted caller's invite gets (W12.12): it never rings.
pub const TEMP_MUTED_DECLINE: &str = "user is unavailable";
/// The decline reason the glare winner answers the loser's invite with.
pub const GLARE_OTHER_LOSES: &str = "glare-resolved-other-loses";
/// The `CallEnd` reason the glare loser sends for its own outgoing call.
pub const GLARE_WE_LOST: &str = "glare-resolved-we-lost";
/// Why the local media stops when the outgoing call loses to glare.
pub const GLARE_RESOLVED: &str = "glare-resolved";

/// One held call and what the machine needs beside [`CallState`].
struct Held {
    call: CallState,
    /// The peer's display name, for `Connected` and `FocusConversation`.
    display_name: String,
    /// We placed the call (the caller side).
    outgoing: bool,
    /// Codecs our webview decodes, sent in our `CallAccept`.
    local_decode_codecs: Vec<String>,
    /// When the call became Active: the consent window's start.
    active_since_ms: Option<u64>,
}

impl Held {
    fn voice_up(&self) -> bool {
        matches!(
            self.call.status,
            CallStatus::Active | CallStatus::Connecting
        )
    }
}

/// Pure-logic state machine for 1:1 calls. The interpreter owns one,
/// feeds it every input, and performs the returned effects in order.
pub struct CallStateMachine {
    /// Our identity key (hex): glare compares it with the caller's.
    own_key: String,
    active: HashMap<String, Held>,
    /// Calls that ended, until their ring would have expired: a re-sent
    /// invite of one never rings again.
    ended: HashMap<String, u64>,
}

impl CallStateMachine {
    /// A machine for the identity `own_key` (hex).
    #[must_use]
    pub fn new(own_key: impl Into<String>) -> Self {
        Self {
            own_key: own_key.into(),
            active: HashMap::new(),
            ended: HashMap::new(),
        }
    }

    /// Every held call. For introspection and the call window's snapshot.
    pub fn snapshot(&self) -> Vec<&CallState> {
        self.active.values().map(|h| &h.call).collect()
    }

    /// One held call.
    #[must_use]
    pub fn get(&self, call_id: &str) -> Option<&CallState> {
        self.active.get(call_id).map(|h| &h.call)
    }

    /// Drive one input; the effects come back in the order they fire.
    pub fn apply(&mut self, input: CallInput) -> Vec<Effect> {
        let now_ms = input.now_ms();
        self.ended.retain(|_, until| *until > now_ms);
        match input {
            CallInput::LocalStartCall(call) => self.apply_local_start_call(call),
            CallInput::LocalCancel {
                call_id, reason, ..
            } => self.end_locally(&call_id, &reason, now_ms),
            CallInput::LocalDialingTimeout { call_id, .. } => {
                self.apply_dialing_timeout(&call_id, now_ms)
            }
            CallInput::LocalUnreachable { call_id, .. } => self.apply_unreachable(&call_id),
            CallInput::AcceptReceived {
                call_id,
                from,
                peer_x25519_pub,
                video_decode_codecs,
                now_ms,
            } => self.apply_accept_received(
                &call_id,
                &from,
                &peer_x25519_pub,
                video_decode_codecs,
                now_ms,
            ),
            CallInput::DeclineReceived {
                call_id,
                from,
                reason,
                ..
            } => self.apply_decline_received(&call_id, &from, reason),
            CallInput::RingingReceived { call_id, from, .. } => {
                self.apply_ringing_received(&call_id, &from)
            }
            CallInput::InviteReceived(invite) => self.apply_invite_received(invite),
            CallInput::LocalAccept {
                call_id,
                my_x25519_secret,
                video_decode_codecs,
                ..
            } => self.apply_local_accept(&call_id, my_x25519_secret, video_decode_codecs),
            CallInput::LocalDecline {
                call_id, reason, ..
            } => self.apply_local_decline(&call_id, &reason),
            CallInput::LocalIncomingTimeout { call_id, .. } => {
                self.apply_incoming_timeout(&call_id, now_ms)
            }
            CallInput::EndReceived {
                call_id,
                from,
                reason,
                ..
            } => self.apply_end_received(&call_id, &from, reason),
            CallInput::VoiceTransportUp { call_id, .. } => {
                self.apply_voice_transport_up(&call_id, now_ms)
            }
            CallInput::VoiceTransportDown {
                call_id, reason, ..
            } => self.apply_voice_transport_down(&call_id, &reason, now_ms),
            CallInput::ConsentCheck {
                call_id,
                last_inbound_media_ms,
                ..
            } => self.apply_consent_check(&call_id, last_inbound_media_ms, now_ms),
            CallInput::EndAll { reason, .. } => self.apply_end_all(&reason, now_ms),
            CallInput::LocalMediaState {
                call_id,
                audio,
                video,
                screen,
                ..
            } => self.apply_local_media_state(&call_id, [audio, video, screen], now_ms),
            CallInput::LocalReaction { call_id, emoji, .. } => {
                self.apply_local_reaction(&call_id, emoji, now_ms)
            }
            CallInput::MediaStateReceived {
                call_id,
                from,
                audio,
                video,
                screen,
                timestamp_ms,
                ..
            } => self.apply_media_state_received(
                &call_id,
                &from,
                [audio, video, screen],
                timestamp_ms,
            ),
            CallInput::ReactionReceived {
                call_id,
                from,
                emoji,
                timestamp_ms,
                ..
            } => self.apply_reaction_received(&call_id, &from, emoji, timestamp_ms),
        }
    }

    /// Remove `call_id`, remembering it until its ring would have expired.
    fn take(&mut self, call_id: &str) -> Option<Held> {
        let held = self.active.remove(call_id)?;
        self.ended
            .insert(call_id.to_string(), held.call.expires_at_ms);
        Some(held)
    }

    /// The held call `call_id` when `from` is its peer: every inbound
    /// signal is checked against the call's peer, so a third key cannot
    /// end, decline or ring someone else's call.
    fn held_from(&self, call_id: &str, from: &str) -> Option<&Held> {
        self.active
            .get(call_id)
            .filter(|h| h.call.peer_pubkey == from)
    }
}

/// Wire string of a [`CallKind`] on `CallEvent`s.
#[must_use]
pub fn kind_str(k: CallKind) -> &'static str {
    match k {
        CallKind::Audio => "audio",
        CallKind::Video => "video",
    }
}

#[cfg(test)]
mod tests;
