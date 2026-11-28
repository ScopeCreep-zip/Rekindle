//! Plan E3.3 M4: a moderator's delete is the governance op `AdminDelete`,
//! which every reader applies from its merge as a tombstone naming the
//! moderator, under delete-wins. The post is removed for every member, its
//! body gone, shown as "removed by" the moderator, and a reload (the stored
//! rows merged with the channel record, which still holds the post) never
//! restores it. No gossip notice carries it (`messageDeleted` @13 retired).
//! M1: a moderator removes only a post whose author ranks no higher.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::permissions::MANAGE_MESSAGES;

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

/// Node `by` asks to remove message `id` of the general channel.
async fn delete_as(
    community: &Community,
    by: usize,
    id: &str,
) -> Result<serde_json::Value, crate::harness::Refused> {
    let request = ChannelMessageRequest::ChannelMessageAdminDelete {
        community: community.anchor.clone(),
        channel: community.general_id(by).await,
        message_id: id.to_owned(),
        reason: None,
    };
    community.nodes[by]
        .call(IpcRequest::Community(CommunityRequest::Messages(request)))
        .await
}

/// Node `i`'s post `body`: its message id, once every node reads it.
async fn posted(community: &Community, i: usize, body: &str) -> String {
    let posted = community.post(i, body).await.expect("the member posts");
    for reader in 0..community.nodes.len() {
        assert!(
            community.history_holds(reader, body, LIMIT).await,
            "node {reader} reads {body:?}"
        );
    }
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

/// M1: bea, a moderator, removes cal's post (cal holds only @everyone),
/// and every member shows it removed by bea; bea's delete of ann's post
/// (ann, the creator, outranks every role) is refused, and ann's post
/// stays for every member.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_moderator_removes_only_lower_ranked_posts() {
    let community = Community::of(3, 217).await;
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: "moderators".into(),
                permissions: MANAGE_MESSAGES,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.anchor.clone(),
                member_pseudonym: community.pseudonym(1).await,
                role_id: role_id.clone(),
            },
        )))
        .await;
    assert!(
        community.holds_role(1, &role_id).await,
        "bea is a moderator"
    );

    let cals = posted(&community, 2, "spam from cal").await;
    let anns = posted(&community, 0, "rules from ann").await;
    delete_as(&community, 1, &cals)
        .await
        .expect("bea removes cal's post");
    let bea = community.pseudonym(1).await;
    for i in 0..3 {
        assert!(
            removed_for(&community, i, &cals, &bea).await,
            "node {i} shows cal's post removed by bea: {:?}",
            shown(&community, i, &cals).await
        );
    }

    assert!(
        delete_as(&community, 1, &anns).await.is_err(),
        "M1: bea may not remove the creator's post"
    );
    tokio::time::sleep(Duration::from_secs(120)).await;
    for i in 0..3 {
        assert_eq!(
            shown(&community, i, &anns).await,
            Some(("rules from ann".to_owned(), None)),
            "node {i} still shows ann's post"
        );
    }
}
