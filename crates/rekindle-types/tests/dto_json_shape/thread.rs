//! The `ThreadSend` and `CreateForumPost` answers (plan E3.3, FT1) in the
//! golden DTO file (D22 `SendState`, `queued` when the Host answers).

use rekindle_types::message::SendState;
use rekindle_types::thread::{ForumPostCreated, ThreadReplyOutcome};
use serde_json::Value;

pub fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "ThreadReplyOutcome.queued",
        serde_json::to_value(ThreadReplyOutcome {
            message_id: "tmsg_0123".into(),
            send_state: SendState::Queued,
        })
        .expect("serialises"),
    );
    add(
        "ForumPostCreated.queued",
        serde_json::to_value(ForumPostCreated {
            thread_id: "ab".repeat(16),
            starter_message_id: "tmsg_4567".into(),
            starter: SendState::Queued,
        })
        .expect("serialises"),
    );
}
