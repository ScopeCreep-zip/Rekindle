//! Forward cross-audience (plan E3.1 E2d): forwarding a post from a channel
//! two members read into a community only its creator is in is held for
//! review with `CrossAudience` naming both audiences, and once confirmed
//! the forward is posted. A forward is live as one: its `New` names the
//! original author on the forwarder's bus and on a reader's, the value
//! history serves (round 28 finding: it rendered as a plain post until a
//! reload).

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_types::intent::Escalation;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events};

const LIMIT: Duration = Duration::from_secs(180);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forward_to_another_membership_is_held_with_both_counts() {
    let community = Community::of(2, 227).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let creator = &community.nodes[0];

    let elsewhere = creator
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name: "Elsewhere".into(),
                description: String::new(),
                approval_required: false,
                co_creators: Vec::new(),
            },
        )))
        .await["anchor"]
        .as_str()
        .expect("an anchor key")
        .to_owned();
    let posted = community
        .post(0, "worth sharing")
        .await
        .expect("the creator posts");
    let source_message = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();

    let forward = || {
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageForward {
                source_community: community.anchor.clone(),
                source_channel: "general".into(),
                source_message: source_message.clone(),
                dest_community: elsewhere.clone(),
                dest_channel: "general".into(),
            },
        ))
    };
    let IpcResponse::IntentRequired(review) = creator.answer(forward()).await else {
        panic!("a forward to another membership is not held");
    };
    let audiences = review.escalations.iter().find_map(|e| match e {
        Escalation::CrossAudience {
            from_members,
            to_members,
        } => Some((*from_members, *to_members)),
        _ => None,
    });
    assert_eq!(
        audiences,
        Some((2, 1)),
        "both members read the source; only the creator is elsewhere"
    );
    creator
        .ok(IpcRequest::IntentCancel {
            intent_id: review.intent_id,
        })
        .await;

    let forwarded = creator
        .call_held(forward())
        .await
        .expect("the forward runs once confirmed");
    assert!(
        forwarded["messageId"]
            .as_str()
            .is_some_and(|id| id != source_message),
        "the forward is a new message: {forwarded}"
    );
}

/// The forward `New` on `events` (its id and original author), the post
/// `source` excluded.
fn forward_seen(events: &Events, source: &str) -> Option<(String, String)> {
    let found = std::cell::RefCell::new(None);
    events.any(|event| match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            message_id,
            forwarded_from_author: Some(author),
            ..
        }) if message_id != source => {
            *found.borrow_mut() = Some((message_id.clone(), author.clone()));
            true
        }
        _ => false,
    });
    found.into_inner()
}

/// The `field` of message `id` in member `i`'s history.
async fn history_field(community: &Community, i: usize, id: &str, field: &str) -> Option<String> {
    let page = community.history(i, None, 100).await.ok()?;
    page.iter()
        .find(|m| m["serverMessageId"] == id)
        .and_then(|m| m[field].as_str().map(str::to_owned))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forward_is_live_with_its_original_author_for_both_members() {
    let community = Community::of(2, 228).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let seen_by = [
        community.nodes[0].events().await,
        community.nodes[1].events().await,
    ];
    let posted = community.post(1, "worth sharing").await.expect("bea posts");
    let source = posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned();
    assert!(
        community.history_holds(0, "worth sharing", LIMIT).await,
        "ann reads bea's post"
    );
    let author = history_field(&community, 0, &source, "senderId")
        .await
        .expect("bea's pseudonym on her post");

    let ann = &community.nodes[0];
    let forward = || {
        IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessageForward {
                source_community: community.anchor.clone(),
                source_channel: "general".into(),
                source_message: source.clone(),
                dest_community: community.anchor.clone(),
                dest_channel: "general".into(),
            },
        ))
    };
    if let IpcResponse::IntentRequired(review) = ann.answer(forward()).await {
        ann.ok(IpcRequest::IntentCancel {
            intent_id: review.intent_id,
        })
        .await;
        ann.call_held(forward())
            .await
            .expect("the forward runs once confirmed");
    }

    let echoed_live = eventually(&community.sim, LIMIT, || async {
        forward_seen(&seen_by[0], &source).is_some()
    })
    .await;
    assert!(echoed_live, "ann's echo of her forward names an author");
    let (forward_id, echoed) = forward_seen(&seen_by[0], &source).expect("the echo seen above");
    assert_eq!(echoed, author, "the echo names bea, not the forwarder");
    let read = eventually(&community.sim, LIMIT, || async {
        forward_seen(&seen_by[1], &source).is_some_and(|(id, by)| id == forward_id && by == author)
    })
    .await;
    assert!(
        read,
        "bea's live read of the forward names her as its author"
    );
    for i in [0, 1] {
        assert_eq!(
            history_field(&community, i, &forward_id, "forwardedFromAuthor")
                .await
                .as_deref(),
            Some(author.as_str()),
            "member {i}'s history serves the author the live event named"
        );
    }
}
