//! Forward cross-audience (plan E3.1 E2d): forwarding a post from a channel
//! two members read into a community only its creator is in is held for
//! review with `CrossAudience` naming both audiences, and once confirmed
//! the forward is posted.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, LifecycleRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_types::intent::Escalation;

use crate::harness::Community;

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
            },
        )))
        .await["governanceKey"]
        .as_str()
        .expect("a governance key")
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
                source_community: community.governance_key.clone(),
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
