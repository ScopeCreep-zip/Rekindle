//! Plan E3.1 E2b: two members each post to the general channel through the
//! Host's channel send, and each sees the other's post as a
//! `ChannelMessageEvent::New` on its Host bus.

use std::time::Duration;

use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

fn message_id(posted: &serde_json::Value) -> String {
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

fn saw_post(events: &Events, id: &str) -> bool {
    SimNode::saw(events, |event| {
        matches!(
            event,
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { message_id, .. })
                if message_id == id
        )
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn channel_post_both_ways() {
    let community = Community::of(2, 213).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let seen_by = [
        community.nodes[0].events().await,
        community.nodes[1].events().await,
    ];

    let first = community
        .post(0, "hello from ann")
        .await
        .expect("ann posts");
    let second = community
        .post(1, "hello from bea")
        .await
        .expect("bea posts");
    let (first, second) = (message_id(&first), message_id(&second));

    let crossed = eventually(&community.sim, LIMIT, || async {
        saw_post(&seen_by[1], &first) && saw_post(&seen_by[0], &second)
    })
    .await;
    assert!(crossed, "each member sees the other's post");
}
