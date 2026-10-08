//! Call signalling — ring, answer, decline, media state, participants.
//!
//! ## Why this family exists
//!
//! The whole call subsystem lived in the desktop's `channels::ChatEvent`
//! and had no Tier 1 representation at all. The alignment audit recorded
//! it as the largest single CLI gap: 13 call commands, zero daemon
//! equivalents, so a CLI could not place, answer, or observe a call.
//!
//! It also does not belong in `ChannelMessageEvent`. A call is not a
//! message; it shared a channel with messages only because the desktop
//! had one `chat-event` channel to put things on.
//!
//! ## Why 12 variants where the desktop had 16
//!
//! The desktop carried a 1:1 and a group variant of the same fact four
//! times over — `IncomingCall`/`IncomingGroupCall`,
//! `CallConnected`/`GroupCallConnected`, `CallEnded`/`GroupCallEnded`,
//! and participant join/leave that only existed for groups. `CallEnded`
//! and `GroupCallEnded` had byte-identical fields.
//!
//! Group-ness is data, not a separate event: it is `is_group` plus the
//! participant list. The one place the two genuinely differ is
//! [`CallEvent::Connected`], because a group call has no single peer —
//! that is [`DirectCallInfo`], present for 1:1 and absent for a group,
//! rather than four `Option` fields that must all be set or all unset
//! together.
//!
//! ## Wire constraints
//!
//! Same as [`super::presence`]: postcard on the daemon IPC, so no
//! `#[serde(flatten)]` and no `tag = "..."` enums.

use serde::{Deserialize, Serialize};

/// The peer on the other end of a **direct** call.
///
/// Absent for a group call, where there is no single other end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectCallInfo {
    /// `"audio"` or `"video"`.
    pub kind: String,
    pub peer_key: String,
    pub peer_display_name: String,
    /// Whether the local camera is expected to be live once connected,
    /// so the UI can reserve the self-view before the first frame.
    pub expected_local_camera: bool,
}

/// Call lifecycle and in-call signalling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CallEvent {
    /// Somebody is calling us.
    Incoming {
        call_id: String,
        /// Caller's public key.
        from: String,
        display_name: String,
        /// `"audio"` or `"video"`.
        kind: String,
        /// Everyone invited, for a group call. Empty for a 1:1.
        participants: Vec<String>,
        is_group: bool,
        expires_at_ms: u64,
    },

    /// Our outbound call reached the peer and is ringing there.
    Ringing { call_id: String },

    /// We placed a call; it is now pending an answer.
    Started {
        call_id: String,
        kind: String,
        peer_key: String,
        peer_display_name: String,
        expires_at_ms: u64,
    },

    /// The call is up and media should flow.
    Connected {
        call_id: String,
        /// `None` for a group call: the group-connected signal carries
        /// only the call id, because the kind and the participants were
        /// established when the call came in and have not changed.
        direct: Option<DirectCallInfo>,
    },

    /// The peer declined.
    Declined { call_id: String, reason: String },

    /// Nobody answered here, and the call is now a missed-call row.
    Missed { call_id: String, from: String },

    /// The ring expired before anyone answered.
    TimedOut { call_id: String },

    /// The call is over. Covers 1:1 and group alike — the desktop's two
    /// variants for this had identical fields.
    Ended { call_id: String, reason: String },

    /// A participant's media toggles changed mid-call.
    MediaStateChanged {
        call_id: String,
        audio: bool,
        video: bool,
        screen: bool,
        timestamp_ms: u64,
    },

    /// An in-call emoji reaction.
    ReactionReceived {
        call_id: String,
        sender: String,
        emoji: String,
        timestamp_ms: u64,
    },

    /// Someone joined an in-progress call.
    ParticipantJoined {
        call_id: String,
        participant_pubkey: String,
    },

    /// Someone left an in-progress call.
    ParticipantLeft {
        call_id: String,
        participant_pubkey: String,
        reason: String,
    },
}

impl CallEvent {
    /// The call this event belongs to. Every variant has one.
    #[must_use]
    pub fn call_id(&self) -> &str {
        match self {
            Self::Incoming { call_id, .. }
            | Self::Ringing { call_id }
            | Self::Started { call_id, .. }
            | Self::Connected { call_id, .. }
            | Self::Declined { call_id, .. }
            | Self::Missed { call_id, .. }
            | Self::TimedOut { call_id }
            | Self::Ended { call_id, .. }
            | Self::MediaStateChanged { call_id, .. }
            | Self::ReactionReceived { call_id, .. }
            | Self::ParticipantJoined { call_id, .. }
            | Self::ParticipantLeft { call_id, .. } => call_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_call_connects_without_a_peer() {
        let group = CallEvent::Connected {
            call_id: "c1".into(),
            direct: None,
        };
        let one_to_one = CallEvent::Connected {
            call_id: "c2".into(),
            direct: Some(DirectCallInfo {
                kind: "video".into(),
                peer_key: "pk".into(),
                peer_display_name: "Ada".into(),
                expected_local_camera: true,
            }),
        };
        assert_eq!(group.call_id(), "c1");
        assert_eq!(one_to_one.call_id(), "c2");
        // The four direct-call facts travel together or not at all —
        // there is no state where the peer key is known but the kind
        // is not.
        assert!(matches!(group, CallEvent::Connected { direct: None, .. }));
    }

    #[test]
    fn every_variant_reports_its_call_id() {
        for event in sample_events() {
            assert!(
                !event.call_id().is_empty(),
                "every call event names its call"
            );
        }
    }

    /// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
    /// there at runtime, so only a real round trip catches them.
    #[test]
    fn postcard_round_trips_every_variant() {
        for event in sample_events() {
            let bytes = postcard::to_allocvec(&event).expect("postcard encode");
            let back: CallEvent = postcard::from_bytes(&bytes).expect("postcard decode");
            assert_eq!(format!("{event:?}"), format!("{back:?}"));
        }
    }

    /// Pins the JSON `src/ipc/channels/call_events.ts` parses.
    #[test]
    fn json_shape_is_what_the_webview_parses() {
        let event = CallEvent::Ended {
            call_id: "c1".into(),
            reason: "hangup".into(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"ended":{"callId":"c1","reason":"hangup"}}"#
        );

        let connected = CallEvent::Connected {
            call_id: "c1".into(),
            direct: Some(DirectCallInfo {
                kind: "audio".into(),
                peer_key: "pk".into(),
                peer_display_name: "Ada".into(),
                expected_local_camera: false,
            }),
        };
        assert_eq!(
            serde_json::to_string(&connected).unwrap(),
            r#"{"connected":{"callId":"c1","direct":{"kind":"audio","peerKey":"pk","peerDisplayName":"Ada","expectedLocalCamera":false}}}"#
        );
    }

    fn sample_events() -> Vec<CallEvent> {
        vec![
            CallEvent::Incoming {
                call_id: "c".into(),
                from: "pk".into(),
                display_name: "Ada".into(),
                kind: "video".into(),
                participants: vec!["a".into(), "b".into()],
                is_group: true,
                expires_at_ms: 1,
            },
            CallEvent::Ringing {
                call_id: "c".into(),
            },
            CallEvent::Started {
                call_id: "c".into(),
                kind: "audio".into(),
                peer_key: "pk".into(),
                peer_display_name: "Ada".into(),
                expires_at_ms: 1,
            },
            CallEvent::Connected {
                call_id: "c".into(),
                direct: None,
            },
            CallEvent::Declined {
                call_id: "c".into(),
                reason: "busy".into(),
            },
            CallEvent::Missed {
                call_id: "c".into(),
                from: "pk".into(),
            },
            CallEvent::TimedOut {
                call_id: "c".into(),
            },
            CallEvent::Ended {
                call_id: "c".into(),
                reason: "hangup".into(),
            },
            CallEvent::MediaStateChanged {
                call_id: "c".into(),
                audio: true,
                video: false,
                screen: false,
                timestamp_ms: 1,
            },
            CallEvent::ReactionReceived {
                call_id: "c".into(),
                sender: "pk".into(),
                emoji: "👍".into(),
                timestamp_ms: 1,
            },
            CallEvent::ParticipantJoined {
                call_id: "c".into(),
                participant_pubkey: "pk".into(),
            },
            CallEvent::ParticipantLeft {
                call_id: "c".into(),
                participant_pubkey: "pk".into(),
                reason: "left".into(),
            },
        ]
    }
}
