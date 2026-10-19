//! `ChannelMessageEvent` samples.

use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::{channel, community, id, pk, rk, text, Sample};

pub const NAMES: &[&str] = &[
    "new",
    "edited",
    "deleted",
    "directMessageReceived",
    "directMessageAcknowledged",
    "directConversationInvited",
    "conversationFocusRequested",
];

pub fn variant_name(e: &ChannelMessageEvent) -> &'static str {
    match e {
        ChannelMessageEvent::New { .. } => "new",
        ChannelMessageEvent::Edited { .. } => "edited",
        ChannelMessageEvent::Deleted { .. } => "deleted",
        ChannelMessageEvent::DirectMessageReceived { .. } => "directMessageReceived",
        ChannelMessageEvent::DirectMessageAcknowledged { .. } => "directMessageAcknowledged",
        ChannelMessageEvent::DirectConversationInvited { .. } => "directConversationInvited",
        ChannelMessageEvent::ConversationFocusRequested { .. } => "conversationFocusRequested",
    }
}

fn ev(name: &str, e: ChannelMessageEvent) -> Sample {
    (
        format!("channelMessage.{name}"),
        SubscriptionEvent::ChannelMessage(e),
    )
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "new",
            ChannelMessageEvent::New {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                sender_pseudonym: pk('a'),
                sequence: 42,
                timestamp: 1_760_000_000_000,
                body: Some(text("hello channel")),
                reply_to_sequence: Some(41),
            },
        ),
        ev(
            "edited",
            ChannelMessageEvent::Edited {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                edited_at: 1_760_000_001_000,
                body: Some(text("hello channel, edited")),
            },
        ),
        ev(
            "deleted",
            ChannelMessageEvent::Deleted {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
            },
        ),
        ev(
            "directMessageReceived",
            ChannelMessageEvent::DirectMessageReceived {
                peer_key: pk('b'),
                timestamp: 1_760_000_002_000,
                sender_name: Some(text("Bob")),
                body: Some(text("hi there")),
                decryption_failed: false,
                automod_blurred: true,
                conversation_id: pk('b'),
                server_message_id: Some(id("dm_", '2')),
                reply_to_id: Some(id("dm_", '3')),
                is_own: false,
            },
        ),
        ev(
            "directMessageAcknowledged",
            ChannelMessageEvent::DirectMessageAcknowledged {
                message_id: 1_760_000_002_500,
                conversation_id: pk('b'),
            },
        ),
        ev(
            "directConversationInvited",
            ChannelMessageEvent::DirectConversationInvited {
                from: pk('b'),
                record_key: rk('d'),
                initiator_pseudonym: pk('e'),
                is_group: true,
            },
        ),
        ev(
            "conversationFocusRequested",
            ChannelMessageEvent::ConversationFocusRequested {
                peer_key: pk('b'),
                display_name: text("Bob"),
                reason: text("incomingCall"),
            },
        ),
    ]
}
