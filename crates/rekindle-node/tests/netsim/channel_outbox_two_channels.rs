//! Plan E3.3 (CHN1): every channel of a community appends to the author's
//! one outbox chain (`M`/4), but each channel is its own `channel_post`
//! stream, drained on its own. Two posts in two channels, sent together,
//! were drained at once: both drains read the same tip and wrote chain seq
//! 1, and the later head replaced the earlier, so one post was lost while
//! its row read sent (Mac FX3). Both posts reach a second member.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelMessageRequest, ChannelRequest, CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(240);

fn send(community: &Community, channel: &str, body: &str) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Messages(
        ChannelMessageRequest::ChannelSend {
            community: community.anchor.clone(),
            channel: channel.into(),
            body: body.into(),
            reply_to: None,
        },
    ))
}

/// Whether node `i`'s page of `channel` holds `body` now.
async fn holds(community: &Community, i: usize, channel: &str, body: &str) -> bool {
    let page = community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelMessages {
                channel: channel.to_owned(),
                limit: 100,
            },
        )))
        .await;
    page.is_ok_and(|page| (page.as_array().into_iter().flatten()).any(|m| m["body"] == body))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn posts_in_two_channels_sent_together_both_reach_a_member() {
    let community = Community::of(2, 241).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let den = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "den".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await["id"]
        .as_str()
        .expect("a channel id")
        .to_owned();
    let general = community.general_id(0).await;
    let listed = eventually(&community.sim, LIMIT, || async {
        let list = community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::Channels(
                ChannelRequest::ChannelList {
                    community: community.anchor.clone(),
                },
            )))
            .await;
        list.is_ok_and(|l| (l.as_array().into_iter().flatten()).any(|c| c["id"] == den))
    })
    .await;
    assert!(listed, "the member lists the new channel");

    let author = &community.nodes[0];
    let (in_general, in_den) = tokio::join!(
        author.call(send(&community, "general", "posted in general")),
        author.call(send(&community, "den", "posted in den")),
    );
    in_general.expect("the general post is accepted");
    in_den.expect("the den post is accepted");

    let both = eventually(&community.sim, LIMIT, || async {
        holds(&community, 1, &general, "posted in general").await
            && holds(&community, 1, &den, "posted in den").await
    })
    .await;
    assert!(
        both,
        "both posts are on the author's chain and readable by a member"
    );
}
