use std::cell::Cell;

use super::{send_prepared_reply, PreparedThreadReply, ThreadReplySent};
use crate::error::ChannelError;
use rekindle_types::message::SendState;

#[tokio::test]
async fn mesh_failure_after_the_durable_queue_returns_queued() {
    let queued = Cell::new(false);
    let prepared = PreparedThreadReply {
        thread_id: "0123456789abcdef0123456789abcdef".into(),
        sent: ThreadReplySent {
            message_id: "tmsg_0123456789abcdef0123456789abcdef".into(),
            sender_pseudonym: "author".into(),
            timestamp: 42,
            send_state: SendState::Queued,
        },
    };
    let sent = send_prepared_reply(
        prepared,
        async {
            queued.set(true);
            Ok(())
        },
        |_| {
            assert!(queued.get(), "mesh notification follows the durable queue");
            Err(ChannelError::Adapter("mesh unavailable".into()))
        },
    )
    .await
    .expect("the durable queue owns delivery");
    assert_eq!(sent.send_state, SendState::Queued);
}
