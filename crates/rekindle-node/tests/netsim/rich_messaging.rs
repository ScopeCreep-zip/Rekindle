//! Plan E3.1 E3: threads, reactions and polls through the Host on two
//! members — each write is read back by the other member, and a member
//! without the permission is refused.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

fn message_id(posted: &serde_json::Value) -> String {
    posted["messageId"]
        .as_str()
        .expect("a message id")
        .to_owned()
}

async fn ask(community: &Community, i: usize, request: CommunityRequest) -> serde_json::Value {
    community.nodes[i].ok(IpcRequest::Community(request)).await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn thread_reply_both_ways() {
    let community = Community::of(2, 231).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();
    let channel = community.general_id(0).await;
    let starter = community
        .post(0, "start a thread")
        .await
        .expect("ann posts");
    let thread = ask(
        &community,
        0,
        CommunityRequest::ThreadCreate {
            community: key.clone(),
            channel_id: channel.clone(),
            name: "plans".into(),
            starter_message_id: message_id(&starter),
            forum_tag: None,
            auto_archive_seconds: None,
        },
    )
    .await;
    let thread = thread.as_str().expect("a thread id").to_owned();

    let listed = eventually(&community.sim, LIMIT, || async {
        community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::ThreadListActive {
                community: key.clone(),
                channel_id: channel.clone(),
            }))
            .await
            .is_ok_and(|v| v.to_string().contains(&thread))
    })
    .await;
    assert!(listed, "bea lists ann's thread");

    let archive = community.nodes[1]
        .call(IpcRequest::Community(CommunityRequest::ThreadArchive {
            community: key.clone(),
            thread_id: thread.clone(),
        }))
        .await;
    assert!(
        archive.is_err_and(|refused| refused.code == 403),
        "a member who does not manage threads cannot archive one"
    );

    ask(
        &community,
        1,
        CommunityRequest::ThreadSend {
            community: key.clone(),
            thread_id: thread.clone(),
            body: "count me in".into(),
        },
    )
    .await;
    let replied = eventually(&community.sim, LIMIT, || async {
        community.nodes[0]
            .call(IpcRequest::Community(CommunityRequest::ThreadMessages {
                community: key.clone(),
                thread_id: thread.clone(),
                limit: 50,
                before_timestamp: None,
            }))
            .await
            .is_ok_and(|v| v.to_string().contains("count me in"))
    })
    .await;
    assert!(replied, "ann reads bea's reply in the thread");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reaction_converges() {
    let community = Community::of(2, 232).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();
    let posted = community.post(0, "react to me").await.expect("ann posts");
    assert!(
        community.history_holds(1, "react to me", LIMIT).await,
        "bea reads ann's post"
    );
    ask(
        &community,
        1,
        CommunityRequest::ReactionAdd {
            community: key,
            channel_id: community.general_id(1).await,
            message_id: message_id(&posted),
            emoji: "tada".into(),
        },
    )
    .await;
    let reacted = eventually(&community.sim, LIMIT, || async {
        community.history(0, None, 100).await.is_ok_and(|page| {
            page.iter()
                .any(|m| m["body"] == "react to me" && m["reactions"].to_string().contains("tada"))
        })
    })
    .await;
    assert!(reacted, "ann sees bea's reaction on her post");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn poll_vote_converges() {
    let community = Community::of(2, 233).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();
    let channel = community.general_id(0).await;
    let posted = community.post(0, "lunch?").await.expect("ann posts");
    let poll = ask(
        &community,
        0,
        CommunityRequest::PollCreate {
            community: key.clone(),
            channel_id: channel.clone(),
            message_id: message_id(&posted),
            question: "where".into(),
            answers: vec!["tacos".into(), "pho".into()],
            multi_select: false,
            duration_seconds: None,
        },
    )
    .await;
    let poll = poll.as_str().expect("a poll id").to_owned();

    let voted = eventually(&community.sim, LIMIT, || async {
        community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::PollVote {
                community: key.clone(),
                channel_id: channel.clone(),
                poll_id: poll.clone(),
                selected_answers: vec![1],
            }))
            .await
            .is_ok()
    })
    .await;
    assert!(voted, "bea votes once she reads the poll");

    let close = community.nodes[1]
        .call(IpcRequest::Community(CommunityRequest::PollClose {
            community: key.clone(),
            channel_id: channel.clone(),
            poll_id: poll.clone(),
        }))
        .await;
    assert!(
        close.is_err_and(|refused| refused.code == 403),
        "only the author or a moderator closes a poll"
    );

    let counted = eventually(&community.sim, LIMIT, || async {
        community.nodes[0]
            .call(IpcRequest::Community(CommunityRequest::PollResults {
                community: key.clone(),
                channel_id: channel.clone(),
                poll_id: poll.clone(),
            }))
            .await
            .is_ok_and(|v| v == serde_json::json!([0, 1]))
    })
    .await;
    assert!(counted, "ann counts bea's vote");
}
