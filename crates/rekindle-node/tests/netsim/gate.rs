//! Plan E3.1 F3 (D26): the ingest gate and the send path over the netsim,
//! driven by production requests: slowmode refuses a second post at send,
//! a banned member's post never reaches the others, a timed-out member's
//! post is kept out until the timeout ends on the sim clock, and an automod
//! rule whose regex cannot compile drops what is posted.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::governance::AutoModAction;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

fn saw_post(events: &Events, id: &str) -> bool {
    SimNode::saw(events, |event| {
        matches!(
            event,
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { message_id, .. })
                if message_id == id
        )
    })
}

fn message_id(posted: &serde_json::Value) -> String {
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

/// Node `from`'s post, unless its send path refuses it; when it is sent,
/// node `to` never sees it within a minute.
async fn kept_out(community: &Community, from: usize, to: &Events, body: &str) {
    if let Ok(posted) = community.post(from, body).await {
        let id = message_id(&posted);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!saw_post(to, &id), "{body:?} is kept out");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn slowmode_refused() {
    let community = Community::of(2, 241).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let general = community.general_id(0).await;
    community.nodes[0]
        .ok(IpcRequest::Community(
            CommunityRequest::ChannelSetSlowmode {
                community: community.governance_key.clone(),
                channel_id: general,
                seconds: 60,
            },
        ))
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.post(1, "first").await.expect("the first post");
    let refused = community
        .post(1, "second")
        .await
        .expect_err("a second post inside the slowmode is refused");
    assert_eq!(refused.code, 429, "{}", refused.message);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn banned_post_dropped() {
    let community = Community::of(3, 243).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let bea = community.pseudonym(1).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Ban {
            community: community.governance_key.clone(),
            target_pseudonym: bea,
            reason: None,
        }))
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let cat = community.nodes[2].events().await;
    kept_out(&community, 1, &cat, "from bea, banned").await;
    let shown = message_id(&community.post(0, "from ann").await.expect("ann posts"));
    assert!(
        eventually(&community.sim, LIMIT, || async { saw_post(&cat, &shown) }).await,
        "the channel is live for the others"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn timeout_expires_on_post() {
    let community = Community::of(2, 247).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let bea = community.pseudonym(1).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Timeout {
            community: community.governance_key.clone(),
            target_pseudonym: bea,
            duration_seconds: 240,
            reason: None,
        }))
        .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let ann = community.nodes[0].events().await;
    kept_out(&community, 1, &ann, "from bea, timed out").await;
    tokio::time::sleep(Duration::from_secs(240)).await;
    let shown = message_id(
        &community
            .post(1, "from bea, after")
            .await
            .expect("bea posts after the timeout"),
    );
    assert!(
        eventually(&community.sim, LIMIT, || async { saw_post(&ann, &shown) }).await,
        "after the timeout the post is admitted"
    );
}

/// A rule whose regex does not compile is malformed, so the Host refuses
/// it with 400 before writing, as every reader would drop it at merge; the
/// writer's gate never holds it and admits the post every member admits.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn automod_uncompilable_rule_refused() {
    let community = Community::of(2, 251).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let refused = community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::AutomodRuleSet {
            community: community.governance_key.clone(),
            rule_id: None,
            name: "broken".into(),
            enabled: true,
            keywords: Vec::new(),
            regex_patterns: vec!["(".into()],
            action: AutoModAction::BlockLocally,
        }))
        .await
        .expect_err("an uncompilable rule is refused");
    assert_eq!(refused.code, 400, "{}", refused.message);
    let ann = community.nodes[0].events().await;
    let posted = community.post(1, "(").await.expect("bea posts");
    let id = message_id(&posted);
    assert!(
        eventually(&community.sim, LIMIT, || async { saw_post(&ann, &id) }).await,
        "with no rule written, the post is admitted"
    );
}
