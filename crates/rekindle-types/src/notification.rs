//! Transport notification types for the IPC boundary.
//!
//! These enums cross the daemon->CLI/TUI boundary. They carry enough
//! context for display decisions ("show a toast", "refresh the peer
//! list") without exposing transport internals.
//!
//! Defined here in `rekindle-types` so that both `rekindle-transport`
//! (producer) and `rekindle-cli` (consumer) can use them without the
//! CLI ever depending on the transport crate.

use serde::{Deserialize, Serialize};

// -- Attachment state --------------------------------------------------------

/// Network attachment state. Maps from Veilid's string representation.
///
/// Ordered by "goodness" -- higher discriminant values indicate stronger
/// attachment. This is a stable ABI contract consumed by CLI display code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AttachmentState {
    Detached = 0,
    Attaching = 1,
    AttachedWeak = 2,
    AttachedGood = 3,
    AttachedStrong = 4,
    FullyAttached = 5,
    OverAttached = 6,
    Detaching = 7,
}

impl AttachmentState {
    /// Parse from Veilid's attachment state string representation.
    ///
    /// Unknown strings map to `Detached` (fail closed -- if we can't parse
    /// the state, we assume the worst).
    pub fn from_veilid_string(s: &str) -> Self {
        match s {
            "Detached" | "detached" => Self::Detached,
            "Attaching" | "attaching" => Self::Attaching,
            "AttachedWeak" | "attached_weak" => Self::AttachedWeak,
            "AttachedGood" | "attached_good" => Self::AttachedGood,
            "AttachedStrong" | "attached_strong" => Self::AttachedStrong,
            "FullyAttached" | "fully_attached" => Self::FullyAttached,
            "OverAttached" | "over_attached" => Self::OverAttached,
            "Detaching" | "detaching" => Self::Detaching,
            _ => Self::Detached,
        }
    }

    /// Whether this state represents an attached (usable) network.
    pub fn is_attached(self) -> bool {
        matches!(
            self,
            Self::AttachedWeak
                | Self::AttachedGood
                | Self::AttachedStrong
                | Self::FullyAttached
                | Self::OverAttached
        )
    }

    /// Human-readable label for display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Attaching => "Attaching",
            Self::AttachedWeak => "AttachedWeak",
            Self::AttachedGood => "AttachedGood",
            Self::AttachedStrong => "AttachedStrong",
            Self::FullyAttached => "FullyAttached",
            Self::OverAttached => "OverAttached",
            Self::Detaching => "Detaching",
        }
    }

    /// Parse from a u8 discriminant. Returns `Detached` for out-of-range values.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Attaching,
            2 => Self::AttachedWeak,
            3 => Self::AttachedGood,
            4 => Self::AttachedStrong,
            5 => Self::FullyAttached,
            6 => Self::OverAttached,
            7 => Self::Detaching,
            _ => Self::Detached,
        }
    }
}

impl std::fmt::Display for AttachmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// -- Transport notification --------------------------------------------------

/// Broadcast notification from the transport layer to CLI/TUI consumers.
///
/// These are summaries of transport events -- not the full event payloads.
/// They carry enough context for display decisions without exposing
/// transport internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransportNotification {
    /// Network attachment state changed.
    AttachmentChanged {
        state: AttachmentState,
        is_attached: bool,
        public_internet_ready: bool,
    },

    /// One or more local private routes died.
    LocalRoutesDied { count: usize },

    /// One or more remote peer routes died.
    RemoteRoutesDied { peer_keys: Vec<String> },

    /// A DHT watch expired or was cancelled.
    WatchDied { record_key: String },

    /// A DM was received from a verified peer.
    DmReceived {
        sender_key: String,
        sender_name: String,
        timestamp: u64,
    },

    /// A new channel message was posted (gossip MessageNotification).
    ///
    /// Carries metadata only -- the message body is in the DHT channel log.
    /// Consumers use this to trigger a history read for the new message.
    MessagePosted {
        community_id: String,
        channel_id: String,
        message_id: String,
        sender_pseudonym: String,
        lamport_ts: u64,
    },

    /// A member is typing in a channel (gossip TypingIndicator).
    ///
    /// Ephemeral -- not stored, expires after ~5 seconds if not refreshed.
    TypingStarted {
        community_id: String,
        channel_id: String,
        sender_pseudonym: String,
    },

    /// A member's presence changed (gossip PresenceUpdate).
    PresenceChanged {
        community_id: String,
        pseudonym_key: String,
        status: String,
        game_name: Option<String>,
    },

    /// A gossip control operation was received that doesn't have a
    /// dedicated notification variant.
    GossipControl {
        community_id: String,
        sender_pseudonym: String,
        lamport_ts: u64,
    },

    /// A DHT value changed (watch notification).
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },

    /// MEK was rotated for a community/channel.
    MekRotated {
        community_id: String,
        channel_id: Option<String>,
        generation: u64,
    },

    /// A voice participant joined.
    VoiceJoin {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },

    /// A voice participant left.
    VoiceLeave {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },

    /// W16 — an envelope was successfully delivered after being queued.
    /// Optional UI surface: "Sending…" → "Sent" indicator on a message
    /// or call-state transition.
    EnvelopeDelivered {
        /// Wire-format string from `EnvelopeKind::as_str()`. UI maps to
        /// its own categorization.
        kind: String,
        /// Optional grouping key (call_id for call envelopes,
        /// conversation peer for DMs, etc.).
        correlation_id: Option<String>,
    },

    /// W16 — an envelope hit max_retries and was dead-lettered. For call
    /// envelopes, the call state machine consumes this and tears down
    /// the call with a clear reason; for DMs the UI shows a
    /// delivery-failed indicator on the message.
    EnvelopeDeliveryFailed {
        /// Wire-format string from `EnvelopeKind::as_str()`.
        kind: String,
        /// Optional grouping key.
        correlation_id: Option<String>,
        /// Last transport error (route lookup failure, network error,
        /// peer offline, etc.).
        last_error: String,
    },

    // ── W16.4: Call lifecycle (1:1) ─────────────────────────────────
    //
    // Emit sites land in W16.6 (operations::calls send paths) and
    // W16.7 (receive dispatch + ring timer). Each frontend (Tauri,
    // CLI, daemon) maps these to its own UI surface — Tauri emits
    // chat-events; CLI prints typed lines; daemon forwards via IPC.

    /// Caller-side: `start_dm_call` enqueued the CallInvite. UI seeds
    /// the OutgoingCallPanel from this payload.
    CallStarted {
        call_id: String,
        /// "audio" | "video".
        kind: String,
        peer_key: String,
        peer_display_name: String,
        /// ms epoch when the ring expires.
        expires_at_ms: u64,
        /// ms epoch when the local CallState was inserted (the moment
        /// the user clicked "Voice Call"). Backend-stamped so all
        /// frontends share the same start time.
        started_at_ms: u64,
        /// "calling" — initial. Transitions to "ringing" / "connecting"
        /// / "active" via [`CallStatusChanged`].
        status: String,
    },

    /// Receiver-side: a CallInvite arrived. UI mounts IncomingCallModal.
    IncomingCall {
        call_id: String,
        /// "audio" | "video".
        kind: String,
        /// Caller's hex-encoded Ed25519 pubkey.
        from: String,
        /// Caller's display name (resolved from friend list / contact
        /// link / pseudonym lookup; empty if unknown).
        display_name: String,
        expires_at_ms: u64,
        /// ms epoch when the CallInvite was received locally.
        received_at_ms: u64,
        /// True for `IncomingGroupCall`-derived flows; false for 1:1.
        /// Frontends with separate group/1:1 UI render accordingly.
        is_group: bool,
    },

    /// Caller-side alerting hint: receiver got the invite and is
    /// ringing the user. Optional UI surface — flips
    /// "Calling…" → "Ringing…" without waiting for accept/decline.
    CallRinging {
        call_id: String,
    },

    /// Voice transport up on both sides. Frontends transition from
    /// OutgoingCallPanel / IncomingCallModal to ActiveCallPanel.
    CallConnected {
        call_id: String,
        /// "audio" | "video".
        kind: String,
        peer_key: String,
        peer_display_name: String,
        /// ms epoch when voice transport started locally.
        started_at_ms: u64,
        /// True when `kind == "video"`. Tauri starts WebCodecs
        /// capture; CLI/daemon ignore (no camera).
        expected_local_camera: bool,
    },

    /// Receiver declined or caller cancelled. Frontend clears the
    /// outgoing/incoming UI slot.
    CallDeclined {
        call_id: String,
        /// Optional reason ("user cancelled", "busy", custom message).
        reason: String,
    },

    /// Active call ended (hangup, peer hung up, network drop with
    /// retry-cap-hit, etc.). Frontend clears `activeCall`.
    CallEnded {
        call_id: String,
        reason: String,
    },

    /// Caller-side timeout: 30 s ring expired with no accept.
    /// Distinct from `CallMissed` (receiver-side) so the UI can show
    /// "no answer" vs. "you missed a call".
    CallTimedOut {
        call_id: String,
    },

    /// W16.5b — Caller-side: the wire-level invite (`app_call`) failed
    /// inside Veilid's 5–10 s RPC budget. The receiver is unreachable
    /// RIGHT NOW (offline, no route, route expired, etc.) — distinct
    /// from `CallTimedOut` (the 30 s user-decision ring expired with
    /// receiver online but not answering). Frontend shows "Couldn't
    /// reach {peer} — they may be offline" and clears the outgoing
    /// call slot immediately, instead of waiting for the 30 s ring
    /// to expire.
    CallUnreachable {
        call_id: String,
        /// Reason classification:
        /// - `"timeout"` — receiver didn't reply within Veilid's RPC
        ///   budget (`network.rpc.timeout_ms`, default 5000 ms,
        ///   doubled to 10000 ms through private routes).
        /// - `"no_route"` — `InvalidTarget` / `NoConnection` — peer
        ///   route blob is stale or peer has no usable route.
        /// - `"service_unavailable"` — peer responded that the
        ///   capability is disabled.
        /// - `"send_failed"` — other transport-level error.
        reason: String,
    },

    /// Receiver-side timeout: 30 s ring expired without the user
    /// answering. Persists a `missed_calls` row.
    CallMissed {
        call_id: String,
        from: String,
    },

    /// State transition not covered by the dedicated variants above
    /// (e.g. Outgoing → Connecting before Connected fires). UI uses
    /// this as a fine-grained progress indicator.
    CallStatusChanged {
        call_id: String,
        /// "calling" | "ringing" | "connecting" | "active" | "ended".
        status: String,
        /// ms epoch of the transition.
        timestamp_ms: u64,
    },

    /// Mid-call peer media toggle (mic / camera / screen-share).
    /// Frontend mounts/unmounts the corresponding tile.
    CallMediaStateChanged {
        call_id: String,
        /// Peer's mic active.
        audio: bool,
        /// Peer's camera active.
        video: bool,
        /// Peer's screen-share active.
        screen: bool,
        timestamp_ms: u64,
    },

    /// Mid-call emoji reaction from peer. UI floats the glyph for ~2 s.
    CallReactionReceived {
        call_id: String,
        /// Sender's hex Ed25519 pubkey.
        sender: String,
        /// Single grapheme cluster, length-capped by receiver to
        /// defeat oversized-emoji DoS.
        emoji: String,
        timestamp_ms: u64,
    },

    /// Backend asks frontends to surface the conversation with the peer.
    /// Emitted on call entry (caller-side after CallInvite send;
    /// receiver-side after acceptance). Tauri brings the chat window
    /// forward; CLI switches active conversation context.
    ConversationFocusRequested {
        peer_key: String,
        display_name: String,
        /// "call-started" | "call-accepted" | future reasons.
        reason: String,
    },

    // ── W16.4: Group call lifecycle ─────────────────────────────────

    /// Caller-side: `start_group_call` fanned out invites. Mirror of
    /// [`CallStarted`] for groups.
    GroupCallStarted {
        call_id: String,
        kind: String,
        initiator_pubkey: String,
        /// Hex pubkeys of all invitees.
        participants: Vec<String>,
        expires_at_ms: u64,
        started_at_ms: u64,
    },

    /// Receiver-side: a `GroupCallOffer` arrived. UI mounts the
    /// IncomingGroupCallModal.
    IncomingGroupCall {
        call_id: String,
        kind: String,
        from: String,
        display_name: String,
        participants: Vec<String>,
        expires_at_ms: u64,
        received_at_ms: u64,
    },

    /// Group call has at least one acceptor; frontend transitions to
    /// the GroupCallPanel grid.
    GroupCallConnected {
        call_id: String,
    },

    /// A new participant joined an in-progress group call. Frontend
    /// adds their tile to the grid.
    GroupCallParticipantJoined {
        call_id: String,
        participant_pubkey: String,
    },

    /// A specific invitee rejected the group call.
    GroupCallParticipantDeclined {
        call_id: String,
        participant_pubkey: String,
        reason: String,
    },

    /// A participant left mid-call.
    GroupCallParticipantLeft {
        call_id: String,
        participant_pubkey: String,
        reason: String,
    },

    /// Entire group call has ended (last participant left, or
    /// initiator hung up).
    GroupCallEnded {
        call_id: String,
        reason: String,
    },

    /// Group call invitation expired without acceptance. Receiver-side.
    GroupCallMissed {
        call_id: String,
        from: String,
    },

    // ── W16.4: Voice control (mute / deafen / voice-mode) ───────────
    //
    // Emit sites land in W16.12 (operations::voice). Each event carries
    // `source: "local" | "remote"` so the frontend can distinguish
    // its own toggle (already optimistically reflected in UI) from a
    // peer's mute that needs to update a participant tile.

    MuteChanged {
        /// Sender's mic state: `true` = mic active (NOT muted).
        audio: bool,
        /// Hex Ed25519 pubkey of whoever's mute changed. For local
        /// changes this matches the user's own identity.
        peer_key: String,
        /// "local" | "remote".
        source: String,
    },

    DeafenChanged {
        /// Sender's deafen state.
        deafened: bool,
        peer_key: String,
        /// "local" | "remote".
        source: String,
    },

    VoiceModeChanged {
        /// "voiceActivity" | "pushToTalk".
        mode: String,
    },

    // ── W16.4: Voice transport telemetry ────────────────────────────

    /// Audio packets dropped on receive (bad MEK decrypt, missing
    /// signing key, AEAD failure, replay-window-drop, etc.). Coalesced
    /// per-reason at ~1 Hz so the UI sees one event per reason per
    /// second.
    PacketsDropped {
        /// "mek_decrypt_failed" | "signing_key_missing" |
        /// "call_key_missing" | "aead_decrypt_failed" |
        /// "replay_window_drop" | "no_recv_context" | "channel_full".
        reason: String,
        /// Number of drops in this coalesced window.
        count: u64,
    },

    /// Per-peer connection quality sample, emitted at ~1 Hz during
    /// active calls. UI maps to bars / latency badge / loss indicator.
    ConnectionQuality {
        peer_key: String,
        /// Round-trip time in milliseconds.
        rtt_ms: u32,
        /// Packet loss as percent (0-100).
        loss_percent: u8,
    },

    // ── W16.4: Device hot-swap ──────────────────────────────────────

    /// Audio device changed (Bluetooth headset connect/disconnect, USB
    /// audio interface, default-device change in OS).
    DeviceChanged {
        /// "input" | "output".
        device_type: String,
        /// New device's name (cpal HostId).
        device_name: String,
        /// "hotswap" | "user_select" | "default_changed".
        reason: String,
    },

    // ── W16.4: DM invite reply (W16.10b) ────────────────────────────

    /// Initiator-side: a `DmInviteReply` arrived for a request matched
    /// by `correlation_id`. The expect-reply path in `EnvelopeQueue`
    /// already wakes the awaiting future via `deliver_reply`; this
    /// notification is for shells that want to log / display the
    /// outcome separately.
    DmInviteReplyReceived {
        correlation_id: String,
        /// "accepted" | "declined".
        decision: String,
        /// Present when accepted; carries the new DM record key.
        dm_log_key: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_state_round_trip() {
        for raw in 0..=7u8 {
            let state = AttachmentState::from_u8(raw);
            assert_eq!(state as u8, raw);
        }
        assert_eq!(AttachmentState::from_u8(255), AttachmentState::Detached);
    }

    #[test]
    fn attachment_state_from_veilid_string() {
        assert_eq!(
            AttachmentState::from_veilid_string("FullyAttached"),
            AttachmentState::FullyAttached
        );
        assert_eq!(
            AttachmentState::from_veilid_string("garbage"),
            AttachmentState::Detached
        );
    }

    #[test]
    fn is_attached_correct() {
        assert!(!AttachmentState::Detached.is_attached());
        assert!(!AttachmentState::Attaching.is_attached());
        assert!(AttachmentState::AttachedWeak.is_attached());
        assert!(AttachmentState::AttachedGood.is_attached());
        assert!(AttachmentState::FullyAttached.is_attached());
        assert!(!AttachmentState::Detaching.is_attached());
    }

    #[test]
    fn notification_serializes_round_trip() {
        let notif = TransportNotification::MessagePosted {
            community_id: "abc".into(),
            channel_id: "general".into(),
            message_id: "msg1".into(),
            sender_pseudonym: "alice".into(),
            lamport_ts: 42,
        };
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: TransportNotification = serde_json::from_str(&json).unwrap();
        match parsed {
            TransportNotification::MessagePosted { lamport_ts, .. } => {
                assert_eq!(lamport_ts, 42);
            }
            _ => panic!("wrong variant"),
        }
    }
}
