//! `ChannelMessageEvent` samples.

use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "new",
    "edited",
    "deleted",
    "channelSendStatus",
    "attachmentDownloaded",
    "expressionAssetReady",
    "directMessageReceived",
    "dmSendStatus",
    "dmHistoryGap",
    "conversationFocusRequested",
];

pub fn variant_name(e: &ChannelMessageEvent) -> &'static str {
    match e {
        ChannelMessageEvent::New { .. } => "new",
        ChannelMessageEvent::Edited { .. } => "edited",
        ChannelMessageEvent::Deleted { .. } => "deleted",
        ChannelMessageEvent::ChannelSendStatus { .. } => "channelSendStatus",
        ChannelMessageEvent::AttachmentDownloaded { .. } => "attachmentDownloaded",
        ChannelMessageEvent::ExpressionAssetReady { .. } => "expressionAssetReady",
        ChannelMessageEvent::DirectMessageReceived { .. } => "directMessageReceived",
        ChannelMessageEvent::DmSendStatus { .. } => "dmSendStatus",
        ChannelMessageEvent::DmHistoryGap { .. } => "dmHistoryGap",
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
                reply_to_message_id: Some("msg_41".into()),
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
            "channelSendStatus",
            ChannelMessageEvent::ChannelSendStatus {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                state: rekindle_types::message::SendState::Sent,
            },
        ),
        ev(
            "attachmentDownloaded",
            ChannelMessageEvent::AttachmentDownloaded {
                community: community(),
                channel: channel(),
                attachment_id: id("att_", '1'),
            },
        ),
        ev(
            "expressionAssetReady",
            ChannelMessageEvent::ExpressionAssetReady {
                community: community(),
                expression_id: id("exp_", '1'),
            },
        ),
        ev(
            "dmSendStatus",
            ChannelMessageEvent::DmSendStatus {
                client_id: text("01900000-0000-7000-8000-000000000002"),
                conversation_id: pk('b'),
                state: rekindle_types::message::SendState::Failed,
            },
        ),
        ev(
            "dmHistoryGap",
            ChannelMessageEvent::DmHistoryGap {
                peer_key: pk('b'),
                through_seq: 17,
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
