//! Plan E3.3 (slice 5b): a channel post is an entry of its author's channel
//! outbox (`M`/4), with no slot write and no gossip notification. A member
//! who joins after the post was made reads it by the catch-up of every
//! outbox from its `stream_cursors` cursor (the community's load), the body
//! opened at its `{anchor, channel, author, streamSeq}` position.
//!
//! The author leaves before the joiner comes, so only the catch-up can
//! carry the post: a departed author is on no watch ring (ring successors
//! are drawn from the merged members) and sends no `HeadAdvanced`, while
//! the catch-up reads each departed author whose departure recorded a head
//! of its outbox. A current author would also reach the joiner through its
//! ring watch's first report of subkey 4, which hides a missing catch-up.

use std::time::Duration;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_reads_a_post_made_before_it_joined_from_the_outbox() {
    let mut community = Community::create(3, 431).await;
    community.join(1).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    community
        .post(1, "posted before you came")
        .await
        .expect("the member posts");
    assert!(
        community
            .reads(0, "posted before you came", Duration::from_secs(120))
            .await,
        "the creator reads the post"
    );
    rekindle_governance_runtime::moderation::leave(
        &community.nodes[1].host.governance(),
        &community.anchor,
    )
    .await
    .expect("the leave op commits");
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.join(2).await;
    assert!(
        community
            .history_holds(2, "posted before you came", Duration::from_secs(300))
            .await,
        "the joiner's catch-up reads the departed author's outbox"
    );
}
