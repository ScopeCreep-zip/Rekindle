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

/// An invited DM-call participant, named by the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallParticipant {
    pub public_key: String,
    pub display_name: String,
    /// This participant is the identity reading the event.
    pub is_self: bool,
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
        participants: Vec<CallParticipant>,
        is_group: bool,
        expires_at_ms: u64,
        /// The group DM a group call belongs to (`GroupCallOffer.group_id`,
        /// checked against its roster); `None` for a 1:1 or a buddy-list
        /// group call.
        group_id: Option<String>,
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

    /// We started a group call; it rings out to `participants` until the
    /// first of them accepts (`Connected`).
    GroupStarted {
        call_id: String,
        /// `"audio"` or `"video"`.
        kind: String,
        /// Everyone invited, us first.
        participants: Vec<CallParticipant>,
        /// The group DM the call belongs to (`GroupCallOffer.group_id`);
        /// `None` for a buddy-list group call.
        group_id: Option<String>,
        expires_at_ms: u64,
    },

    /// The call is up and media should flow.
    Connected {
        call_id: String,
        /// `None` for a group call: the group-connected signal carries
        /// only the call id, because the kind and the participants were
        /// established when the call came in and have not changed.
        direct: Option<DirectCallInfo>,
        /// The group DM of a group call, as on `Incoming`.
        group_id: Option<String>,
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

    /// Our own microphone mute in the call or voice session changed (the
    /// mute button or the mute shortcut). It names no call: the desktop
    /// call engine holds the mute for whatever session is live, and the
    /// Host's skin signals fold it into `MuteSelf`/`UnmuteSelf` (lead
    /// decision 58).
    SelfMuteChanged { muted: bool, timestamp_ms: u64 },
}

impl CallEvent {
    /// The call this event belongs to; `None` only for
    /// [`CallEvent::SelfMuteChanged`], which is ours, not a call's.
    #[must_use]
    pub fn call_id(&self) -> Option<&str> {
        let id = match self {
            Self::Incoming { call_id, .. }
            | Self::Ringing { call_id }
            | Self::Started { call_id, .. }
            | Self::GroupStarted { call_id, .. }
            | Self::Connected { call_id, .. }
            | Self::Declined { call_id, .. }
            | Self::Missed { call_id, .. }
            | Self::TimedOut { call_id }
            | Self::Ended { call_id, .. }
            | Self::MediaStateChanged { call_id, .. }
            | Self::ReactionReceived { call_id, .. }
            | Self::ParticipantJoined { call_id, .. }
            | Self::ParticipantLeft { call_id, .. } => call_id,
            Self::SelfMuteChanged { .. } => return None,
        };
        Some(id)
    }
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
