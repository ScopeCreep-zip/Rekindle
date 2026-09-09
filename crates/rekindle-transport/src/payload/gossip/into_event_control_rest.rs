//! Tail of [`ControlPayload::into_event`] — see `into_event_control.rs`
//! for the split rationale. Handles events, threads, game servers,
//! governance, voice signaling, admin delegation, bootstrap, sync, and
//! system payloads.

use rekindle_types::subscription_events::{
    CryptoEvent, GovernanceEvent, SocialEvent, SubscriptionEvent, SystemEvent, VoiceEvent,
    VoiceScope,
};

/// Every voice signal on the gossip mesh is community-scoped — a DM
/// call never crosses it, so this side only ever builds the one arm.
fn scope(community: String, channel: String) -> VoiceScope {
    VoiceScope::Community { community, channel }
}

use rekindle_protocol::dht::community::envelope::ControlPayload;

/// Tail of [`ControlPayload::into_event`] — see that method for the split
/// rationale. Handles events, threads, game servers, governance, voice
/// signaling, admin delegation, bootstrap, sync, and system payloads.
/// Earlier variants are routed by `into_event` and never reach the
/// `unreachable!` arm below.
pub fn control_into_event_rest(
    payload: ControlPayload,
    community: &str,
    sender: &str,
) -> SubscriptionEvent {
    let c = || community.to_string();
    let s = || sender.to_string();

    match payload {
        // ── Events ──────────────────────────────────────────
        ControlPayload::EventCreated { event } => {
            SubscriptionEvent::Social(SocialEvent::EventCreated {
                community: c(),
                event: Box::new(event),
            })
        }
        ControlPayload::EventUpdated { event } => {
            SubscriptionEvent::Social(SocialEvent::EventUpdated {
                community: c(),
                event: Box::new(event),
            })
        }
        ControlPayload::EventDeleted { event_id } => {
            SubscriptionEvent::Social(SocialEvent::EventDeleted {
                community: c(),
                event_id,
            })
        }
        ControlPayload::EventRsvpChanged {
            event_id,
            pseudonym_key,
            status,
        } => SubscriptionEvent::Social(SocialEvent::EventRsvpChanged {
            community: c(),
            event_id,
            pseudonym: pseudonym_key,
            rsvp_status: status,
        }),
        ControlPayload::EventReminder {
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
        ControlPayload::ThreadCreated { thread } => {
            SubscriptionEvent::Social(SocialEvent::ThreadCreated {
                community: c(),
                thread: Box::new(thread),
            })
        }
        ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym,
            timestamp,
            reply_to_id,
            ..
        } => SubscriptionEvent::Social(SocialEvent::ThreadMessagePosted {
            community: c(),
            thread_id,
            message_id,
            sender_pseudonym,
            timestamp,
            // The payload carries `ciphertext` and a `mek_generation`,
            // not a plaintext body — decryption happens above this
            // layer, so this decoder cannot fill it.
            body: None,
            reply_to_id,
        }),
        ControlPayload::ThreadArchived {
            thread_id,
            archived,
        } => SubscriptionEvent::Social(SocialEvent::ThreadArchiveChanged {
            community: c(),
            thread_id,
            archived,
        }),

        // ── Game servers ────────────────────────────────────
        ControlPayload::GameServerAdded { server } => {
            SubscriptionEvent::Social(SocialEvent::GameServerAdded {
                community: c(),
                server: Box::new(server),
            })
        }
        ControlPayload::GameServerRemoved { server_id } => {
            SubscriptionEvent::Social(SocialEvent::GameServerRemoved {
                community: c(),
                server_id,
            })
        }

        // ── Governance ──────────────────────────────────────
        ControlPayload::GovernanceUpdated {
            subkey_index,
            lamport_ts,
            ..
        } => SubscriptionEvent::Governance(GovernanceEvent::GovernanceSubkeyUpdated {
            community: c(),
            subkey_index,
            lamport_ts,
        }),

        // ── Voice signaling ─────────────────────────────────
        ControlPayload::VoiceJoin {
            channel_id,
            route_blob,
            display_name,
            ..
        } => {
            // Both were being discarded here, under a comment claiming
            // gossip carried neither. `VoiceJoin` has always had them:
            // the display name rides the handshake so a roster never
            // waits on registry-scan timing, and the route blob is how
            // a peer is reached at all.
            SubscriptionEvent::Voice(VoiceEvent::Joined {
                scope: scope(c(), channel_id),
                pseudonym: s(),
                display_name,
                route_blob: Some(route_blob),
            })
        }
        ControlPayload::VoiceLeave { channel_id } => SubscriptionEvent::Voice(VoiceEvent::Left {
            scope: scope(c(), channel_id),
            pseudonym: s(),
        }),
        ControlPayload::VoiceModeSwitch {
            channel_id,
            mode,
            host_pseudonym,
        } => SubscriptionEvent::Voice(VoiceEvent::ModeChanged {
            scope: scope(c(), channel_id),
            mode,
            host_pseudonym,
        }),
        ControlPayload::VoiceMute {
            channel_id,
            target_pseudonym,
            muted,
        } => SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
            scope: scope(c(), channel_id),
            target_pseudonym,
            muted,
        }),
        ControlPayload::VoiceDeafen {
            channel_id,
            target_pseudonym,
            deafened,
        } => SubscriptionEvent::Voice(VoiceEvent::DeafenChanged {
            scope: scope(c(), channel_id),
            target_pseudonym,
            deafened,
        }),
        ControlPayload::VoiceRoster {
            channel_id,
            participants,
        } => SubscriptionEvent::Voice(VoiceEvent::RosterUpdated {
            scope: scope(c(), channel_id),
            // The roster's whole purpose is telling a joiner *who* is
            // already present, so the members travel with it rather
            // than a count that cannot seed anything.
            participants: participants
                .into_iter()
                .map(|p| rekindle_types::subscription_events::VoiceParticipant {
                    pseudonym_key: p.pseudonym_key,
                    display_name: p.display_name,
                })
                .collect(),
        }),

        // ── Stage signalling ────────────────────────────────
        ControlPayload::StageUpdate {
            channel_id,
            topic,
            speakers,
            moderator_pseudonym,
            ..
        } => SubscriptionEvent::Voice(VoiceEvent::StageUpdated {
            scope: scope(c(), channel_id),
            topic,
            speakers,
            moderator_pseudonym,
        }),
        ControlPayload::SpeakRequest {
            channel_id,
            requester_pseudonym,
            ..
        } => SubscriptionEvent::Voice(VoiceEvent::SpeakRequested {
            scope: scope(c(), channel_id),
            requester_pseudonym,
        }),
        ControlPayload::SpeakResponse {
            channel_id,
            requester_pseudonym,
            granted,
            moderator_pseudonym,
            ..
        } => SubscriptionEvent::Voice(VoiceEvent::SpeakResponded {
            scope: scope(c(), channel_id),
            requester_pseudonym,
            granted,
            moderator_pseudonym,
        }),
        ControlPayload::SoundboardPlay {
            channel_id,
            expression_id,
            actor_pseudonym,
        } => SubscriptionEvent::Voice(VoiceEvent::SoundboardPlayed {
            scope: scope(c(), channel_id),
            expression_id,
            actor_pseudonym,
        }),

        // ── Admin delegation ────────────────────────────────
        ControlPayload::AdminKeypairGrant { .. } => {
            SubscriptionEvent::Crypto(CryptoEvent::AdminKeypairGranted { community: c() })
        }
        ControlPayload::SlotKeypairGrant {
            slot_index,
            segment_index,
            ..
        } => SubscriptionEvent::Crypto(CryptoEvent::SlotKeypairGranted {
            community: c(),
            slot_index,
            segment_index,
        }),

        // ── Bootstrap ───────────────────────────────────────
        ControlPayload::BootstrapRequest {
            joiner_pseudonym, ..
        } => SubscriptionEvent::System(SystemEvent::BootstrapRequested {
            community: c(),
            joiner_pseudonym,
        }),
        ControlPayload::BootstrapResponse { .. } => {
            SubscriptionEvent::System(SystemEvent::BootstrapReceived { community: c() })
        }

        // ── Sync ────────────────────────────────────────────
        ControlPayload::SyncRequest {
            channel_id,
            since_timestamp,
        } => SubscriptionEvent::System(SystemEvent::SyncRequested {
            community: c(),
            channel: channel_id,
            since_timestamp,
        }),
        ControlPayload::SyncResponse {
            channel_id,
            messages,
        } => SubscriptionEvent::System(SystemEvent::SyncReceived {
            community: c(),
            channel: channel_id,
            message_count: messages.len(),
        }),

        // ── System ──────────────────────────────────────────
        ControlPayload::SystemMessage { body, timestamp } => {
            SubscriptionEvent::System(SystemEvent::Announcement {
                community: Some(c()),
                body,
                timestamp,
            })
        }
        ControlPayload::RaidAlert { active } => SubscriptionEvent::System(SystemEvent::RaidAlert {
            community: c(),
            active,
        }),
        ControlPayload::ChannelLockdown { locked } => {
            SubscriptionEvent::System(SystemEvent::ChannelLockdown {
                community: c(),
                locked,
            })
        }
        ControlPayload::KickedNotification => {
            SubscriptionEvent::System(SystemEvent::Kicked { community: c() })
        }

        // Unreachable: every variant above the split is routed here by
        // `into_event`, whose match is exhaustive — a new `ControlPayload`
        // variant is a compile error there, not a silent fall-through.
        _ => unreachable!("into_event_rest received a variant owned by into_event"),
    }
}
