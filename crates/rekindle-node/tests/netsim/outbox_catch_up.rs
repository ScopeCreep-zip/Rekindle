//! Plan E3.3 (slice 5b): a channel post is an entry of its author's channel
//! outbox (`M`/4), with no slot write and no gossip notification. A member
//! who joins after the post was made reads it by the catch-up of every
//! member's outbox from its `stream_cursors` cursor (the community's load),
//! the body opened at its `{anchor, channel, author, streamSeq}` position.

use std::time::Duration;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_reads_a_post_made_before_it_joined_from_the_outbox() {
    let mut community = Community::create(2, 431).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    community
        .post(0, "posted before you came")
        .await
        .expect("the creator posts");
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.join(1).await;
    assert!(
        community
            .history_holds(1, "posted before you came", Duration::from_secs(300))
            .await,
        "the joiner's catch-up reads the creator's outbox"
    );
}
