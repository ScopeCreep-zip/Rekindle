//! Plan E3.3 M4: a moderator's delete is the governance op `AdminDelete`,
//! which every reader applies from its merge as a tombstone naming the
//! moderator, under delete-wins. The post is removed for every member, its
//! body gone, shown as "removed by" the moderator, and a reload (the stored
//! rows merged with the channel record, which still holds the post) never
//! restores it. No gossip notice carries it (`messageDeleted` @13 retired).

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

/// Node `i`'s reload of the general channel: message `id`'s body and the
/// moderator who removed it, when it is shown.
async fn shown(community: &Community, i: usize, id: &str) -> Option<(String, Option<String>)> {
    let page = community.history(i, None, 100).await.ok()?;
    page.iter().find(|m| m["serverMessageId"] == id).map(|m| {
        (
            m["body"].as_str().unwrap_or("").to_owned(),
            m["removedBy"].as_str().map(str::to_owned),
        )
    })
}

async fn removed_for(community: &Community, i: usize, id: &str, by: &str) -> bool {
    eventually(&community.sim, LIMIT, || async {
        shown(community, i, id).await == Some((String::new(), Some(by.to_owned())))
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn moderator_delete_removes_for_everyone() {
    let community = Community::of(3, 216).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let posted = community.post(1, "spam from bea").await.expect("bea posts");
    let id = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();
    for i in [0, 2] {
        assert!(
            community.history_holds(i, "spam from bea", LIMIT).await,
            "node {i} reads bea's post"
        );
    }

    let channel = community.general_id(0).await;
    let request = ChannelMessageRequest::ChannelMessageAdminDelete {
        community: community.anchor.clone(),
        channel,
        message_id: id.clone(),
        reason: Some("spam".into()),
    };
    community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::Messages(request)))
        .await
        .expect("ann, the owner, removes bea's post");

    let ann = community.pseudonym(0).await;
    for (i, who) in [(0, "ann"), (1, "bea"), (2, "cal")] {
        assert!(
            removed_for(&community, i, &id, &ann).await,
            "{who}'s reload shows the post removed by ann: {:?}",
            shown(&community, i, &id).await
        );
    }
    tokio::time::sleep(Duration::from_secs(120)).await;
    for i in 0..3 {
        assert_eq!(
            shown(&community, i, &id).await,
            Some((String::new(), Some(ann.clone()))),
            "node {i}: a later reload never restores the post"
        );
    }
}
