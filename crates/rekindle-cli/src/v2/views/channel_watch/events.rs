//! Channel watch event handling — command results and subscription events.

use super::state::ChannelWatchView;
use crate::v2::helpers;
use crate::v2::tui::action::CommandResult;
use crate::v2::tui::components::channel_tree::{ChannelEntry, TreeNodeId};
use crate::v2::tui::components::peer_list::PeerEntry;
use rekindle_types::display::DecryptedMessageDisplay;
use rekindle_types::subscription_events::{
    SubscriptionEvent, ChannelMessageEvent, TypingEvent, TypingContext,
    PresenceEvent, UnreadContext, SocialEvent,
};

pub fn handle_command_result(view: &mut ChannelWatchView, result: CommandResult) {
    match result {
        CommandResult::ChannelHistoryLoaded { community, channel, messages } => {
            if community == view.community && channel == view.channel {
                let missing: std::collections::HashSet<u64> = messages.iter().filter_map(|m| m.needs_mek).collect();
                for gen in &missing {
                    tracing::info!(community = view.community.as_str(), channel = view.channel.as_str(), generation = gen, "auto-requesting missing MEK");
                }
                view.pending_mek_requests = missing;
                let count = messages.len();
                view.message_list.set_messages(messages);
                if count > 0 { view.message_list.set_last_read(count.saturating_sub(1)); }
            }
        }
        CommandResult::MessageSent { message_id } => {
            view.message_list.confirm_message(&message_id);
        }
        CommandResult::SendFailed => {
            view.message_list.fail_pending_message();
        }
        CommandResult::ThreadMessagesLoaded { thread_id, messages } => {
            if view.thread_panel.visible && view.thread_panel.thread_id == thread_id {
                view.thread_panel.message_list.set_messages(messages);
            }
        }
        CommandResult::PinsLoaded { mut pins } => {
            // Enrich pin previews from cached message list
            for pin in &mut pins {
                if pin.body_preview == "(pinned message)" {
                    for i in 0..view.message_list.len() {
                        if let Some(msg) = view.message_list.message_at(i) {
                            if msg.message_id == pin.message_id {
                                let body = &msg.body;
                                pin.body_preview = if body.len() > 80 {
                                    format!("{}...", &body[..77])
                                } else {
                                    body.clone()
                                };
                                break;
                            }
                        }
                    }
                }
            }
            view.pins_panel.set_pins(pins);
        }
        CommandResult::CommunityInfoLoaded { detail } => {
            if detail.governance_key == view.community {
                if view.channel_id.is_none() {
                    if let Some(ch) = detail.channels.iter().find(|c| c.name == view.channel || c.id == view.channel) {
                        view.channel_id = Some(ch.id.clone());
                    }
                }
                let channels: Vec<ChannelEntry> = detail.channels.iter().map(|ch| {
                    let unread = detail.channel_unreads.get(&ch.id).copied().unwrap_or(0);
                    ChannelEntry {
                        id: ch.id.clone(), name: ch.name.clone(), kind: format!("{:?}", ch.kind),
                        category: ch.category_id.clone(), unread, sort_order: ch.sort_order,
                    }
                }).collect();
                view.channel_tree.expand(&TreeNodeId::Community(detail.governance_key.clone()));
                view.channel_tree.set_communities(&[(detail.governance_key.clone(), detail.name.clone(), channels)], &[]);

                let members: Vec<PeerEntry> = detail.members.iter().map(|m| PeerEntry {
                    key: m.member.pseudonym_key.clone(),
                    display_name: m.member.display_name.clone(),
                    status: m.status.clone(),
                    role: m.role_name.clone(),
                }).collect();
                view.peer_list.set_members(members);
            }
        }
        CommandResult::DmThreadLoaded { ref peer_key, ref messages } => {
            if view.split_dm.is_peer(peer_key) {
                if let Some(ref mut ml) = view.split_dm_message_list {
                    let display_msgs: Vec<DecryptedMessageDisplay> = messages.iter().enumerate().map(|(i, m)| {
                        DecryptedMessageDisplay {
                            message_id: format!("dm-{}-{i}", m.timestamp),
                            sequence: i as u64,
                            author_pseudonym: m.sender_key.clone(),
                            author_display_name: m.sender_name.clone(),
                            body: m.body.clone(),
                            timestamp: m.timestamp,
                            reply_to_sequence: None, mek_generation: 0,
                            is_encrypted: false, needs_mek: None,
                            delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                            thread_id: None,
                        }
                    }).collect();
                    ml.set_messages(display_msgs);
                }
            }
        }
        _ => {}
    }
}

pub fn handle_subscription_event(view: &mut ChannelWatchView, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community, channel, message_id, sender_pseudonym,
            sequence, timestamp, body, reply_to_sequence, is_self, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            if *is_self {
                view.message_list.confirm_message(message_id);
            } else if let Some(ref body_text) = body {
                if !view.message_list.try_enrich_placeholder(sender_pseudonym, *timestamp, message_id, body_text, *sequence, *reply_to_sequence) {
                    let display_name = view.peer_list.resolve_name(sender_pseudonym)
                        .unwrap_or_else(|| helpers::abbreviate_key(sender_pseudonym));
                    view.message_list.push(DecryptedMessageDisplay {
                        message_id: message_id.clone(), sequence: *sequence,
                        author_pseudonym: sender_pseudonym.clone(), author_display_name: display_name,
                        body: body_text.clone(), timestamp: *timestamp, reply_to_sequence: *reply_to_sequence,
                        mek_generation: 0, is_encrypted: false, needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                        thread_id: None,
                    });
                }
            } else {
                let display_name = view.peer_list.resolve_name(sender_pseudonym)
                    .unwrap_or_else(|| helpers::abbreviate_key(sender_pseudonym));
                view.message_list.push(DecryptedMessageDisplay {
                    message_id: message_id.clone(), sequence: *sequence,
                    author_pseudonym: sender_pseudonym.clone(), author_display_name: display_name,
                    body: "(decrypting...)".into(), timestamp: *timestamp, reply_to_sequence: *reply_to_sequence,
                    mek_generation: 0, is_encrypted: true, needs_mek: Some(0),
                    delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    thread_id: None,
                });
            }
        }
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Edited {
            community, channel, message_id, body: Some(ref body_text), ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.update_body(message_id, body_text);
        }
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
            community, channel, message_id,
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.remove_by_id(message_id);
        }
        SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Channel { community, channel }, who,
        }) if *community == view.community && view.channel_matches(channel) => {
            view.typing_indicators.insert(who.clone(), std::time::Instant::now());
        }
        SubscriptionEvent::Typing(TypingEvent::Stopped {
            context: TypingContext::Channel { community, channel }, who,
        }) if *community == view.community && view.channel_matches(channel) => {
            view.typing_indicators.remove(who);
        }
        SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community, pseudonym, status, ..
        }) if *community == view.community => {
            view.peer_list.update_member_status(pseudonym, status);
        }
        SubscriptionEvent::UnreadChanged {
            context: UnreadContext::Channel { community, channel }, count,
        } => {
            view.channel_tree.set_channel_unread(community, channel, *count);
        }
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key, timestamp, sender_name, body, is_self,
        }) if view.split_dm.is_peer(peer_key) => {
            if let Some(ref mut ml) = view.split_dm_message_list {
                if !*is_self {
                    let display_name = sender_name.clone().unwrap_or_else(|| helpers::abbreviate_key(peer_key));
                    ml.push(DecryptedMessageDisplay {
                        message_id: format!("dm-{timestamp}"), sequence: 0,
                        author_pseudonym: peer_key.clone(), author_display_name: display_name,
                        body: body.clone().unwrap_or_else(|| "(decrypting...)".into()),
                        timestamp: *timestamp, reply_to_sequence: None, mek_generation: 0,
                        is_encrypted: body.is_none(), needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                        thread_id: None,
                    });
                }
            }
        }
        SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Dm { peer_key }, ..
        }) if view.split_dm.is_peer(peer_key) => {
            view.split_dm.peer_typing = true;
        }
        SubscriptionEvent::Typing(TypingEvent::Stopped {
            context: TypingContext::Dm { peer_key }, ..
        }) if view.split_dm.is_peer(peer_key) => {
            view.split_dm.peer_typing = false;
        }
        SubscriptionEvent::Social(SocialEvent::ReactionAdded {
            community, channel, ref message_id, ref emoji, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.add_reaction(message_id, emoji);
        }
        SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
            community, channel, ref message_id, ref emoji, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.remove_reaction(message_id, emoji);
        }
        SubscriptionEvent::Social(SocialEvent::MessagePinned {
            community, channel, ref message_id, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.set_pinned(message_id, true);
        }
        SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
            community, channel, ref message_id, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.set_pinned(message_id, false);
        }
        SubscriptionEvent::Social(SocialEvent::ThreadCreated {
            community, channel, ref thread_id, ..
        }) if *community == view.community && view.channel_matches(channel) => {
            view.message_list.set_thread(thread_id, thread_id);
        }
        SubscriptionEvent::Social(SocialEvent::ThreadMessagePosted {
            community, ref thread_id, ref message_id, ref sender_pseudonym, timestamp, ..
        }) if *community == view.community => {
            view.message_list.increment_thread_replies(thread_id);
            // If thread panel is open for this thread, push the message
            if view.thread_panel.visible && view.thread_panel.thread_id == *thread_id {
                let display_name = view.peer_list.resolve_name(sender_pseudonym)
                    .unwrap_or_else(|| crate::v2::helpers::abbreviate_key(sender_pseudonym));
                view.thread_panel.message_list.push(DecryptedMessageDisplay {
                    message_id: message_id.clone(), sequence: 0,
                    author_pseudonym: sender_pseudonym.clone(), author_display_name: display_name,
                    body: "(thread message — decrypting...)".into(), timestamp: *timestamp,
                    reply_to_sequence: None, mek_generation: 0, is_encrypted: true, needs_mek: None,
                    delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    thread_id: Some(thread_id.clone()),
                });
            }
        }
        SubscriptionEvent::BulkTransferProgress {
            ref transfer_id, bytes_transferred, total_size, ref status, ..
        } => {
            match status.as_str() {
                "completed" => view.transfer_rail.complete(transfer_id),
                "failed" => view.transfer_rail.fail(transfer_id, "transfer failed"),
                "cancelled" => {
                    if let Some(t) = view.transfer_rail.transfers.iter_mut().find(|t| t.transfer_id == *transfer_id) {
                        t.status = crate::v2::tui::components::transfer_rail::TransferStatus::Cancelled;
                    }
                }
                _ => view.transfer_rail.update(transfer_id, *bytes_transferred, *total_size),
            }
        }
        _ => {}
    }
}
