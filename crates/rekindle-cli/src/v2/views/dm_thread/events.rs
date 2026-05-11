//! DM thread event handling.

use super::DmThreadView;
use crate::v2::helpers;
use crate::v2::tui::action::CommandResult;
use rekindle_types::display::DecryptedMessageDisplay;
use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent, TypingEvent, TypingContext};

#[allow(clippy::needless_pass_by_value)] // CommandResult variants are destructured and fields moved
pub fn handle_command_result(view: &mut DmThreadView, result: CommandResult) {
    match result {
        CommandResult::SendFailed => { view.message_list.fail_pending_message(); }
        CommandResult::MessageSent { ref message_id } => { view.message_list.confirm_message(message_id); }
        CommandResult::DmThreadLoaded { ref peer_key, ref messages } => {
            if *peer_key == view.peer_key {
                #[allow(clippy::cast_possible_truncation)]
                let display_msgs: Vec<DecryptedMessageDisplay> = messages.iter().enumerate().map(|(i, m)| {
                    DecryptedMessageDisplay {
                        message_id: format!("dm-{}-{i}", m.timestamp), sequence: i as u64,
                        author_pseudonym: m.sender_key.clone(), author_display_name: m.sender_name.clone(),
                        body: m.body.clone(), timestamp: m.timestamp, reply_to_sequence: None,
                        mek_generation: 0, is_encrypted: false, needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    }
                }).collect();
                let count = display_msgs.len();
                view.message_list.set_messages(display_msgs);
                if count > 0 { view.message_list.set_last_read(count.saturating_sub(1)); }
                if let Some(msg) = messages.iter().find(|m| !m.is_self) {
                    view.peer_name.clone_from(&msg.sender_name);
                }
                view.loaded = true;
            }
        }
        _ => {}
    }
}

pub fn handle_subscription_event(view: &mut DmThreadView, event: &SubscriptionEvent) {
    match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key, timestamp, sender_name, body, is_self,
        }) if *peer_key == view.peer_key && !*is_self => {
            if let Some(ref body_text) = body {
                let msg_id = format!("dm-{timestamp}");
                if !view.message_list.try_enrich_placeholder(peer_key, *timestamp, &msg_id, body_text, 0, None) {
                    let display_name = sender_name.clone().unwrap_or_else(|| helpers::abbreviate_key(peer_key));
                    view.message_list.push(DecryptedMessageDisplay {
                        message_id: msg_id, sequence: 0, author_pseudonym: peer_key.clone(),
                        author_display_name: display_name, body: body_text.clone(), timestamp: *timestamp,
                        reply_to_sequence: None, mek_generation: 0, is_encrypted: false, needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    });
                }
            } else {
                let display_name = sender_name.clone().unwrap_or_else(|| helpers::abbreviate_key(peer_key));
                view.message_list.push(DecryptedMessageDisplay {
                    message_id: format!("dm-{timestamp}"), sequence: 0, author_pseudonym: peer_key.clone(),
                    author_display_name: display_name, body: "(decrypting...)".into(), timestamp: *timestamp,
                    reply_to_sequence: None, mek_generation: 0, is_encrypted: true, needs_mek: None,
                    delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                });
            }
        }
        SubscriptionEvent::Typing(TypingEvent::Started { context: TypingContext::Dm { peer_key }, .. })
            if *peer_key == view.peer_key => { view.is_peer_typing = true; }
        SubscriptionEvent::Typing(TypingEvent::Stopped { context: TypingContext::Dm { peer_key }, .. })
            if *peer_key == view.peer_key => { view.is_peer_typing = false; }
        _ => {}
    }
}
