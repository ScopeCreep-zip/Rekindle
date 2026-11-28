//! [`CallInput`]: one observed change, with the time it was observed.
//!
//! Named `CallInput`, not `CallEvent`, to keep it distinct from
//! `rekindle_types::subscription_events::CallEvent`, the opposite
//! direction: what subscribers observe after the machine has run.

use std::sync::Arc;

use rekindle_secrets::sframe::SframeSender;
use x25519_dalek::StaticSecret;

use crate::state::CallKind;

/// The local user places a call.
pub struct OutgoingCall {
    pub call_id: String,
    pub peer: String,
    pub peer_display_name: String,
    pub kind: CallKind,
    /// Our fresh X25519 secret for this call; its public half rides the invite.
    pub my_x25519_secret: StaticSecret,
    /// Fresh sender state generated outside the machine.
    pub media_sender: Arc<SframeSender>,
    /// Codecs our webview decodes (they ride the invite).
    pub video_decode_codecs: Vec<String>,
    pub now_ms: u64,
}

/// A decoded inbound `CallInvite`, with what the interpreter looked up.
pub struct InboundInvite {
    pub call_id: String,
    /// The authenticated sender's identity key (hex).
    pub from: String,
    /// The sender's friend name, or its short key.
    pub from_display_name: String,
    pub kind: CallKind,
    /// The caller's X25519 public key as received (32 bytes when well formed).
    pub peer_x25519_pub: Vec<u8>,
    /// Fresh local sender state generated outside the machine.
    pub media_sender: Arc<SframeSender>,
    pub expires_at_ms: u64,
    /// Codecs the caller's webview decodes.
    pub video_decode_codecs: Vec<String>,
    /// The sender is on our temp-mute list (W12.12).
    pub temp_muted: bool,
    pub now_ms: u64,
}

/// Inputs to [`super::CallStateMachine::apply`]: a local action, an
/// inbound signal (its `from` is the authenticated sender), a timer, or a
/// media callback. Every input carries `now_ms` from the host's clock.
///
/// `StaticSecret` is move-only, so inputs are built once and consumed.
pub enum CallInput {
    LocalStartCall(OutgoingCall),
    /// Hang up or cancel, in any state: the local end comes first.
    LocalCancel {
        call_id: String,
        reason: String,
        now_ms: u64,
    },
    /// The caller's ring timer fired.
    LocalDialingTimeout {
        call_id: String,
        now_ms: u64,
    },
    /// The invite could not be committed or sent: the peer is unreachable.
    LocalUnreachable {
        call_id: String,
        reason: String,
        now_ms: u64,
    },
    AcceptReceived {
        call_id: String,
        from: String,
        peer_x25519_pub: Vec<u8>,
        video_decode_codecs: Vec<String>,
        now_ms: u64,
    },
    DeclineReceived {
        call_id: String,
        from: String,
        reason: String,
        now_ms: u64,
    },
    RingingReceived {
        call_id: String,
        from: String,
        now_ms: u64,
    },
    InviteReceived(InboundInvite),
    LocalAccept {
        call_id: String,
        my_x25519_secret: StaticSecret,
        /// Codecs our webview decodes (they ride our `CallAccept`).
        video_decode_codecs: Vec<String>,
        now_ms: u64,
    },
    LocalDecline {
        call_id: String,
        reason: String,
        now_ms: u64,
    },
    /// The callee's ring timer fired.
    LocalIncomingTimeout {
        call_id: String,
        now_ms: u64,
    },
    EndReceived {
        call_id: String,
        from: String,
        reason: String,
        now_ms: u64,
    },
    /// The call's media started.
    VoiceTransportUp {
        call_id: String,
        now_ms: u64,
    },
    /// The call's media failed to start or dropped.
    VoiceTransportDown {
        call_id: String,
        reason: String,
        now_ms: u64,
    },
    /// The consent timer fired (RFC 7675 §5.1): the interpreter reads the
    /// latest media-plane proof of life of `call_id`'s own media.
    ConsentCheck {
        call_id: String,
        last_inbound_media_ms: Option<u64>,
        now_ms: u64,
    },
    /// End every held call (logout, quit, the node detaching).
    EndAll {
        reason: String,
        now_ms: u64,
    },
    /// Our media toggles changed.
    LocalMediaState {
        call_id: String,
        audio: bool,
        video: bool,
        screen: bool,
        now_ms: u64,
    },
    /// We sent an in-call reaction.
    LocalReaction {
        call_id: String,
        emoji: String,
        now_ms: u64,
    },
    MediaStateReceived {
        call_id: String,
        from: String,
        audio: bool,
        video: bool,
        screen: bool,
        timestamp_ms: u64,
        now_ms: u64,
    },
    ReactionReceived {
        call_id: String,
        from: String,
        emoji: String,
        timestamp_ms: u64,
        now_ms: u64,
    },
}

impl CallInput {
    /// When the input was observed.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        match self {
            Self::LocalStartCall(c) => c.now_ms,
            Self::InviteReceived(i) => i.now_ms,
            Self::LocalCancel { now_ms, .. }
            | Self::LocalDialingTimeout { now_ms, .. }
            | Self::LocalUnreachable { now_ms, .. }
            | Self::AcceptReceived { now_ms, .. }
            | Self::DeclineReceived { now_ms, .. }
            | Self::RingingReceived { now_ms, .. }
            | Self::LocalAccept { now_ms, .. }
            | Self::LocalDecline { now_ms, .. }
            | Self::LocalIncomingTimeout { now_ms, .. }
            | Self::EndReceived { now_ms, .. }
            | Self::VoiceTransportUp { now_ms, .. }
            | Self::VoiceTransportDown { now_ms, .. }
            | Self::ConsentCheck { now_ms, .. }
            | Self::EndAll { now_ms, .. }
            | Self::LocalMediaState { now_ms, .. }
            | Self::LocalReaction { now_ms, .. }
            | Self::MediaStateReceived { now_ms, .. }
            | Self::ReactionReceived { now_ms, .. } => *now_ms,
        }
    }
}
