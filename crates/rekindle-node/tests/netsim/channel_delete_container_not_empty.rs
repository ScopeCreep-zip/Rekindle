//! Channel delete, container not empty (plan E3.1 E1): deleting a channel
//! its members read is held for review with `ContainerNotEmpty` (the
//! messages stored for it and its readers, `channel_read_roster`), and
//! once confirmed it is gone on both members.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_types::intent::Escalation;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn deleting_a_read_channel_is_held_then_deletes_on_both_members() {
    let community = Community::of(2, 163).await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    let creator = &community.nodes[0];
    let created = creator
        .ok(IpcRequest::Community(CommunityRequest::ChannelCreate {
            community: community.governance_key.clone(),
            name: "doomed".into(),
            kind: "text".into(),
            category: None,
            topic: None,
            slowmode_seconds: 0,
            parent_voice_channel: None,
        }))
        .await;
    let channel = created["id"].as_str().expect("a channel id").to_owned();
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.info(1).await.to_string().contains("doomed")
        })
        .await,
        "the member sees the new channel"
    );

    let delete = || {
        IpcRequest::Community(CommunityRequest::ChannelDelete {
            community: community.governance_key.clone(),
            channel_id: channel.clone(),
        })
    };
    let IpcResponse::IntentRequired(review) = creator.answer(delete()).await else {
        panic!("deleting a channel two members read is not held");
    };
    let held = review.escalations.iter().find_map(|e| match e {
        Escalation::ContainerNotEmpty { messages, members } => Some((*messages, *members)),
        _ => None,
    });
    let (_, members) = held.expect("the review names ContainerNotEmpty");
    assert_eq!(members, 2, "both members read the channel");
    creator
        .ok(IpcRequest::IntentCancel {
            intent_id: review.intent_id,
        })
        .await;

    creator
        .call_held(delete())
        .await
        .expect("the delete runs once confirmed");
    for i in 0..2 {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                !community.info(i).await.to_string().contains("doomed")
            })
            .await,
            "member {i} no longer has the channel"
        );
    }
}
