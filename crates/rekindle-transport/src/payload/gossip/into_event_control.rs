//! `ControlPayload::into_event` — member lifecycle, moderation,
//! messages, MEK management, roles, channel permissions, reactions and
//! pins. The remaining categories (events, threads, game servers,
//! governance, voice, admin delegation, bootstrap, sync, system) are
//! handled by `into_event_control_rest.rs`.

use rekindle_types::subscription_events::{
    ChannelMessageEvent, CryptoEvent, GovernanceEvent, MembershipEvent, SocialEvent,
    SubscriptionEvent,
};

use rekindle_protocol::dht::community::envelope::ControlPayload;

use super::into_event_control_rest::control_into_event_rest;

/// Convert a control payload into a subscription event.
///
/// Exhaustive over the canonical `ControlPayload` — every variant has a
/// match arm, and a new one is a compile error here.
///
/// `None` when the payload is not a subscription event: nineteen
/// variants are media- or transport-plane traffic — voice signaling,
/// video fragments and FEC, attachment chunks, bandwidth and topology
/// reports — consumed by the subsystems that own them and never
/// surfaced to a subscriber. Saying so in the return type is what stops
/// them being given an invented event just to satisfy the match.
pub fn control_into_event(
    payload: ControlPayload,
    community: &str,
    sender: &str,
) -> Option<SubscriptionEvent> {
    let c = || community.to_string();
    let s = || sender.to_string();

    Some(match payload {
        // ── Member lifecycle ─────────────────────────────────
        ControlPayload::MemberJoinRequest {
            pseudonym_key,
            display_name,
            invite_code,
            ..
        } => SubscriptionEvent::Membership(MembershipEvent::JoinRequested {
            community: c(),
            pseudonym: pseudonym_key,
            display_name,
            has_invite: invite_code.is_some(),
        }),
        ControlPayload::MemberLeave { pseudonym_key } => {
            SubscriptionEvent::Membership(MembershipEvent::Left {
                community: c(),
                pseudonym: pseudonym_key,
            })
        }
        ControlPayload::JoinAccepted {
            mek_generation,
            slot_index,
            ..
        } => SubscriptionEvent::Membership(MembershipEvent::JoinAccepted {
            community: c(),
            mek_generation,
            slot_index,
        }),
        ControlPayload::JoinRejected { reason } => {
            SubscriptionEvent::Membership(MembershipEvent::JoinRejected {
                community: c(),
                reason,
            })
        }
        ControlPayload::MemberJoined {
            pseudonym_key,
            display_name,
            role_ids,
            ..
        } => SubscriptionEvent::Membership(MembershipEvent::Joined {
            community: c(),
            pseudonym: pseudonym_key,
            display_name,
            role_ids,
        }),
        ControlPayload::MemberRemoved { pseudonym_key } => {
            SubscriptionEvent::Membership(MembershipEvent::Removed {
                community: c(),
                pseudonym: pseudonym_key,
            })
        }

        // ── Moderation ──────────────────────────────────────
        ControlPayload::Kick { target_pseudonym } => {
            SubscriptionEvent::Membership(MembershipEvent::Kicked {
                community: c(),
                target_pseudonym,
            })
        }
        ControlPayload::Ban { target_pseudonym } => {
            SubscriptionEvent::Membership(MembershipEvent::Banned {
                community: c(),
                target_pseudonym,
            })
        }
        ControlPayload::Unban { target_pseudonym } => {
            SubscriptionEvent::Membership(MembershipEvent::Unbanned {
                community: c(),
                target_pseudonym,
            })
        }
        ControlPayload::TimeoutMember {
            target_pseudonym,
            duration_seconds,
            reason,
        } => SubscriptionEvent::Membership(MembershipEvent::TimedOut {
            community: c(),
            target_pseudonym,
            duration_seconds,
            reason,
        }),
        ControlPayload::RemoveTimeout { target_pseudonym } => {
            SubscriptionEvent::Membership(MembershipEvent::TimeoutRemoved {
                community: c(),
                target_pseudonym,
            })
        }
        ControlPayload::MemberTimedOut {
            pseudonym_key,
            timeout_until,
        } => SubscriptionEvent::Membership(MembershipEvent::TimeoutStatusChanged {
            community: c(),
            pseudonym: pseudonym_key,
            timeout_until,
        }),

        // ── Messages ────────────────────────────────────────
        ControlPayload::MessageEdited {
            channel_id,
            message_id,
            edited_at,
            ..
        } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited {
            community: c(),
            channel: channel_id,
            message_id,
            edited_at,
            body: None, // populated by enrichment stage
        }),
        ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
            community: c(),
            channel: channel_id,
            message_id,
        }),

        // ── MEK management ──────────────────────────────────
        ControlPayload::MEKRotated {
            channel_id,
            new_generation,
            rotator_pseudonym,
        } => SubscriptionEvent::Crypto(CryptoEvent::MekRotated {
            community: c(),
            channel: channel_id,
            generation: new_generation,
            rotator_pseudonym,
        }),
        ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            ..
        } => SubscriptionEvent::Crypto(CryptoEvent::MekRequested {
            community: c(),
            channel: channel_id,
            needed_generation,
            requester_pseudonym,
        }),
        // Protocol's is a tuple variant wrapping the whole payload;
        // transport's near-copy flattened it. The payload carries the
        // channel as `Option<String>` because the community-wide key has
        // none, so an absent channel means "the community MEK".
        ControlPayload::MekTransfer(transfer) => {
            SubscriptionEvent::Crypto(CryptoEvent::MekTransferred {
                community: transfer.community_id,
                channel: transfer.channel_id,
                generation: transfer.generation,
                sender_pseudonym: transfer.sender_pseudonym,
            })
        }

        // ── Roles ───────────────────────────────────────────
        ControlPayload::MemberRolesChanged {
            pseudonym_key,
            role_ids,
        } => SubscriptionEvent::Membership(MembershipEvent::RolesChanged {
            community: c(),
            pseudonym: pseudonym_key,
            role_ids,
        }),
        ControlPayload::OnboardingComplete {
            pseudonym_key,
            role_ids,
        } => SubscriptionEvent::Membership(MembershipEvent::OnboardingCompleted {
            community: c(),
            pseudonym: pseudonym_key,
            role_ids,
        }),
        ControlPayload::SubmitOnboardingAnswers { answers } => {
            SubscriptionEvent::Membership(MembershipEvent::OnboardingAnswersSubmitted {
                community: c(),
                sender_pseudonym: s(),
                answer_count: answers.len(),
            })
        }

        // ── Channel permissions ─────────────────────────────
        ControlPayload::ChannelOverwriteChanged { channel_id } => {
            SubscriptionEvent::Governance(GovernanceEvent::ChannelPermissionsChanged {
                community: c(),
                channel: channel_id,
            })
        }

        // ── Reactions & pins ────────────────────────────────
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => SubscriptionEvent::Social(SocialEvent::ReactionAdded {
            community: c(),
            channel: channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        }),
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
            community: c(),
            channel: channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        }),
        ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        } => SubscriptionEvent::Social(SocialEvent::MessagePinned {
            community: c(),
            channel: channel_id,
            message_id,
            pinned_by,
        }),
        ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        } => SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
            community: c(),
            channel: channel_id,
            message_id,
        }),

        // The remaining categories (events, threads, game servers,
        // governance, voice, admin delegation, bootstrap, sync, system)
        // are mapped by `into_event_rest` to keep this method under the
        // clippy::too_many_lines limit. They are listed explicitly rather
        // than via `_` so that adding a `ControlPayload` variant remains a
        // compile error here — exhaustiveness is preserved.
        ControlPayload::EventCreated { .. }
        | ControlPayload::EventUpdated { .. }
        | ControlPayload::EventDeleted { .. }
        | ControlPayload::EventRsvpChanged { .. }
        | ControlPayload::EventReminder { .. }
        | ControlPayload::ThreadCreated { .. }
        | ControlPayload::ThreadMessageReceived { .. }
        | ControlPayload::ThreadArchived { .. }
        | ControlPayload::GameServerAdded { .. }
        | ControlPayload::GameServerRemoved { .. }
        | ControlPayload::GovernanceUpdated { .. }
        | ControlPayload::VoiceJoin { .. }
        | ControlPayload::VoiceLeave { .. }
        | ControlPayload::VoiceModeSwitch { .. }
        | ControlPayload::VoiceMute { .. }
        | ControlPayload::VoiceDeafen { .. }
        | ControlPayload::VoiceRoster { .. }
        | ControlPayload::AdminKeypairGrant { .. }
        | ControlPayload::SlotKeypairGrant { .. }
        | ControlPayload::BootstrapRequest { .. }
        | ControlPayload::BootstrapResponse { .. }
        | ControlPayload::SyncRequest { .. }
        | ControlPayload::SyncResponse { .. }
        | ControlPayload::SystemMessage { .. }
        | ControlPayload::RaidAlert { .. }
        | ControlPayload::ChannelLockdown { .. }
        | ControlPayload::KickedNotification => control_into_event_rest(payload, community, sender),

        // ── Not subscription events ──────────────────────────
        //
        // Media and transport plane. Each is consumed by the subsystem
        // that owns it — `rekindle-voice` signaling, the video
        // fragment/FEC pipeline, the files chunk transfer, the MEK
        // rotation ack path — and none has a meaning in a subscriber's
        // event stream. Listed by name rather than caught by `_` so a
        // new variant still has to be classified here.
        ControlPayload::MekTransferAck(_)
        | ControlPayload::RequestSegmentExpansion { .. }
        | ControlPayload::VoiceJoinAck { .. }
        | ControlPayload::VoiceJoinConfirmed { .. }
        | ControlPayload::StageUpdate { .. }
        | ControlPayload::SpeakRequest { .. }
        | ControlPayload::SpeakResponse { .. }
        | ControlPayload::RequestAttachment { .. }
        | ControlPayload::AttachmentChunk { .. }
        | ControlPayload::MultiAttachmentChunk { .. }
        | ControlPayload::SoundboardPlay { .. }
        | ControlPayload::VideoFragment(_)
        | ControlPayload::VideoParityFragment(_)
        | ControlPayload::FrameAck { .. }
        | ControlPayload::KeyframeRequest { .. }
        | ControlPayload::BandwidthEstimate { .. }
        | ControlPayload::MediaCapabilities { .. }
        | ControlPayload::TopologyChange { .. }
        | ControlPayload::LinkPreview { .. } => return None,
    })
}
