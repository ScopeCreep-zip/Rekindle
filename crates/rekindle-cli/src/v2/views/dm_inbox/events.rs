//! DM inbox event handling.

use super::DmInboxView;
use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use rekindle_types::display::DmMessageDisplay;
use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent, TypingEvent, TypingContext};

pub fn handle_command_result(view: &mut DmInboxView, result: CommandResult) -> Option<Action> {
    match result {
        CommandResult::DmInboxLoaded { threads } => {
            view.threads = threads;
            view.threads.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
            if !view.threads.is_empty() && view.thread_list_state.selected().is_none() {
                view.thread_list_state.select(Some(0));
            }
            view.loaded = true;
            // Auto-load first selected thread's messages
            let thread = view.thread_list_state.selected()
                .and_then(|i| view.threads.get(i))?;
            if thread.messages.is_empty()
                && view.loading_peer.as_deref() != Some(&thread.peer_key)
            {
                view.loading_peer = Some(thread.peer_key.clone());
                return Some(Action::LoadDmThread { peer_key: thread.peer_key.clone() });
            }
            None
        }
        CommandResult::DmThreadLoaded { ref peer_key, ref messages } => {
            tracing::debug!(peer_key = &peer_key[..12.min(peer_key.len())], msg_count = messages.len(), "TUI: dm thread loaded into inbox");
            view.loading_peer = None;
            if let Some(thread) = view.threads.iter_mut().find(|t| t.peer_key == *peer_key) {
                thread.messages = messages.clone();
            }
            None
        }
        _ => None,
    }
}

pub fn handle_subscription_event(view: &mut DmInboxView, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key, timestamp, sender_name, body, is_self,
        }) => {
            // Enrich existing placeholder
            if let Some(ref body_text) = body {
                if let Some(thread) = view.threads.iter_mut().find(|t| t.peer_key == *peer_key) {
                    if let Some(existing) = thread.messages.iter_mut().rev().find(|m| {
                        m.body == "(decrypting...)" && m.timestamp.abs_diff(*timestamp) < 5000
                    }) {
                        existing.body.clone_from(body_text);
                        if let Some(ref name) = sender_name {
                            existing.sender_name.clone_from(name);
                        }
                        return;
                    }
                }
            }

            let display_msg = DmMessageDisplay {
                sender_key: peer_key.clone(),
                sender_name: sender_name.clone().unwrap_or_else(|| helpers::abbreviate_key(peer_key)),
                body: body.clone().unwrap_or_else(|| "(decrypting...)".into()),
                timestamp: *timestamp,
                is_self: *is_self,
                sequence: 0,
            };

            if let Some(thread) = view.threads.iter_mut().find(|t| t.peer_key == *peer_key) {
                thread.messages.push(display_msg);
                thread.last_message_at = Some(*timestamp);
                if !*is_self { thread.unread_count += 1; }
            } else {
                let thread_name = if *is_self {
                    helpers::abbreviate_key(peer_key)
                } else {
                    sender_name.clone().unwrap_or_else(|| helpers::abbreviate_key(peer_key))
                };
                view.threads.push(rekindle_types::display::DmThreadDisplay {
                    peer_key: peer_key.clone(), peer_name: thread_name,
                    last_message_at: Some(*timestamp), unread_count: u32::from(!*is_self),
                    is_group: false,
                    messages: vec![display_msg],
                });
            }
            view.threads.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
        }
        SubscriptionEvent::Typing(TypingEvent::Started { context: TypingContext::Dm { peer_key }, .. }) => {
            view.typing_peers.insert(peer_key.clone());
        }
        SubscriptionEvent::Typing(TypingEvent::Stopped { context: TypingContext::Dm { peer_key }, .. }) => {
            view.typing_peers.remove(peer_key.as_str());
        }
        SubscriptionEvent::Dm(rekindle_types::subscription_events::DmLifecycleEvent::InviteReceived {
            record_key, sender_pseudonym, is_group, ..
        }) => {
            if !view.threads.iter().any(|t| t.peer_key == *record_key) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                view.threads.push(rekindle_types::display::DmThreadDisplay {
                    peer_key: record_key.clone(),
                    peer_name: format!("[invite] {sender_pseudonym}"),
                    last_message_at: Some(now_ms),
                    unread_count: 1,
                    is_group: *is_group,
                    messages: Vec::new(),
                });
                view.threads.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
            }
        }
        _ => {}
    }
}
