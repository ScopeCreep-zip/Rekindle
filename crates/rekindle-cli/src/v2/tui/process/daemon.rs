//! Daemon event processing — subscription events and command results.
//! Exhaustive match on every variant. No catchall.

use std::collections::VecDeque;

use rekindle_types::display::{DecryptedMessageDisplay, DeliveryStatus};
use rekindle_types::subscription_events::*;

use crate::v2::helpers;
use rekindle_types::daemon::DaemonResponse;
use rekindle_types::display as dt;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::DaemonEvent;
use crate::v2::tui::state::channels::PinDisplay;
use crate::v2::tui::reconnect;
use crate::v2::tui::state::channels::{ChannelKey, ChannelViewState};
use crate::v2::tui::state::communities::CommunitySnapshot;
use crate::v2::tui::state::dm::{DmMessage, DmThreadState};
use crate::v2::tui::state::ephemeral::{
    RailSignal, SignalPriority, SignalScope, ToastLevel, TypingKey,
};
use crate::v2::tui::state::friends::{presence_rank, PendingFriendRequest};
use crate::v2::tui::state::in_flight::{PendingSend, RequestKind};
use crate::v2::tui::state::ephemeral::IdentitySnapshot;
use crate::v2::tui::state::navigation::ViewKind;
use crate::v2::tui::state::TuiState;

pub fn process_daemon(event: &DaemonEvent, state: &mut TuiState) -> Vec<Effect> {
    match event {
        DaemonEvent::Subscription(sub) => {
            tracing::info!("tui: subscription event");
            process_subscription(sub, state)
        }
        DaemonEvent::CommandResult { request_id, response } => {
            let kind_label = state.in_flight.peek_request_kind(*request_id);
            let body_len = match response {
                DaemonResponse::Ok(bytes) => bytes.len(),
                DaemonResponse::Error { .. } => 0,
            };
            tracing::info!(
                request_id, kind = ?kind_label, body_len,
                is_error = matches!(response, DaemonResponse::Error { .. }),
                "tui: command result"
            );
            process_command_result(*request_id, response, state)
        }
        DaemonEvent::CommandFailed { request_id, error } => {
            tracing::warn!(request_id, error, "tui: command failed");
            process_command_failed(*request_id, error, state)
        }
        DaemonEvent::ConnectionLost => {
            tracing::warn!("tui: connection lost");
            reconnect::begin(state)
        }
    }
}

fn process_subscription(event: &SubscriptionEvent, state: &mut TuiState) -> Vec<Effect> {
    let mut effects = vec![];

    match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community,
            channel,
            message_id,
            sender_pseudonym,
            sequence,
            timestamp,
            body,
            reply_to_sequence,
            is_self,
            ..
        }) => {
            tracing::info!(
                community = &community[..12.min(community.len())],
                channel, is_self, sequence,
                has_body = body.is_some(),
                "tui: ChannelMessage::New"
            );
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            let ch = state.channels.channels.entry(key).or_insert_with(ChannelViewState::new);

            if *is_self {
                let _ = ch.confirm_last_sending(message_id);
            } else if let Some(ref body_text) = body {
                if !ch.try_enrich_placeholder(
                    sender_pseudonym, *timestamp, message_id,
                    body_text, *sequence, *reply_to_sequence,
                ) {
                    let _ = ch.push_message(DecryptedMessageDisplay {
                        message_id: message_id.clone(),
                        sequence: *sequence,
                        author_pseudonym: sender_pseudonym.clone(),
                        author_display_name: sender_pseudonym.clone(),
                        body: body_text.clone(),
                        timestamp: *timestamp,
                        reply_to_sequence: *reply_to_sequence,
                        mek_generation: 0,
                        is_encrypted: false,
                        needs_mek: None,
                        delivery_status: DeliveryStatus::Confirmed,
                        thread_id: None,
                    });
                }
            } else {
                let _ = ch.push_message(DecryptedMessageDisplay {
                    message_id: message_id.clone(),
                    sequence: *sequence,
                    author_pseudonym: sender_pseudonym.clone(),
                    author_display_name: sender_pseudonym.clone(),
                    body: "(decrypting...)".into(),
                    timestamp: *timestamp,
                    reply_to_sequence: *reply_to_sequence,
                    mek_generation: 0,
                    is_encrypted: true,
                    needs_mek: Some(0),
                    delivery_status: DeliveryStatus::Confirmed,
                    thread_id: None,
                });
            }

            if !*is_self && state.nav.tab_bar.selected_id() != Some("communities") {
                state.nav.tab_bar.increment_unread("communities");
            }
        }

        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited {
            community, channel, message_id, body: Some(body_text), ..
        }) => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.edit_message(message_id, format!("{body_text} (edited)"));
            }
        }

        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited { .. }) => {}

        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
            community, channel, message_id,
        }) => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.remove_message(message_id);
            }
        }

        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key, timestamp, sender_name, body, is_self,
        }) => {
            tracing::info!(
                peer = &peer_key[..12.min(peer_key.len())],
                is_self, has_body = body.is_some(),
                thread_exists = state.dm.threads.contains_key(peer_key),
                "tui: DirectMessageReceived"
            );
            if *is_self {
                tracing::info!(peer = &peer_key[..12.min(peer_key.len())], "tui: DM self echo");
                if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                    if let Some(ref body_text) = body {
                        let already_confirmed = thread.messages().iter().rev().any(|m| {
                            m.display.is_self
                                && m.delivery_status == DeliveryStatus::Sending
                                && m.display.body == *body_text
                                && m.display.timestamp.abs_diff(*timestamp) < 5000
                        });
                        if already_confirmed {
                            let _ = thread.confirm_by_body(body_text);
                        }
                    }
                }
            } else {
                let display_name = sender_name.clone()
                    .unwrap_or_else(|| helpers::abbreviate_key(peer_key));

                if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                    if let Some(ref body_text) = body {
                        if !thread.try_enrich_placeholder(*timestamp, body_text, Some(&display_name)) {
                            thread.push_message(DmMessage {
                                display: rekindle_types::display::DmMessageDisplay {
                                    sender_key: peer_key.clone(),
                                    sender_name: display_name.clone(),
                                    body: body_text.clone(),
                                    timestamp: *timestamp,
                                    is_self: false,
                                    sequence: 0,
                                },
                                delivery_status: DeliveryStatus::Confirmed,
                            });
                        }
                    } else {
                        thread.push_message(DmMessage {
                            display: rekindle_types::display::DmMessageDisplay {
                                sender_key: peer_key.clone(),
                                sender_name: display_name.clone(),
                                body: "(decrypting...)".into(),
                                timestamp: *timestamp,
                                is_self: false,
                                sequence: 0,
                            },
                            delivery_status: DeliveryStatus::Confirmed,
                        });
                    }
                    thread.last_message_at = Some(*timestamp);
                    thread.unread_count += 1;
                } else {
                    let mut new_thread = DmThreadState::new(peer_key.clone(), display_name.clone());
                    new_thread.last_message_at = Some(*timestamp);
                    new_thread.unread_count = 1;
                    if let Some(ref body_text) = body {
                        new_thread.push_message(DmMessage {
                            display: rekindle_types::display::DmMessageDisplay {
                                sender_key: peer_key.clone(),
                                sender_name: display_name,
                                body: body_text.clone(),
                                timestamp: *timestamp,
                                is_self: false,
                                sequence: 0,
                            },
                            delivery_status: DeliveryStatus::Confirmed,
                        });
                    }
                    state.dm.threads.insert(peer_key.clone(), new_thread);
                }
                state.dm.sort_by_last_message();

                // Load history for new or unloaded threads — the subscription
                // event delivered one message, but earlier messages may exist
                // in the vault from before the TUI connected.
                if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                    if !thread.loaded && !thread.loading {
                        thread.loading = true;
                        let (_, load_effect) = state.in_flight.track_request(
                            RequestKind::DmThread { peer_key: peer_key.clone() },
                            rekindle_types::daemon::DaemonRequest::Chat(
                                rekindle_types::daemon::ChatRequest::DmThread {
                                    peer_key: peer_key.clone(),
                                    limit: 50,
                                },
                            ),
                            state.now,
                        );
                        effects.push(load_effect);
                        tracing::info!(peer = &peer_key[..12.min(peer_key.len())], "tui: DM thread — loading history");
                    }
                }

                if state.session.dm_selected_peer.is_none() {
                    state.session.dm_selected_peer = Some(peer_key.clone());
                    if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                        if !thread.loaded && !thread.loading {
                            thread.loading = true;
                            let (_, effect) = state.in_flight.track_request(
                                RequestKind::DmThread { peer_key: peer_key.clone() },
                                rekindle_types::daemon::DaemonRequest::Chat(
                                    rekindle_types::daemon::ChatRequest::DmThread {
                                        peer_key: peer_key.clone(),
                                        limit: 50,
                                    },
                                ),
                                state.now,
                            );
                            effects.push(effect);
                        }
                    }
                }

                if state.nav.tab_bar.selected_id() != Some("dms") {
                    state.nav.tab_bar.increment_unread("dms");
                }
                state.ephemeral.toasts.push(
                    format!("New DM from {}", helpers::abbreviate_key(peer_key)),
                    ToastLevel::Info,
                    state.now,
                );
                if !state.nav.terminal_focused {
                    effects.push(Effect::OsNotify {
                        title: "New DM".into(),
                        body: format!("From {}", helpers::abbreviate_key(peer_key)),
                    });
                }
            }
        }

        SubscriptionEvent::Typing(TypingEvent::Started { context, who }) => {
            match context {
                TypingContext::Channel { community, channel } => {
                    let key = TypingKey::Channel {
                        community: community.clone(),
                        channel: channel.clone(),
                        pseudonym: who.clone(),
                    };
                    state.ephemeral.typing_indicators.insert(key, state.now);
                }
                TypingContext::Dm { peer_key } => {
                    if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                        thread.is_typing = true;
                        thread.typing_since = Some(state.now);
                    }
                }
            }
        }

        SubscriptionEvent::Typing(TypingEvent::Stopped { context, who }) => {
            match context {
                TypingContext::Channel { community, channel } => {
                    let key = TypingKey::Channel {
                        community: community.clone(),
                        channel: channel.clone(),
                        pseudonym: who.clone(),
                    };
                    state.ephemeral.typing_indicators.remove(&key);
                }
                TypingContext::Dm { peer_key } => {
                    if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                        thread.is_typing = false;
                        thread.typing_since = None;
                    }
                }
            }
        }

        SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community, pseudonym, status, ..
        }) => {
            if let Some(members) = state.communities.members.get_mut(community) {
                if let Some(m) = members.iter_mut().find(|m| m.member.pseudonym_key == *pseudonym) {
                    m.status = status.clone();
                }
            }
        }

        SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, status, ..
        }) => {
            if let Some(f) = state.friends.friends.iter_mut().find(|f| f.public_key == *peer_key) {
                f.status = status.clone();
            }
        }

        SubscriptionEvent::Friend(FriendEvent::RequestReceived {
            from_key, display_name, ..
        }) => {
            if !state.friends.pending_requests.iter().any(|r| r.public_key == *from_key) {
                state.friends.pending_requests.push(PendingFriendRequest {
                    public_key: from_key.clone(),
                    display_name: display_name.clone(),
                    received_at: state.wall_clock_ms,
                });
            }
            if state.nav.tab_bar.selected_id() != Some("friends") {
                state.nav.tab_bar.increment_unread("friends");
            }
            state.ephemeral.toasts.push(
                format!("Friend request from {display_name}"),
                ToastLevel::Info,
                state.now,
            );
            effects.push(Effect::OsNotify {
                title: "Friend Request".into(),
                body: format!("From {display_name}"),
            });
        }

        SubscriptionEvent::Friend(FriendEvent::Accepted { peer_key, .. }) => {
            state.friends.pending_requests.retain(|r| r.public_key != *peer_key);
            state.ephemeral.toasts.push("Friend request accepted".into(), ToastLevel::Success, state.now);
            let (_, effect) = state.in_flight.track_request(
                RequestKind::DmInbox,
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::DmInbox { limit: 50 },
                ),
                state.now,
            );
            effects.push(effect);
        }

        SubscriptionEvent::Friend(FriendEvent::Rejected { peer_key }) => {
            state.friends.pending_requests.retain(|r| r.public_key != *peer_key);
        }

        SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) => {
            state.friends.friends.retain(|f| f.public_key != *peer_key);
            state.friends.pending_requests.retain(|r| r.public_key != *peer_key);
            state.ephemeral.toasts.push("Friend removed".into(), ToastLevel::Info, state.now);
        }

        SubscriptionEvent::Friend(FriendEvent::RequestSent { target_profile_key, .. }) => {
            state.ephemeral.toasts.push(
                format!("Friend request sent to {}...", &target_profile_key[..12.min(target_profile_key.len())]),
                ToastLevel::Success,
                state.now,
            );
        }

        SubscriptionEvent::Friend(
            FriendEvent::RequestAcknowledged { .. }
            | FriendEvent::RemoveAcknowledged { .. }
            | FriendEvent::ProfileKeyRotated { .. }
        ) => {}

        SubscriptionEvent::Dm(DmLifecycleEvent::InviteReceived {
            record_key, sender_pseudonym, is_group, ..
        }) => {
            if !state.dm.threads.contains_key(record_key) {
                let mut thread = DmThreadState::new(
                    record_key.clone(),
                    format!("[invite] {sender_pseudonym}"),
                );
                thread.last_message_at = Some(state.wall_clock_ms);
                thread.unread_count = 1;
                thread.is_group = *is_group;
                state.dm.threads.insert(record_key.clone(), thread);
                state.dm.sort_by_last_message();
                if state.session.dm_selected_peer.is_none() {
                    state.session.dm_selected_peer = Some(record_key.clone());
                }
            }
        }

        SubscriptionEvent::Dm(DmLifecycleEvent::InviteDeclined { .. }) => {
            state.ephemeral.toasts.push("DM invite declined".into(), ToastLevel::Info, state.now);
        }

        SubscriptionEvent::Dm(DmLifecycleEvent::MemberLeft { .. }) => {
            state.ephemeral.toasts.push("DM member left".into(), ToastLevel::Info, state.now);
        }

        SubscriptionEvent::Membership(event) => {
            effects.extend(process_membership_event(event, state));
        }

        SubscriptionEvent::Governance(event) => {
            effects.extend(process_governance_event(event, state));
        }

        SubscriptionEvent::Social(event) => {
            process_social_event(event, state);
        }

        SubscriptionEvent::Voice(event) => {
            process_voice_event(event, state);
        }

        SubscriptionEvent::Crypto(event) => {
            process_crypto_event(event, state);
        }

        SubscriptionEvent::Network(event) => {
            process_network_event(event, state);
        }

        SubscriptionEvent::System(event) => {
            effects.extend(process_system_event(event, state));
        }

        SubscriptionEvent::UnreadChanged { context, count } => {
            match context {
                UnreadContext::Channel { community, channel } => {
                    let key = ChannelKey { community: community.clone(), channel: channel.clone() };
                    if let Some(ch) = state.channels.channels.get_mut(&key) {
                        ch.unread_count = *count;
                    }
                }
                UnreadContext::Dm { peer_key } => {
                    if let Some(thread) = state.dm.threads.get_mut(peer_key) {
                        thread.unread_count = *count;
                    }
                }
                UnreadContext::FriendRequests => {
                    *state.nav.tab_unreads.entry("friends".into()).or_insert(0) = *count;
                }
            }
        }

        SubscriptionEvent::BulkTransferProgress { .. } => {}
    }

    effects
}

fn process_membership_event(event: &MembershipEvent, state: &mut TuiState) -> Vec<Effect> {
    match event {
        MembershipEvent::Created { community, .. } => {
            state.ephemeral.toasts.push(format!("Community created: {community}"), ToastLevel::Success, state.now);
            vec![]
        }
        MembershipEvent::CommunityJoined { community, .. } => {
            state.ephemeral.toasts.push(format!("Joined community: {community}"), ToastLevel::Success, state.now);
            tracing::info!(community = &community[..12.min(community.len())], "tui: CommunityJoined — reloading list + subscribing");
            // Reload community list so the new community appears immediately
            let (_, reload_effect) = state.in_flight.track_request(
                RequestKind::CommunityList,
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::CommunityList,
                ),
                state.now,
            );
            // Subscribe to the new community's events
            let sub_effect = Effect::IpcSubscribe {
                filters: vec![rekindle_types::subscription_events::SubscriptionFilter::community(
                    community.clone(),
                )],
            };
            vec![reload_effect, sub_effect, Effect::Navigate(ViewKind::Onboarding { community: community.clone() })]
        }
        MembershipEvent::CommunityLeft { community, .. } => {
            state.communities.list.retain(|c| c.governance_key != *community);
            state.ephemeral.rails.clear_for_community(community);
            // Clear stale channel state, details, and members for the left community
            state.channels.channels.retain(|key, _| key.community != *community);
            state.communities.details.remove(community);
            state.communities.members.remove(community);
            state.communities.invites.remove(community);
            state.communities.events.remove(community);
            state.communities.bans.remove(community);
            state.communities.pending_members.remove(community);
            state.communities.onboarding.remove(community);
            tracing::info!(community = &community[..12.min(community.len())], "tui: CommunityLeft — cleared all community state");
            state.ephemeral.toasts.push(format!("Left community: {community}"), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::JoinRequested { display_name, .. } => {
            state.ephemeral.toasts.push(format!("{display_name} requested to join"), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::JoinAccepted { .. } => {
            state.ephemeral.toasts.push("Join request accepted".into(), ToastLevel::Success, state.now);
            vec![]
        }
        MembershipEvent::JoinRejected { reason, .. } => {
            state.ephemeral.toasts.push(format!("Join rejected: {reason}"), ToastLevel::Warning, state.now);
            vec![]
        }
        MembershipEvent::Joined { display_name, .. } => {
            state.ephemeral.toasts.push(format!("{display_name} joined"), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::Left { pseudonym, .. } => {
            state.ephemeral.toasts.push(format!("{} left", helpers::abbreviate_key(pseudonym)), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::Removed { pseudonym, .. } => {
            state.ephemeral.toasts.push(format!("{} removed", helpers::abbreviate_key(pseudonym)), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::Kicked { target_pseudonym, community } => {
            if state.communities.list.iter().any(|c| c.governance_key == *community) {
                state.communities.list.retain(|c| c.governance_key != *community);
                state.ephemeral.toasts.push(
                    format!("Kicked from {}", helpers::abbreviate_key(community)),
                    ToastLevel::Error,
                    state.now,
                );
                return vec![Effect::Navigate(ViewKind::Dashboard)];
            }
            state.ephemeral.toasts.push(
                format!("{} was kicked", helpers::abbreviate_key(target_pseudonym)),
                ToastLevel::Warning,
                state.now,
            );
            vec![]
        }
        MembershipEvent::Banned { target_pseudonym, .. } => {
            state.ephemeral.toasts.push(
                format!("{} was banned", helpers::abbreviate_key(target_pseudonym)),
                ToastLevel::Warning,
                state.now,
            );
            vec![]
        }
        MembershipEvent::Unbanned { target_pseudonym, .. } => {
            state.ephemeral.toasts.push(format!("{} unbanned", helpers::abbreviate_key(target_pseudonym)), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::TimedOut { target_pseudonym, duration_seconds, .. } => {
            state.ephemeral.toasts.push(
                format!("{} timed out for {duration_seconds}s", helpers::abbreviate_key(target_pseudonym)),
                ToastLevel::Warning,
                state.now,
            );
            vec![]
        }
        MembershipEvent::TimeoutRemoved { target_pseudonym, .. } => {
            state.ephemeral.toasts.push(format!("{} timeout removed", helpers::abbreviate_key(target_pseudonym)), ToastLevel::Info, state.now);
            vec![]
        }
        MembershipEvent::TimeoutStatusChanged { .. } => vec![],
        MembershipEvent::RolesChanged { .. } => vec![],
        MembershipEvent::OnboardingCompleted { .. } => vec![],
        MembershipEvent::OnboardingAnswersSubmitted { .. } => vec![],
    }
}

fn process_governance_event(event: &GovernanceEvent, state: &mut TuiState) -> Vec<Effect> {
    match event {
        GovernanceEvent::MetadataChanged { community }
        | GovernanceEvent::ChannelsChanged { community }
        | GovernanceEvent::RolesChanged { community } => {
            state.communities.details.remove(community);
            vec![]
        }
        GovernanceEvent::BansChanged { .. } => vec![],
        GovernanceEvent::InvitesChanged { .. } => vec![],
        GovernanceEvent::ChannelPermissionsChanged { .. } => vec![],
        GovernanceEvent::GovernanceSubkeyUpdated { .. } => vec![],
    }
}

fn process_social_event(event: &SocialEvent, state: &mut TuiState) {
    match event {
        SocialEvent::ReactionAdded { community, channel, message_id, emoji, .. } => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.add_reaction(message_id, emoji);
            }
        }
        SocialEvent::ReactionRemoved { community, channel, message_id, emoji, .. } => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.remove_reaction(message_id, emoji);
            }
        }
        SocialEvent::MessagePinned { community, channel, .. } => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.touch_generation();
            }
        }
        SocialEvent::MessageUnpinned { community, channel, .. } => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            if let Some(ch) = state.channels.channels.get_mut(&key) {
                ch.touch_generation();
            }
        }
        SocialEvent::ThreadCreated { thread_name, .. } => {
            state.ephemeral.toasts.push(format!("Thread created: {thread_name}"), ToastLevel::Info, state.now);
        }
        SocialEvent::ThreadMessagePosted { community, thread_id, message_id, sender_pseudonym, timestamp, body } => {
            tracing::info!(
                community = &community[..12.min(community.len())],
                thread_id = &thread_id[..12.min(thread_id.len())],
                has_body = body.is_some(),
                "tui: ThreadMessagePosted"
            );
            let display_body = body.as_deref().unwrap_or("(encrypted)").to_string();
            for (key, ch) in state.channels.channels.iter_mut() {
                if key.community != *community { continue; }
                if let Some(msgs) = ch.thread_messages.get_mut(thread_id) {
                    msgs.push(DecryptedMessageDisplay {
                        message_id: message_id.clone(),
                        sequence: 0,
                        author_pseudonym: sender_pseudonym.clone(),
                        author_display_name: sender_pseudonym.clone(),
                        body: display_body.clone(),
                        timestamp: *timestamp,
                        reply_to_sequence: None,
                        mek_generation: 0,
                        is_encrypted: body.is_none(),
                        needs_mek: if body.is_none() { Some(0) } else { None },
                        delivery_status: DeliveryStatus::Confirmed,
                        thread_id: Some(thread_id.clone()),
                    });
                }
            }
        }
        SocialEvent::ThreadArchiveChanged { .. } => {}
        SocialEvent::EventCreated { title, .. } => {
            state.ephemeral.toasts.push(format!("Event created: {title}"), ToastLevel::Info, state.now);
        }
        SocialEvent::EventUpdated { .. } => {}
        SocialEvent::EventDeleted { .. } => {}
        SocialEvent::EventRsvpChanged { .. } => {}
        SocialEvent::EventReminder { community, event_id, title, minutes_until_start, .. } => {
            state.ephemeral.toasts.push(
                format!("{title} starts in {minutes_until_start} min"),
                ToastLevel::Warning,
                state.now,
            );
            state.ephemeral.rails.set(RailSignal {
                id: format!("event:{community}:{event_id}"),
                scope: SignalScope::Community,
                text: format!("{title} \u{2014} starts in {minutes_until_start} min"),
                priority: if *minutes_until_start <= 5 { SignalPriority::Warning } else { SignalPriority::Info },
                dismissible: true,
            });
        }
        SocialEvent::GameServerAdded { .. } => {}
        SocialEvent::GameServerRemoved { .. } => {}
    }
}

fn process_voice_event(event: &VoiceEvent, state: &mut TuiState) {
    match event {
        VoiceEvent::Joined { pseudonym, channel, .. } => {
            state.ephemeral.toasts.push(
                format!("{} joined voice #{channel}", helpers::abbreviate_key(pseudonym)),
                ToastLevel::Info,
                state.now,
            );
        }
        VoiceEvent::Left { pseudonym, channel, .. } => {
            state.ephemeral.toasts.push(
                format!("{} left voice #{channel}", helpers::abbreviate_key(pseudonym)),
                ToastLevel::Info,
                state.now,
            );
        }
        VoiceEvent::ModeChanged { .. } => {}
        VoiceEvent::MuteChanged { .. } => {}
        VoiceEvent::DeafenChanged { .. } => {}
        VoiceEvent::RosterUpdated { .. } => {}
    }
}

fn process_crypto_event(event: &CryptoEvent, state: &mut TuiState) {
    match event {
        CryptoEvent::MekRotated { .. } => {}
        CryptoEvent::MekRequested { .. } => {}
        CryptoEvent::MekTransferred { .. } => {
            state.ephemeral.toasts.push("MEK transferred".into(), ToastLevel::Info, state.now);
        }
        CryptoEvent::AdminKeypairGranted { .. } => {
            state.ephemeral.toasts.push("Admin keypair granted".into(), ToastLevel::Success, state.now);
        }
        CryptoEvent::SlotKeypairGranted { .. } => {}
    }
}

fn process_network_event(event: &NetworkEvent, state: &mut TuiState) {
    match event {
        NetworkEvent::AttachmentChanged { is_attached, .. } => {
            state.node_connected = *is_attached;
        }
        NetworkEvent::LocalRoutesDied { .. } => {}
        NetworkEvent::RemoteRoutesDied { .. } => {}
        NetworkEvent::WatchRenewed { .. } => {}
        NetworkEvent::WatchReestablished { .. } => {}
        NetworkEvent::WatchFailed { record_key, .. } => {
            tracing::warn!(record_key, "watch failed");
        }
        NetworkEvent::ValueChanged { .. } => {}
    }
}

fn process_system_event(event: &SystemEvent, state: &mut TuiState) -> Vec<Effect> {
    match event {
        SystemEvent::IdentityCreated { public_key } => {
            state.ephemeral.toasts.push(
                format!("Identity created: {}...", &public_key[..12.min(public_key.len())]),
                ToastLevel::Success,
                state.now,
            );
            vec![]
        }
        SystemEvent::IdentityRotated { new_public_key } => {
            state.ephemeral.toasts.push(
                format!("Identity rotated: {}...", &new_public_key[..12.min(new_public_key.len())]),
                ToastLevel::Info,
                state.now,
            );
            vec![]
        }
        SystemEvent::Announcement { community, body, .. } => {
            let preview = body.char_indices().nth(80).map_or(body.as_str(), |(i, _)| &body[..i]);
            state.ephemeral.toasts.push(format!("Announcement: {preview}"), ToastLevel::Info, state.now);
            if let Some(ref gov) = community {
                state.ephemeral.rails.set(RailSignal {
                    id: format!("announce:{gov}"),
                    scope: SignalScope::Community,
                    text: preview.to_string(),
                    priority: SignalPriority::Info,
                    dismissible: true,
                });
            }
            vec![]
        }
        SystemEvent::RaidAlert { community, active } => {
            if *active {
                state.ephemeral.toasts.push("Raid alert activated!".into(), ToastLevel::Error, state.now);
                state.ephemeral.rails.set(RailSignal {
                    id: format!("raid:{community}"),
                    scope: SignalScope::Community,
                    text: format!("RAID ALERT \u{2014} {}", helpers::abbreviate_key(community)),
                    priority: SignalPriority::Critical,
                    dismissible: false,
                });
            } else {
                state.ephemeral.toasts.push("Raid alert cleared".into(), ToastLevel::Success, state.now);
                state.ephemeral.rails.remove(&format!("raid:{community}"));
            }
            vec![]
        }
        SystemEvent::ChannelLockdown { community, locked } => {
            if *locked {
                state.ephemeral.toasts.push("Channel locked down".into(), ToastLevel::Warning, state.now);
                state.ephemeral.rails.set(RailSignal {
                    id: format!("lockdown:{community}"),
                    scope: SignalScope::Channel,
                    text: "Channel locked \u{2014} non-operators cannot send".into(),
                    priority: SignalPriority::Warning,
                    dismissible: false,
                });
            } else {
                state.ephemeral.toasts.push("Lockdown lifted".into(), ToastLevel::Success, state.now);
                state.ephemeral.rails.remove(&format!("lockdown:{community}"));
            }
            vec![]
        }
        SystemEvent::Kicked { community } => {
            state.communities.list.retain(|c| c.governance_key != *community);
            state.ephemeral.toasts.push(
                format!("Kicked from {}", helpers::abbreviate_key(community)),
                ToastLevel::Error,
                state.now,
            );
            vec![Effect::Navigate(ViewKind::Dashboard)]
        }
        SystemEvent::BootstrapRequested { .. } => vec![],
        SystemEvent::BootstrapReceived { .. } => vec![],
        SystemEvent::SyncRequested { .. } => vec![],
        SystemEvent::SyncReceived { .. } => vec![],
    }
}

fn process_command_result(
    request_id: u64,
    response: &DaemonResponse,
    state: &mut TuiState,
) -> Vec<Effect> {
    let meta = state.in_flight.remove_request(request_id);
    if let Some(ref meta) = meta {
        tracing::debug!(request_id, kind = ?meta.kind, "tui: processing command result");
        state.in_flight.release_credit(&meta.kind);
    } else {
        tracing::warn!(request_id, "tui: command result for UNTRACKED request");
    }

    let bytes = match response {
        DaemonResponse::Ok(bytes) => {
            tracing::debug!(request_id, byte_len = bytes.len(), "tui: response OK");
            bytes
        }
        DaemonResponse::Error { code, message, .. } => {
            tracing::warn!(request_id, code, message, "tui: daemon error response");
            if let Some(ref meta) = meta {
                if matches!(meta.kind, RequestKind::Send) {
                    if let Some(pending) = state.in_flight.remove_pending_send(request_id) {
                        match pending {
                            PendingSend::Dm { peer_key, .. } => {
                                if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                                    thread.fail_last_sending();
                                }
                            }
                            PendingSend::ChannelMessage { community, channel, .. } => {
                                let key = ChannelKey { community, channel };
                                if let Some(ch) = state.channels.channels.get_mut(&key) {
                                    ch.fail_last_sending();
                                }
                            }
                        }
                    }
                }
            }
            state.ephemeral.toasts.push(
                format!("daemon ({code}): {message}"),
                ToastLevel::Error,
                state.now,
            );
            state.ephemeral.spinner.stop();
            return vec![];
        }
    };

    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            state.ephemeral.toasts.push(
                format!("response parse failed: {e}"),
                ToastLevel::Error,
                state.now,
            );
            return vec![];
        }
    };

    let kind = meta.as_ref().map(|m| &m.kind);
    let mut effects = vec![];

    match kind {
        Some(RequestKind::Status) => {
            if let Ok(snapshot) = serde_json::from_value::<dt::StatusSnapshot>(value) {
                state.node_connected = snapshot.is_attached;
                state.cached_peer_count = snapshot.peer_count;
                state.ephemeral.peer_history.push_back(snapshot.peer_count as f64);
                if state.ephemeral.peer_history.len() > 60 {
                    state.ephemeral.peer_history.pop_front();
                }
                state.ephemeral.spinner.stop();
                state.status_snapshot = Some(snapshot);
            }
        }

        Some(RequestKind::CommunityList) => {
            if let Ok(communities) = serde_json::from_value::<Vec<dt::CommunityOverview>>(value) {
                state.communities.list = communities.iter().map(CommunitySnapshot::from).collect();
                state.communities.list_loaded = true;
                state.communities.list_loaded_at = Some(state.now);
                state.ephemeral.community_history.push_back(state.communities.list.len() as f64);
                if state.ephemeral.community_history.len() > 60 {
                    state.ephemeral.community_history.pop_front();
                }

                // Subscribe to all joined communities for real-time events
                // (channel messages, typing, presence, governance, social).
                for community in &state.communities.list {
                    effects.push(Effect::IpcSubscribe {
                        filters: vec![rekindle_types::subscription_events::SubscriptionFilter::community(
                            community.governance_key.clone(),
                        )],
                    });
                }
                tracing::info!(
                    community_count = state.communities.list.len(),
                    "tui: CommunityList loaded — subscribed to all communities"
                );

                // Pre-load community details (channels, members) so command
                // palette shows channel names and the channel tree is populated
                // before the user navigates to a community.
                for community in &state.communities.list {
                    if !state.communities.details.contains_key(&community.governance_key) {
                        let (_, effect) = state.in_flight.track_request(
                            RequestKind::CommunityInfo { community: community.governance_key.clone() },
                            rekindle_types::daemon::DaemonRequest::Chat(
                                rekindle_types::daemon::ChatRequest::CommunityInfo {
                                    governance_key: community.governance_key.clone(),
                                },
                            ),
                            state.now,
                        );
                        effects.push(effect);
                    }
                }

                if let Some(restore) = state.session.pending_restore.take() {
                    if state.communities.list.iter().any(|c| c.governance_key == restore.community) {
                        let view = match restore.channel {
                            Some(ch) => ViewKind::ChannelWatch {
                                community: restore.community,
                                channel: ch,
                            },
                            None => ViewKind::CommunityInfo {
                                community: restore.community,
                            },
                        };
                        effects.push(Effect::Navigate(view));
                    }
                }
            }
        }

        Some(RequestKind::Identity) => {
            let public_key = value.get("public_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let display_name = value.get("display_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !public_key.is_empty() {
                state.ephemeral.identity = Some(IdentitySnapshot {
                    public_key,
                    display_name,
                });
            }
        }

        Some(RequestKind::FriendList) => {
            if let Ok(friends) = serde_json::from_value::<Vec<dt::FriendDisplay>>(value) {
                state.friends.friends = friends;
                state.friends.friends.sort_by(|a, b| {
                    presence_rank(&a.status).cmp(&presence_rank(&b.status))
                        .then(a.display_name.cmp(&b.display_name))
                });
                state.friends.loaded = true;
                state.friends.loaded_at = Some(state.now);
            }
        }

        Some(RequestKind::NetworkPeers) => {
            if let Ok(peers) = serde_json::from_value::<Vec<dt::PeerSnapshot>>(value) {
                state.communities.peer_list = Some(peers);
            }
        }

        Some(RequestKind::DmInbox) => {
            tracing::info!("tui: DmInbox response processing");
            match serde_json::from_value::<Vec<dt::DmThreadDisplay>>(value) {
                Ok(threads) => {
                    tracing::info!(thread_count = threads.len(), "tui: DmInbox deserialized OK");
                    for thread in &threads {
                        state.dm.threads
                            .entry(thread.peer_key.clone())
                            .and_modify(|existing| {
                                existing.peer_name = thread.peer_name.clone();
                                existing.last_message_at = thread.last_message_at;
                                existing.is_group = thread.is_group;
                            })
                            .or_insert_with(|| {
                                let mut t = DmThreadState::new(thread.peer_key.clone(), thread.peer_name.clone());
                                t.last_message_at = thread.last_message_at;
                                t.unread_count = thread.unread_count;
                                t.is_group = thread.is_group;
                                t
                            });
                    }
                    state.dm.sort_by_last_message();
                    state.dm.inbox_loaded = true;
                    state.dm.inbox_loaded_at = Some(state.now);

                    if let Some((peer_key, thread)) = state.dm.threads.iter_mut().next() {
                        tracing::info!(
                            peer = &peer_key[..12.min(peer_key.len())],
                            dm_selected_peer = ?state.session.dm_selected_peer.as_deref().map(|k| &k[..12.min(k.len())]),
                            loaded = thread.loaded,
                            loading = thread.loading,
                            "tui: DmInbox auto-select check"
                        );
                        if state.session.dm_selected_peer.is_none() {
                            state.session.dm_selected_peer = Some(peer_key.clone());
                            tracing::info!(peer = &peer_key[..12.min(peer_key.len())], "tui: DmInbox auto-selected peer");
                        }
                        if !thread.loaded && !thread.loading {
                            thread.loading = true;
                            let (req_id, effect) = state.in_flight.track_request(
                                RequestKind::DmThread { peer_key: peer_key.clone() },
                                rekindle_types::daemon::DaemonRequest::Chat(
                                    rekindle_types::daemon::ChatRequest::DmThread {
                                        peer_key: peer_key.clone(),
                                        limit: 50,
                                    },
                                ),
                                state.now,
                            );
                            tracing::info!(req_id, peer = &peer_key[..12.min(peer_key.len())], "tui: DmInbox auto-load DmThread request queued");
                            effects.push(effect);
                        } else {
                            tracing::info!(
                                loaded = thread.loaded, loading = thread.loading,
                                "tui: DmInbox auto-load SKIPPED (already loaded or loading)"
                            );
                        }
                    } else {
                        tracing::info!("tui: DmInbox no threads to auto-select");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "DM inbox deserialize failed");
                }
            }
        }

        Some(RequestKind::DmThread { peer_key }) => {
            let peer_key = peer_key.clone();
            tracing::info!(peer = &peer_key[..12.min(peer_key.len())], "tui: DmThread response processing");
            match serde_json::from_value::<Vec<dt::DmMessageDisplay>>(value) {
                Ok(messages) => {
                    tracing::info!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        message_count = messages.len(),
                        thread_exists = state.dm.threads.contains_key(&peer_key),
                        "tui: DmThread deserialized OK"
                    );
                    if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                        thread.set_messages(messages.iter().map(|m| DmMessage {
                            display: m.clone(),
                            delivery_status: DeliveryStatus::Confirmed,
                        }).collect());
                        thread.loaded = true;
                        thread.loading = false;
                        tracing::info!(
                            peer = &peer_key[..12.min(peer_key.len())],
                            final_count = thread.messages().len(),
                            generation = thread.generation(),
                            "tui: DmThread messages set, thread.loaded=true"
                        );
                    } else {
                        tracing::warn!(peer = &peer_key[..12.min(peer_key.len())], "tui: DmThread response but thread not in state.dm.threads");
                    }
                    if state.session.dm_selected_peer.is_none() {
                        state.session.dm_selected_peer = Some(peer_key.clone());
                        tracing::info!(peer = &peer_key[..12.min(peer_key.len())], "tui: DmThread auto-selected peer");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, peer = &peer_key[..12.min(peer_key.len())], "tui: DmThread deserialize FAILED");
                    if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                        thread.loading = false;
                    }
                }
            }
        }

        Some(RequestKind::ChannelHistory { community, channel }) => {
            let community = community.clone();
            let channel = channel.clone();
            tracing::info!(community = &community[..12.min(community.len())], channel = &channel, "tui: ChannelHistory response processing");
            match serde_json::from_value::<Vec<dt::DecryptedMessageDisplay>>(value) {
                Ok(messages) => {
                    tracing::info!(
                        community = &community[..12.min(community.len())],
                        channel = &channel,
                        message_count = messages.len(),
                        "tui: ChannelHistory deserialized OK"
                    );
                    let key = ChannelKey { community, channel };
                    let ch = state.channels.channels.entry(key).or_insert_with(ChannelViewState::new);
                    ch.set_messages(VecDeque::from(messages));
                    ch.loaded = true;
                    ch.loading = false;
                    ch.loaded_at = Some(state.now);
                }
                Err(e) => {
                    tracing::warn!(error = %e, community = &community[..12.min(community.len())], channel = &channel, "tui: ChannelHistory deserialize FAILED");
                }
            }
        }

        Some(RequestKind::CommunityInfo { .. }) => {
            if let Ok(detail) = serde_json::from_value::<dt::CommunityDetail>(value) {
                state.communities.members.insert(
                    detail.governance_key.clone(),
                    detail.members.clone(),
                );
                state.communities.details.insert(detail.governance_key.clone(), detail);
            }
        }

        Some(RequestKind::Send) => {
            let message_id = value.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            tracing::info!(request_id, message_id = &message_id[..12.min(message_id.len())], "tui: Send confirmed");
            if let Some(pending) = state.in_flight.remove_pending_send(request_id) {
                match pending {
                    PendingSend::Dm { peer_key, body } => {
                        if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                            let _ = thread.confirm_by_body(&body);
                        }
                    }
                    PendingSend::ChannelMessage { community, channel, .. } => {
                        let key = ChannelKey { community, channel };
                        if let Some(ch) = state.channels.channels.get_mut(&key) {
                            let _ = ch.confirm_last_sending(&message_id);
                        }
                    }
                }
            }
            state.ephemeral.spinner.stop();
        }

        Some(RequestKind::Invites { .. }) => {
            if let Some(key) = current_community_key(state) {
                let invites = value.as_array().cloned().unwrap_or_default();
                state.communities.invites.insert(key, invites);
            }
        }

        Some(RequestKind::Events { .. }) => {
            if let Some(key) = current_community_key(state) {
                let events = value.as_array().cloned().unwrap_or_default();
                state.communities.events.insert(key, events);
            }
        }

        Some(RequestKind::Onboarding { .. }) => {
            if let Some(key) = current_community_key(state) {
                let config = serde_json::from_value(value.clone()).ok();
                let welcome = value.get("welcome").and_then(|v| serde_json::from_value(v.clone()).ok());
                state.communities.onboarding.insert(key, (config, welcome));
            }
        }

        Some(RequestKind::Pins { .. }) => {
            if let Some(ch_key) = current_channel_key(state) {
                if let Some(ch) = state.channels.channels.get_mut(&ch_key) {
                    let pins = value.as_array().map(|arr| {
                        arr.iter().filter_map(|p| {
                            Some(PinDisplay {
                                message_id: p.get("messageId").or_else(|| p.get("message_id")).and_then(|v| v.as_str())?.to_string(),
                                channel_id: p.get("channelId").or_else(|| p.get("channel_id")).and_then(|v| v.as_str())?.to_string(),
                                pinned_by: p.get("pinnedBy").or_else(|| p.get("pinned_by")).and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                                pinned_at: p.get("pinnedAt").or_else(|| p.get("pinned_at")).and_then(|v| v.as_u64()).unwrap_or(0),
                                body_preview: "(pinned message)".to_string(),
                            })
                        }).collect()
                    }).unwrap_or_default();
                    ch.set_pins(pins);
                }
            }
        }

        Some(RequestKind::ThreadMessages { community, thread_id }) => {
            let community = community.clone();
            let thread_id = thread_id.clone();
            if let Ok(messages) = serde_json::from_value::<Vec<dt::DecryptedMessageDisplay>>(value) {
                for (key, ch) in state.channels.channels.iter_mut() {
                    if !community.is_empty() && key.community != community {
                        continue;
                    }
                    ch.thread_messages.insert(thread_id.clone(), messages.clone());
                }
            }
        }

        Some(RequestKind::BanList { .. }) => {
            if let Some(key) = current_community_key(state) {
                let bans = value.as_array().cloned().unwrap_or_default();
                state.communities.bans.insert(key, bans);
            }
        }

        Some(RequestKind::PendingMembers { .. }) => {
            if let Some(key) = current_community_key(state) {
                let pending = value.as_array().cloned().unwrap_or_default();
                state.communities.pending_members.insert(key, pending);
            }
        }

        Some(RequestKind::MarkRead) | Some(RequestKind::Subscribe)
        | Some(RequestKind::Typing) | Some(RequestKind::Other) => {}

        None => {
            tracing::warn!(request_id, "response for unknown request");
        }
    }

    effects
}

fn process_command_failed(
    request_id: u64,
    error: &str,
    state: &mut TuiState,
) -> Vec<Effect> {
    let mut is_typing = false;
    if let Some(meta) = state.in_flight.remove_request(request_id) {
        is_typing = matches!(meta.kind, RequestKind::Typing);
        state.in_flight.release_credit(&meta.kind);
        if matches!(meta.kind, RequestKind::Send) {
            if let Some(pending) = state.in_flight.remove_pending_send(request_id) {
                match pending {
                    PendingSend::Dm { peer_key, .. } => {
                        if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                            thread.fail_last_sending();
                        }
                    }
                    PendingSend::ChannelMessage { community, channel, .. } => {
                        let key = ChannelKey { community, channel };
                        if let Some(ch) = state.channels.channels.get_mut(&key) {
                            ch.fail_last_sending();
                        }
                    }
                }
            }
        }
    }
    if is_typing {
        tracing::debug!(request_id, error, "tui: typing indicator failed (suppressed)");
    } else {
        state.ephemeral.toasts.push(
            format!("Request failed: {error}"),
            ToastLevel::Error,
            state.now,
        );
    }
    state.ephemeral.spinner.stop();
    vec![]
}

fn current_channel_key(state: &TuiState) -> Option<ChannelKey> {
    match state.nav.current_view() {
        ViewKind::ChannelWatch { community, channel } => {
            Some(ChannelKey { community: community.clone(), channel: channel.clone() })
        }
        _ => None,
    }
}

fn current_community_key(state: &TuiState) -> Option<String> {
    match state.nav.current_view() {
        ViewKind::ChannelWatch { community, .. }
        | ViewKind::CommunityInfo { community }
        | ViewKind::Moderation { community }
        | ViewKind::Invite { community }
        | ViewKind::Events { community }
        | ViewKind::Onboarding { community }
        | ViewKind::VoiceSession { community, .. } => Some(community.clone()),
        _ => None,
    }
}
