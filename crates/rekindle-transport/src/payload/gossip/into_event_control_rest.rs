//! Tail of [`ControlPayload::into_event`] — see `into_event_control.rs`
//! for the split rationale. Handles events, threads, game servers,
//! governance, voice signaling, admin delegation, bootstrap, sync, and
//! system payloads.

use rekindle_types::subscription_events::{
    CryptoEvent, GovernanceEvent, SocialEvent, SubscriptionEvent, SystemEvent, VoiceEvent,
};

use super::ControlPayload;

impl ControlPayload {
    /// Tail of [`ControlPayload::into_event`] — see that method for the split
    /// rationale. Handles events, threads, game servers, governance, voice
    /// signaling, admin delegation, bootstrap, sync, and system payloads.
    /// Earlier variants are routed by `into_event` and never reach the
    /// `unreachable!` arm below.
    pub(super) fn into_event_rest(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();

        match self {
            // ── Events ──────────────────────────────────────────
            Self::EventCreated { event } => SubscriptionEvent::Social(SocialEvent::EventCreated {
                community: c(),
                event_id: event.id,
                title: event.title,
                start_time: event.start_time,
            }),
            Self::EventUpdated { event } => SubscriptionEvent::Social(SocialEvent::EventUpdated {
                community: c(),
                event_id: event.id,
                title: event.title,
            }),
            Self::EventDeleted { event_id } => {
                SubscriptionEvent::Social(SocialEvent::EventDeleted {
                    community: c(),
                    event_id,
                })
            }
            Self::EventRsvpChanged {
                event_id,
                pseudonym_key,
                status,
            } => SubscriptionEvent::Social(SocialEvent::EventRsvpChanged {
                community: c(),
                event_id,
                pseudonym: pseudonym_key,
                rsvp_status: status,
            }),
            Self::EventReminder {
                event_id,
                title,
                minutes_until_start,
            } => SubscriptionEvent::Social(SocialEvent::EventReminder {
                community: c(),
                event_id,
                title,
                minutes_until_start,
            }),

            // ── Threads ─────────────────────────────────────────
            Self::ThreadCreated { thread } => {
                SubscriptionEvent::Social(SocialEvent::ThreadCreated {
                    community: c(),
                    channel: thread.channel_id,
                    thread_id: thread.id,
                    thread_name: thread.name,
                    creator_pseudonym: thread.creator_pseudonym,
                })
            }
            Self::ThreadMessage {
                thread_id,
                message_id,
                sender_pseudonym,
                timestamp,
                ..
            } => SubscriptionEvent::Social(SocialEvent::ThreadMessagePosted {
                community: c(),
                thread_id,
                message_id,
                sender_pseudonym,
                timestamp,
            }),
            Self::ThreadArchived {
                thread_id,
                archived,
            } => SubscriptionEvent::Social(SocialEvent::ThreadArchiveChanged {
                community: c(),
                thread_id,
                archived,
            }),

            // ── Game servers ────────────────────────────────────
            Self::GameServerAdded { server } => {
                SubscriptionEvent::Social(SocialEvent::GameServerAdded {
                    community: c(),
                    server_id: server.id,
                    game_id: server.game_id,
                    label: server.label,
                })
            }
            Self::GameServerRemoved { server_id } => {
                SubscriptionEvent::Social(SocialEvent::GameServerRemoved {
                    community: c(),
                    server_id,
                })
            }

            // ── Governance ──────────────────────────────────────
            Self::GovernanceUpdated {
                subkey_index,
                lamport_ts,
                ..
            } => SubscriptionEvent::Governance(GovernanceEvent::GovernanceSubkeyUpdated {
                community: c(),
                subkey_index,
                lamport_ts,
            }),

            // ── Voice signaling ─────────────────────────────────
            Self::VoiceJoin { channel_id, .. } => SubscriptionEvent::Voice(VoiceEvent::Joined {
                community: c(),
                channel: channel_id,
                pseudonym: s(),
            }),
            Self::VoiceLeave { channel_id } => SubscriptionEvent::Voice(VoiceEvent::Left {
                community: c(),
                channel: channel_id,
                pseudonym: s(),
            }),
            Self::VoiceModeSwitch {
                channel_id,
                mode,
                host_pseudonym,
            } => SubscriptionEvent::Voice(VoiceEvent::ModeChanged {
                community: c(),
                channel: channel_id,
                mode,
                host_pseudonym,
            }),
            Self::VoiceMute {
                channel_id,
                target_pseudonym,
                muted,
            } => SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
                community: c(),
                channel: channel_id,
                target_pseudonym,
                muted,
            }),
            Self::VoiceDeafen {
                channel_id,
                target_pseudonym,
                deafened,
            } => SubscriptionEvent::Voice(VoiceEvent::DeafenChanged {
                community: c(),
                channel: channel_id,
                target_pseudonym,
                deafened,
            }),
            Self::VoiceRoster {
                channel_id,
                participants,
            } => SubscriptionEvent::Voice(VoiceEvent::RosterUpdated {
                community: c(),
                channel: channel_id,
                participant_count: participants.len(),
            }),

            // ── Admin delegation ────────────────────────────────
            Self::AdminKeypairGrant { .. } => {
                SubscriptionEvent::Crypto(CryptoEvent::AdminKeypairGranted { community: c() })
            }
            Self::SlotKeypairGrant {
                slot_index,
                segment_index,
                ..
            } => SubscriptionEvent::Crypto(CryptoEvent::SlotKeypairGranted {
                community: c(),
                slot_index,
                segment_index,
            }),

            // ── Bootstrap ───────────────────────────────────────
            Self::BootstrapRequest {
                joiner_pseudonym, ..
            } => SubscriptionEvent::System(SystemEvent::BootstrapRequested {
                community: c(),
                joiner_pseudonym,
            }),
            Self::BootstrapResponse { .. } => {
                SubscriptionEvent::System(SystemEvent::BootstrapReceived { community: c() })
            }

            // ── Sync ────────────────────────────────────────────
            Self::SyncRequest {
                channel_id,
                since_timestamp,
            } => SubscriptionEvent::System(SystemEvent::SyncRequested {
                community: c(),
                channel: channel_id,
                since_timestamp,
            }),
            Self::SyncResponse {
                channel_id,
                messages,
            } => SubscriptionEvent::System(SystemEvent::SyncReceived {
                community: c(),
                channel: channel_id,
                message_count: messages.len(),
            }),

            // ── System ──────────────────────────────────────────
            Self::SystemMessage { body, timestamp } => {
                SubscriptionEvent::System(SystemEvent::Announcement {
                    community: Some(c()),
                    body,
                    timestamp,
                })
            }
            Self::RaidAlert { active } => SubscriptionEvent::System(SystemEvent::RaidAlert {
                community: c(),
                active,
            }),
            Self::ChannelLockdown { locked } => {
                SubscriptionEvent::System(SystemEvent::ChannelLockdown {
                    community: c(),
                    locked,
                })
            }
            Self::KickedNotification => {
                SubscriptionEvent::System(SystemEvent::Kicked { community: c() })
            }

            // Unreachable: every variant above the split is routed here by
            // `into_event`, whose match is exhaustive — a new `ControlPayload`
            // variant is a compile error there, not a silent fall-through.
            _ => unreachable!("into_event_rest received a variant owned by into_event"),
        }
    }
}
