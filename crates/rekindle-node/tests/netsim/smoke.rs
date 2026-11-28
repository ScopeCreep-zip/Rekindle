//! The base flow every scenario builds on: two identities unlock, one
//! creates a community and invites the other, who joins, and each reads
//! the other's post.

use std::time::Duration;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn two_hosts_unlock_create_invite_join_and_post() {
    let community = Community::of(2, 1).await;
    community
        .post(0, "hello from the creator")
        .await
        .expect("the creator posts");
    community
        .post(1, "hello from the joiner")
        .await
        .expect("the joiner posts");
    let limit = Duration::from_secs(120);
    assert!(community.reads(1, "hello from the creator", limit).await);
    assert!(community.reads(0, "hello from the joiner", limit).await);
}
