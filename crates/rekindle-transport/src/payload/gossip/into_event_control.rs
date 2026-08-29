//! `ControlPayload::into_event` — member lifecycle, moderation,
//! messages, MEK management, roles, channel permissions, reactions and
//! pins. The remaining categories (events, threads, game servers,
//! governance, voice, admin delegation, bootstrap, sync, system) are
//! handled by `into_event_control_rest.rs`.

use rekindle_types::subscription_events::{
    ChannelMessageEvent, CryptoEvent, GovernanceEvent, MembershipEvent, SocialEvent,
    SubscriptionEvent,
};

use super::ControlPayload;

impl ControlPayload {
    /// Convert a control payload into a `SubscriptionEvent`.
    /// Exhaustive — every variant has a match arm.
    pub fn into_event(self, community: &str, sender: &str) -> SubscriptionEvent {
        let c = || community.to_string();
        let s = || sender.to_string();

        match self {
            // ── Member lifecycle ─────────────────────────────────
            Self::MemberJoinRequest {
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
            Self::MemberLeave { pseudonym_key } => {
                SubscriptionEvent::Membership(MembershipEvent::Left {
                    community: c(),
                    pseudonym: pseudonym_key,
                })
            }
            Self::JoinAccepted {
                mek_generation,
                slot_index,
                ..
            } => SubscriptionEvent::Membership(MembershipEvent::JoinAccepted {
                community: c(),
                mek_generation,
                slot_index,
            }),
            Self::JoinRejected { reason } => {
                SubscriptionEvent::Membership(MembershipEvent::JoinRejected {
                    community: c(),
                    reason,
                })
            }
            Self::MemberJoined {
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
            Self::MemberRemoved { pseudonym_key } => {
                SubscriptionEvent::Membership(MembershipEvent::Removed {
                    community: c(),
                    pseudonym: pseudonym_key,
                })
            }

            // ── Moderation ──────────────────────────────────────
            Self::Kick { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Kicked {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::Ban { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Banned {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::Unban { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::Unbanned {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::TimeoutMember {
                target_pseudonym,
                duration_seconds,
                reason,
            } => SubscriptionEvent::Membership(MembershipEvent::TimedOut {
                community: c(),
                target_pseudonym,
                duration_seconds,
                reason,
            }),
            Self::RemoveTimeout { target_pseudonym } => {
                SubscriptionEvent::Membership(MembershipEvent::TimeoutRemoved {
                    community: c(),
                    target_pseudonym,
                })
            }
            Self::MemberTimedOut {
                pseudonym_key,
                timeout_until,
            } => SubscriptionEvent::Membership(MembershipEvent::TimeoutStatusChanged {
                community: c(),
                pseudonym: pseudonym_key,
                timeout_until,
            }),

            // ── Messages ────────────────────────────────────────
            Self::MessageEdited {
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
            Self::MessageDeleted {
                channel_id,
                message_id,
            } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
                community: c(),
                channel: channel_id,
                message_id,
            }),

            // ── MEK management ──────────────────────────────────
            Self::MekRotated {
                channel_id,
                new_generation,
                rotator_pseudonym,
            } => SubscriptionEvent::Crypto(CryptoEvent::MekRotated {
                community: c(),
                channel: channel_id,
                generation: new_generation,
                rotator_pseudonym,
            }),
            Self::RequestMek {
                channel_id,
                needed_generation,
                requester_pseudonym,
            } => SubscriptionEvent::Crypto(CryptoEvent::MekRequested {
                community: c(),
                channel: channel_id,
                needed_generation,
                requester_pseudonym,
            }),
            Self::MekTransfer {
                community_id,
                channel_id,
                generation,
                sender_pseudonym,
                ..
            } => SubscriptionEvent::Crypto(CryptoEvent::MekTransferred {
                community: community_id,
                channel: channel_id,
                generation,
                sender_pseudonym,
            }),

            // ── Roles ───────────────────────────────────────────
            Self::MemberRolesChanged {
                pseudonym_key,
                role_ids,
            } => SubscriptionEvent::Membership(MembershipEvent::RolesChanged {
                community: c(),
                pseudonym: pseudonym_key,
                role_ids,
            }),
            Self::OnboardingComplete {
                pseudonym_key,
                role_ids,
            } => SubscriptionEvent::Membership(MembershipEvent::OnboardingCompleted {
                community: c(),
                pseudonym: pseudonym_key,
                role_ids,
            }),
            Self::SubmitOnboardingAnswers { answers } => {
                SubscriptionEvent::Membership(MembershipEvent::OnboardingAnswersSubmitted {
                    community: c(),
                    sender_pseudonym: s(),
                    answer_count: answers.len(),
                })
            }

            // ── Channel permissions ─────────────────────────────
            Self::ChannelOverwriteChanged { channel_id } => {
                SubscriptionEvent::Governance(GovernanceEvent::ChannelPermissionsChanged {
                    community: c(),
                    channel: channel_id,
                })
            }

            // ── Reactions & pins ────────────────────────────────
            Self::ReactionAdded {
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
            Self::ReactionRemoved {
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
            Self::MessagePinned {
                channel_id,
                message_id,
                pinned_by,
            } => SubscriptionEvent::Social(SocialEvent::MessagePinned {
                community: c(),
                channel: channel_id,
                message_id,
                pinned_by,
            }),
            Self::MessageUnpinned {
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
            Self::EventCreated { .. }
            | Self::EventUpdated { .. }
            | Self::EventDeleted { .. }
            | Self::EventRsvpChanged { .. }
            | Self::EventReminder { .. }
            | Self::ThreadCreated { .. }
            | Self::ThreadMessage { .. }
            | Self::ThreadArchived { .. }
            | Self::GameServerAdded { .. }
            | Self::GameServerRemoved { .. }
            | Self::GovernanceUpdated { .. }
            | Self::VoiceJoin { .. }
            | Self::VoiceLeave { .. }
            | Self::VoiceModeSwitch { .. }
            | Self::VoiceMute { .. }
            | Self::VoiceDeafen { .. }
            | Self::VoiceRoster { .. }
            | Self::AdminKeypairGrant { .. }
            | Self::SlotKeypairGrant { .. }
            | Self::BootstrapRequest { .. }
            | Self::BootstrapResponse { .. }
            | Self::SyncRequest { .. }
            | Self::SyncResponse { .. }
            | Self::SystemMessage { .. }
            | Self::RaidAlert { .. }
            | Self::ChannelLockdown { .. }
            | Self::KickedNotification => self.into_event_rest(community, sender),
        }
    }
}
