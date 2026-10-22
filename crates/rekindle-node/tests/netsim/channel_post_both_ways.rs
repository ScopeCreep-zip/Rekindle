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

/// Each member reads the other's post through `get_channel_messages` (the
/// Host's merge of its rows with the channel record), and
/// `get_older_channel_messages` pages back from the newest (plan E3.1 E2c).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn channel_history_read() {
    let community = Community::of(2, 214).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    for (i, body) in ["one from ann", "two from ann", "three from ann"]
        .into_iter()
        .enumerate()
    {
        community.post(0, body).await.expect("ann posts");
        tokio::time::sleep(Duration::from_secs(u64::try_from(i).unwrap_or(0) + 1)).await;
    }
    community
        .post(1, "hello from bea")
        .await
        .expect("bea posts");

    assert!(
        community.history_holds(0, "hello from bea", LIMIT).await,
        "ann reads bea's post"
    );
    assert!(
        community.history_holds(1, "three from ann", LIMIT).await,
        "bea reads ann's post"
    );

    let page = community.history(1, None, 2).await.expect("a page");
    assert_eq!(page.len(), 2, "a page is cut to its limit");
    let oldest = page
        .iter()
        .filter_map(|m| m["timestamp"].as_u64())
        .min()
        .expect("timestamps");
    let older = community
        .history(1, Some(oldest), 100)
        .await
        .expect("an older page");
    assert!(!older.is_empty(), "there are older messages");
    assert!(
        older
            .iter()
            .all(|m| m["timestamp"].as_u64().is_some_and(|t| t < oldest)),
        "an older page holds only messages before its cursor"
    );
    assert!(
        older.iter().any(|m| m["body"] == "one from ann"),
        "the first post is on the older page"
    );
}
