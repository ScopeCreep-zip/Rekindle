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

/// Voice channel activity, and the local session's own state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VoiceEvent {
    /// A member joined a voice channel.
    /// Triggered by: gossip `ControlPayload::VoiceJoin`.
    Joined {
        scope: VoiceScope,
        pseudonym: String,
        /// Carried when the joiner is known to us by name. Gossip
        /// supplies only the pseudonym; a local join supplies both.
        display_name: Option<String>,
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
    RosterUpdated {
        scope: VoiceScope,
        participant_count: usize,
    },

    // ── Local session state ──────────────────────────────────────
    //
    // Observed by our own audio engine rather than reported by a peer.
    /// **We** joined a call, and the audio pipeline is running.
    LocalJoined { scope: VoiceScope },

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
        /// `"good"`, `"fair"`, `"poor"`.
        quality: String,
        rx_overflow_drops: u64,
        rx_late_drops: u64,
        rx_mek_drops: u64,
        ingress_drops: u64,
    },
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
            | Self::LocalJoined { scope }
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
mod tests {
    use super::*;

    fn community_scope() -> VoiceScope {
        VoiceScope::Community {
            community: "c".into(),
            channel: "ch".into(),
        }
    }

    #[test]
    fn a_dm_call_is_expressible_and_has_no_community() {
        let dm = VoiceScope::Dm {
            peer_key: "peer".into(),
        };
        assert_eq!(dm.community(), None);
        assert_eq!(dm.call_type(), "dm");
        assert_eq!(dm.session_key(), "peer");

        let event = VoiceEvent::LocalJoined { scope: dm };
        assert_eq!(event.community(), None, "a DM call belongs to no community");
    }

    #[test]
    fn device_changes_are_machine_wide() {
        let event = VoiceEvent::DeviceChanged {
            device_type: "input".into(),
            device_name: "Yeti".into(),
            reason: "unplugged".into(),
        };
        assert!(
            event.scope().is_none(),
            "a device can change with no call running"
        );
    }

    #[test]
    fn community_scope_reports_its_community() {
        let event = VoiceEvent::RosterUpdated {
            scope: community_scope(),
            participant_count: 3,
        };
        assert_eq!(event.community(), Some("c"));
        assert_eq!(event.scope().unwrap().session_key(), "ch");
    }

    /// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
    /// there at runtime, so only a real round trip catches them.
    #[test]
    fn postcard_round_trips_every_variant() {
        let events = vec![
            VoiceEvent::Joined {
                scope: community_scope(),
                pseudonym: "p".into(),
                display_name: Some("Ada".into()),
            },
            VoiceEvent::Left {
                scope: community_scope(),
                pseudonym: "p".into(),
            },
            VoiceEvent::ModeChanged {
                scope: community_scope(),
                mode: "stage".into(),
                host_pseudonym: None,
            },
            VoiceEvent::MuteChanged {
                scope: community_scope(),
                target_pseudonym: "p".into(),
                muted: true,
            },
            VoiceEvent::DeafenChanged {
                scope: community_scope(),
                target_pseudonym: "p".into(),
                deafened: false,
            },
            VoiceEvent::RosterUpdated {
                scope: community_scope(),
                participant_count: 4,
            },
            VoiceEvent::LocalJoined {
                scope: VoiceScope::Dm {
                    peer_key: "peer".into(),
                },
            },
            VoiceEvent::SpeakingChanged {
                scope: community_scope(),
                pseudonym: "p".into(),
                speaking: true,
            },
            VoiceEvent::DeviceChanged {
                device_type: "input".into(),
                device_name: "Yeti".into(),
                reason: "user".into(),
            },
            VoiceEvent::PacketsDropped {
                scope: community_scope(),
                reason: "voice loop".into(),
                count: 12,
            },
            VoiceEvent::ConnectionQuality {
                scope: community_scope(),
                quality: "good".into(),
                rx_overflow_drops: 1,
                rx_late_drops: 2,
                rx_mek_drops: 3,
                ingress_drops: 4,
            },
        ];
        for event in events {
            let bytes = postcard::to_allocvec(&event).expect("postcard encode");
            let back: VoiceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
            assert_eq!(format!("{event:?}"), format!("{back:?}"));
        }
    }

    /// Pins the JSON `src/ipc/channels/voice_events.ts` parses.
    #[test]
    fn json_shape_is_what_the_webview_parses() {
        let event = VoiceEvent::LocalJoined {
            scope: community_scope(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"localJoined":{"scope":{"community":{"community":"c","channel":"ch"}}}}"#
        );

        let dm = VoiceEvent::Joined {
            scope: VoiceScope::Dm {
                peer_key: "peer".into(),
            },
            pseudonym: "p".into(),
            display_name: None,
        };
        assert_eq!(
            serde_json::to_string(&dm).unwrap(),
            r#"{"joined":{"scope":{"dm":{"peerKey":"peer"}},"pseudonym":"p","displayName":null}}"#
        );
    }
}
