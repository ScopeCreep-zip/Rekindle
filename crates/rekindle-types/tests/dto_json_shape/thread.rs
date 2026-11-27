//! The `ThreadSend` and `CreateForumPost` answers (plan E3.3, FT1) in the
//! golden DTO file (D22 `SendState`, `queued` when the Host answers).

use rekindle_types::dto::community::CommunityChannelView;
use rekindle_types::dto::community_capability::ChannelCapability;
use rekindle_types::message::SendState;

use super::to;
use rekindle_types::thread::{thread_reply_key, ForumPostCreated, ThreadReplyOutcome};
use serde_json::Value;

pub fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "community.channelView",
        to(CommunityChannelView {
            id: "0c".repeat(16),
            name: "Ideas".into(),
            channel_type: "forum".into(),
            category_id: None,
            topic: String::new(),
            forum_tags: None,
            stage_speakers: Vec::new(),
            stage_moderator: None,
            slowmode_seconds: None,
            notification_level: "default".into(),
            notification_sound_ref: None,
            locked: false,
            locked_reason: String::new(),
            denied: [
                (
                    ChannelCapability::CreatePublicThreads,
                    "You need the Create Public Threads permission.".into(),
                ),
                (
                    ChannelCapability::SendInThreads,
                    "You need the Send In Threads permission.".into(),
                ),
            ]
            .into(),
        }),
    );

    add(
        "ThreadReplyOutcome.queued",
        serde_json::to_value(ThreadReplyOutcome {
            message_id: thread_reply_key(&"ab".repeat(32), "tmsg_0123"),
            send_state: SendState::Queued,
        })
        .expect("serialises"),
    );
    add(
        "ForumPostCreated.queued",
        serde_json::to_value(ForumPostCreated {
            thread_id: "ab".repeat(16),
            starter_message_id: thread_reply_key(&"ab".repeat(32), "tmsg_4567"),
            starter: SendState::Queued,
        })
        .expect("serialises"),
    );
}
