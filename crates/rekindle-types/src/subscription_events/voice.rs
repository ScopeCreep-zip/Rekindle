//! Voice events — join, leave, mute, deafen, mode, roster, and the
//! local session's own state.
//!
//! ## Why local-session events live here
//!
//! This enum used to hold only what *gossip* said about a community
//! voice channel, while the desktop kept a parallel
//! `channels::VoiceEvent` for what the *local audio engine* was doing —
//! device changes, speaking state, packet loss, connection quality.
//! A CLI voice client needs all of that too (connection quality display
//! is an open roadmap item), and splitting by "who observed it" left
//! neither vocabulary able to describe a call.
//!
//! ## Why [`VoiceScope`] rather than `community` + `channel`
//!
//! Every variant used to require a `community: String`, so a DM call —
//! which the desktop has always supported, tagged `active_call_type:
//! "dm"` — could not be expressed at all. `VoiceScope` names either a
//! community channel or a DM peer, so one vocabulary covers both call
//! types instead of one silently excluding the other.
//!
//! ## Wire constraints
//!
//! Same as [`super::presence`]: this crosses the daemon IPC as postcard,
//! which is not self-describing, so no `#[serde(flatten)]` and no
//! `tag = "..."` enums. Everything here stays externally tagged.

use serde::{Deserialize, Serialize};

/// The local three-way voice join handshake stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinHandshake {
    /// We announced the join.
    #[default]
    Announced,
    /// A peer has seen us.
    Seen,
    /// We confirmed transport readiness.
    Connected,
}

/// Current readiness and its highest-priority blocker, or the ready reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceMediaReady {
    pub ready: bool,
    pub reason: String,
}

/// Where a voice session is taking place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VoiceScope {
    /// A community's voice channel.
    Community { community: String, channel: String },
    /// A direct call with one peer.
    Dm { peer_key: String },
}

impl VoiceScope {
    /// The community this scope belongs to, or `None` for a DM call.
    #[must_use]
    pub fn community(&self) -> Option<&str> {
        match self {
            Self::Community { community, .. } => Some(community.as_str()),
            Self::Dm { .. } => None,
        }
    }

    /// The channel id for a community call, or the peer key for a DM.
    ///
    /// What a UI keys its "current call" state on, either way.
    #[must_use]
    pub fn session_key(&self) -> &str {
        match self {
            Self::Community { channel, .. } => channel.as_str(),
            Self::Dm { peer_key } => peer_key.as_str(),
        }
    }

    /// `"community"` or `"dm"` — the call type the UI switches on.
    #[must_use]
    pub fn call_type(&self) -> &'static str {
        match self {
            Self::Community { .. } => "community",
            Self::Dm { .. } => "dm",
        }
    }
}

/// One participant in a voice roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceParticipant {
    pub pseudonym_key: String,
    /// The member’s shown name from the Host.
    pub display_name: String,
}

/// Voice channel activity, and the local session's own state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VoiceEvent {
    /// A member joined a voice channel.
    /// Triggered by: gossip `ControlPayload::VoiceJoin`.
    Joined {
        scope: VoiceScope,
        pseudonym: String,
        /// The joiner's shown name from the Host.
        display_name: String,
        /// The joiner's private-route blob, when their announcement
        /// carried one.
        ///
        /// The desktop ignores it — its voice transport resolves the
        /// route separately — but a CLI voice client has no other way
        /// to reach the peer, so dropping it here would make one
        /// impossible to build.
        route_blob: Option<Vec<u8>>,
    },
    /// A member left a voice channel.
    /// Triggered by: gossip `ControlPayload::VoiceLeave`.
    Left {
        scope: VoiceScope,
        pseudonym: String,
    },
    /// The voice channel mode changed (e.g., stage mode, host change).
    /// Triggered by: gossip `ControlPayload::VoiceModeSwitch`.
    ModeChanged {
        scope: VoiceScope,
        mode: String,
        host_pseudonym: Option<String>,
    },
    /// A participant's mute state changed.
    /// Triggered by: gossip `ControlPayload::VoiceMute`.
    MuteChanged {
        scope: VoiceScope,
        target_pseudonym: String,
        muted: bool,
    },
    /// A participant's deafen state changed.
    /// Triggered by: gossip `ControlPayload::VoiceDeafen`.
    DeafenChanged {
        scope: VoiceScope,
        target_pseudonym: String,
        deafened: bool,
    },
    /// Full voice roster update (authoritative participant list).
    /// Triggered by: gossip `ControlPayload::VoiceRoster`.
    ///
    /// Architecture §10.1/§10.5 — a present member's catch-up roster,
    /// telling a joiner about everyone already in the channel,
    /// including those whose `Joined` we never received. It carries the
    /// participants rather than a count for exactly that reason: a
    /// count cannot seed a roster.
    RosterUpdated {
        scope: VoiceScope,
        participants: Vec<VoiceParticipant>,
    },

    /// Progress through the three-leg voice join handshake.
    JoinHandshake {
        scope: VoiceScope,
        state: JoinHandshake,
        /// The peer this leg concerns, when it names one.
        peer: Option<String>,
        display_name: Option<String>,
    },

    /// A peer completed the handshake and is transport-ready.
    ///
    /// RFC 5104 FIR semantics on the video side: a newly-confirmed peer
    /// needs a full intra frame to start decoding.
    PeerConfirmed {
        scope: VoiceScope,
        pseudonym: String,
    },

    /// Whether our media path for this call is ready to carry audio.
    MediaReady {
        scope: VoiceScope,
        ready: bool,
        /// Why — e.g. `"roster_non_empty"`, `"awaiting_peers"`.
        reason: String,
    },

    /// Stage-mode state: who may speak, and the current topic.
    StageUpdated {
        scope: VoiceScope,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator_pseudonym: String,
    },

    /// A listener asked to speak on stage.
    SpeakRequested {
        scope: VoiceScope,
        requester_pseudonym: String,
    },

    /// A member triggered a soundboard expression in a voice channel.
    ///
    /// Architecture §10.9 — gossiped to every participant, who plays the
    /// cached audio locally. The audio bytes are not carried here; the
    /// expression is looked up by id in the local cache.
    SoundboardPlayed {
        scope: VoiceScope,
        expression_id: String,
        actor_pseudonym: String,
    },

    /// A moderator granted or denied a speak request.
    SpeakResponded {
        scope: VoiceScope,
        requester_pseudonym: String,
        granted: bool,
        moderator_pseudonym: String,
    },

    // ── Local session state ──────────────────────────────────────
    //
    // Observed by our own audio engine rather than reported by a peer.
    /// **We** joined a call, and the audio pipeline is running.
    LocalJoined {
        scope: VoiceScope,
        join_handshake: JoinHandshake,
        media_ready: VoiceMediaReady,
    },

    /// A participant started or stopped speaking (voice activity).
    SpeakingChanged {
        scope: VoiceScope,
        pseudonym: String,
        speaking: bool,
    },

    /// The local audio device changed.
    ///
    /// No scope: a device belongs to the machine, not to a call, and it
    /// can change while no call is running at all.
    DeviceChanged {
        /// `"input"` or `"output"`.
        device_type: String,
        device_name: String,
        /// Why it changed — e.g. `"unplugged"`, `"user"`.
        reason: String,
    },

    /// Packets were dropped somewhere in the local pipeline.
    PacketsDropped {
        scope: VoiceScope,
        reason: String,
        count: u64,
    },

    /// Connection quality, with the drop counters behind the verdict.
    ///
    /// The send loop reports `quality` and the receive loop reports the
    /// `rx_*` counters on independent cadences, so an emitter caches the
    /// halves and every emission carries the full picture.
    ConnectionQuality {
        scope: VoiceScope,
        /// `"good"`, `"fair"`, `"poor"`, `"lost"`, `"recovering"`.
        ///
        /// `lost` and `recovering` are distinct from `poor` on purpose:
        /// a degraded call and a call that dropped and is coming back
        /// need different words, and only the second one resolves by
        /// waiting.
        quality: String,
        rx_overflow_drops: u64,
        rx_late_drops: u64,
        rx_key_drops: u64,
        ingress_drops: u64,
        /// What the far end measured about our outbound stream, from
        /// its RFC 3550 receiver reports. `None` means no peer has
        /// reported yet, and `quality` came from local send-failure
        /// counts — which cannot observe network loss at all.
        link: Option<LinkMeasurement>,
    },
}

/// One window's end-to-end measurement of an outbound media stream, as
/// the far end sees it.
///
/// A flat projection of `rekindle-media-stats`' `ReceptionMetrics` +
/// `QualityScore` for the IPC boundary. Deliberately **not** those
/// types: they are Tier 3 and carry the measurement model (burst/gap
/// accounting, the tracker's internal state), while Tier 1 may only
/// hold plain data and must stay postcard-safe. Converge the two and
/// Tier 1 gains a dependency it is not allowed to have.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkMeasurement {
    /// Packets that never arrived, Q8 (`0..=255` maps to `0.0..=1.0`).
    pub loss_q8: u8,
    /// Packets that arrived unusable — too late, or into a full buffer.
    /// Separate from loss because the fixes are opposite: loss wants
    /// FEC, discard wants a deeper buffer.
    pub discard_q8: u8,
    /// RFC 3550 interarrival jitter, milliseconds.
    pub jitter_ms: u32,
    /// Round trip, when the report's LSR/DLSR echo yielded a
    /// believable one.
    pub rtt_ms: Option<u32>,
    /// G.107 R factor, 0–100.
    pub r_factor: u8,
    /// Listening-quality MOS — "does it sound clean".
    pub mos_lq: f32,
    /// Conversational-quality MOS — "can you hold a conversation".
    pub mos_cq: f32,
    /// The Opus bitrate this measurement led the sender to set.
    pub bitrate_bps: u32,
    /// Estimated one-way route delay, median and 95th percentile, ms
    /// (plan E4.3.0) — `None` until a report yields a round trip.
    pub one_way_p50_ms: Option<u32>,
    pub one_way_p95_ms: Option<u32>,
    /// Estimated mouth-to-ear at the far end: one-way p95 + its playout
    /// depth + the in-process stages, ms.
    pub mouth_to_ear_ms: Option<u32>,
    /// Mouth-to-ear above ITU-T G.114's 400 ms limit.
    pub outside_g114: bool,
}

impl VoiceEvent {
    /// The scope this event belongs to, or `None` for device changes,
    /// which are machine-wide.
    #[must_use]
    pub fn scope(&self) -> Option<&VoiceScope> {
        match self {
            Self::Joined { scope, .. }
            | Self::Left { scope, .. }
            | Self::ModeChanged { scope, .. }
            | Self::MuteChanged { scope, .. }
            | Self::DeafenChanged { scope, .. }
            | Self::RosterUpdated { scope, .. }
            | Self::JoinHandshake { scope, .. }
            | Self::PeerConfirmed { scope, .. }
            | Self::MediaReady { scope, .. }
            | Self::StageUpdated { scope, .. }
            | Self::SpeakRequested { scope, .. }
            | Self::SpeakResponded { scope, .. }
            | Self::SoundboardPlayed { scope, .. }
            | Self::LocalJoined { scope, .. }
            | Self::SpeakingChanged { scope, .. }
            | Self::PacketsDropped { scope, .. }
            | Self::ConnectionQuality { scope, .. } => Some(scope),
            Self::DeviceChanged { .. } => None,
        }
    }

    /// The community this event belongs to, if any.
    #[must_use]
    pub fn community(&self) -> Option<&str> {
        self.scope().and_then(VoiceScope::community)
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
