//! `ChannelMessageEvent` samples.

use rekindle_types::subscription_events::{
    ChannelMessageEvent, ConversationContext, SubscriptionEvent,
};

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "new",
    "edited",
    "deleted",
    "channelSendStatus",
    "channelHistoryGap",
    "channelHistoryGapClosed",
    "attachmentDownloaded",
    "expressionAssetReady",
    "directMessageReceived",
    "directMessageSent",
    "dmSendStatus",
    "dmHistoryGap",
    "disappearingTimerChanged",
    "messagesExpired",
    "conversationFocusRequested",
];

pub fn variant_name(e: &ChannelMessageEvent) -> &'static str {
    match e {
        ChannelMessageEvent::New { .. } => "new",
        ChannelMessageEvent::Edited { .. } => "edited",
        ChannelMessageEvent::Deleted { .. } => "deleted",
        ChannelMessageEvent::ChannelSendStatus { .. } => "channelSendStatus",
        ChannelMessageEvent::ChannelHistoryGap { .. } => "channelHistoryGap",
        ChannelMessageEvent::ChannelHistoryGapClosed { .. } => "channelHistoryGapClosed",
        ChannelMessageEvent::AttachmentDownloaded { .. } => "attachmentDownloaded",
        ChannelMessageEvent::ExpressionAssetReady { .. } => "expressionAssetReady",
        ChannelMessageEvent::DirectMessageReceived { .. } => "directMessageReceived",
        ChannelMessageEvent::DirectMessageSent { .. } => "directMessageSent",
        ChannelMessageEvent::DmSendStatus { .. } => "dmSendStatus",
        ChannelMessageEvent::DmHistoryGap { .. } => "dmHistoryGap",
        ChannelMessageEvent::DisappearingTimerChanged { .. } => "disappearingTimerChanged",
        ChannelMessageEvent::MessagesExpired { .. } => "messagesExpired",
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
                forwarded_from_author: Some(pk('b')),
                forwarded_from_name: Some("bbbb bbbb".into()),
                automod_blurred: true,
                undecryptable: None,
            },
        ),
        ev(
            "new.undecryptable",
            ChannelMessageEvent::New {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '2'),
                sender_pseudonym: pk('a'),
                sequence: 43,
                timestamp: 1_760_000_000_001,
                body: None,
                reply_to_message_id: None,
                forwarded_from_author: None,
                forwarded_from_name: None,
                automod_blurred: false,
                undecryptable: Some(
                    rekindle_types::dto::channel_message::UndecryptableReason::Damaged.into(),
                ),
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
                removed_by: Some(pk('c')),
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
                reason: None,
            },
        ),
        ev(
            "channelHistoryGap",
            ChannelMessageEvent::ChannelHistoryGap {
                community: community(),
                channel: channel(),
                sender_pseudonym: pk('b'),
                through_seq: 4,
            },
        ),
        ev(
            "channelHistoryGapClosed",
            ChannelMessageEvent::ChannelHistoryGapClosed {
                community: community(),
                sender_pseudonym: pk('b'),
                through_seq: 4,
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
            "directMessageSent",
            ChannelMessageEvent::DirectMessageSent {
                conversation_id: pk('b'),
                reply_to_id: Some(text("01900000-0000-7000-8000-000000000001")),
                message: Box::new(rekindle_types::dto::dm::DmMessage {
                    id: 42,
                    message_id: Some(text("01900000-0000-7000-8000-000000000002")),
                    sender_id: pk('a'),
                    body: text("gg"),
                    timestamp: 1_700_000_000_000,
                    is_own: true,
                    send_state: rekindle_types::message::SendState::Queued,
                    history_gap_through: None,
                    expire_seconds: 300,
                    expire_at: Some(1_700_000_300_000),
                    timer_notice_seconds: None,
                    reply_to_id: None,
                }),
            },
        ),
        ev(
            "dmSendStatus",
            ChannelMessageEvent::DmSendStatus {
                client_id: text("01900000-0000-7000-8000-000000000002"),
                conversation_id: pk('b'),
                state: rekindle_types::message::SendState::Sent,
                retry: Some(rekindle_types::message::DeliveryRetry {
                    attempts: 3,
                    next_attempt_at: 1_700_000_004_000,
                }),
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
            "disappearingTimerChanged",
            ChannelMessageEvent::DisappearingTimerChanged {
                context: ConversationContext::Dm { peer_key: pk('b') },
                seconds: 3_600,
                set_by: pk('a'),
                client_id: "0190a0b0-0000-7000-8000-000000000002".into(),
                timestamp: 1_760_000_003_000,
            },
        ),
        ev(
            "messagesExpired",
            ChannelMessageEvent::MessagesExpired {
                context: ConversationContext::GroupDm {
                    group_id: "0f".repeat(16),
                },
                client_ids: vec!["0190a0b0-0000-7000-8000-000000000001".into()],
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
