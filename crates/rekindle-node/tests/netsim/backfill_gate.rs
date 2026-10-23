//! Plan E3.1 (D26, register FX2): every received channel post passes the
//! Host gate, whatever path brought it in. A banned author's post, which
//! the channel record still holds, is not shown by another member's
//! history backfill; an automod `BlurContent` match reaches the bus as a
//! blurred `ChannelMessageEvent::New`.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::governance::AutoModAction;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn banned_post_not_backfilled() {
    let community = Community::of(3, 253).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let bea = community.pseudonym(1).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Ban {
                community: community.governance_key.clone(),
                target_pseudonym: bea,
                reason: None,
            },
        )))
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    community
        .post(1, "from bea, banned")
        .await
        .expect("the send path writes the post to the record");
    assert!(
        community.history_holds(1, "from bea, banned", LIMIT).await,
        "the post is in bea's record"
    );
    community.post(0, "from ann").await.expect("ann posts");
    assert!(
        community.history_holds(2, "from ann", LIMIT).await,
        "cat's backfill reads the channel record"
    );
    let page = community.history(2, None, 100).await.expect("cat's page");
    assert!(
        !page.iter().any(|m| m["body"] == "from bea, banned"),
        "the banned author's backfilled post is not shown"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn automod_blur_carried_on_event() {
    let community = Community::of(2, 257).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::AutomodRuleSet {
                community: community.governance_key.clone(),
                rule_id: None,
                name: "spoilers".into(),
                enabled: true,
                keywords: vec!["spoiler".into()],
                regex_patterns: Vec::new(),
                action: AutoModAction::BlurContent,
            },
        )))
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let ann = community.nodes[0].events().await;
    let posted = community.post(1, "a spoiler").await.expect("bea posts");
    let id = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            SimNode::saw(&ann, |event| {
                matches!(
                    event,
                    SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                        message_id,
                        automod_blurred: true,
                        ..
                    }) if *message_id == id
                )
            })
        })
        .await,
        "the hidden post reaches the bus blurred"
    );
}
