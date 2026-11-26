//! Plan E4.1 S5a2-1: three Host nodes in one community join voice under
//! `AudioIo::HEADLESS` (the netsim never opens a real device). Each join
//! starts the Host's one engine (lead ruling 2026-11-26: one engine, the
//! Host's) with a live session on the community channel; a second join on
//! another channel is refused in the ruling's words, never replacing the
//! first; leave and teardown (lock) end the session.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::subscription_events::VoiceScope;

use crate::harness::Community;

fn join(community: &str, channel: &str) -> IpcRequest {
    IpcRequest::VoiceJoin {
        community: community.to_owned(),
        channel: channel.to_owned(),
        muted: false,
        deafened: false,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn community_voice_join_leave() {
    let community = Community::of(3, 541).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let channel = community.general_id(0).await;
    let scope = VoiceScope::Community {
        community: community.anchor.clone(),
        channel: channel.clone(),
    };

    for node in &community.nodes {
        node.call(join(&community.anchor, &channel))
            .await
            .expect("each member joins voice");
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    for (i, node) in community.nodes.iter().enumerate() {
        let roster = node.host.voice().roster().await;
        let roster = roster.unwrap_or_else(|| panic!("node {i} has a live voice session"));
        assert_eq!(roster.scope, scope, "node {i}'s session is the channel's");
        assert!(
            node.host.voice().engine_present(),
            "node {i}'s engine is up"
        );
        assert!(
            node.host.voice().transport().is_some(),
            "node {i}'s transport is up"
        );
    }

    // The same channel again is already joined.
    community.nodes[0]
        .call(join(&community.anchor, &channel))
        .await
        .expect("a rejoin of the live channel is no new session");
    // Another channel while one is live is refused, never switched. Its id
    // is well formed (the general channel's with its first digit changed),
    // so the refusal is the precedence ruling's, not the id check's.
    let first = if channel.starts_with('0') { "1" } else { "0" };
    let another = format!("{first}{}", &channel[1..]);
    let refused = community.nodes[0]
        .call(join(&community.anchor, &another))
        .await
        .expect_err("a second session is refused");
    assert_eq!(refused.message, "Leave voice in Netsim first.");
    assert_eq!(
        community.nodes[0].host.voice().session(),
        Some(scope.clone()),
        "the first session stays"
    );

    // Leave ends the session; a second leave has nothing to leave.
    let left = community.nodes[0].ok(IpcRequest::VoiceLeave).await;
    assert_eq!(left["left"], true);
    assert!(community.nodes[0].host.voice().roster().await.is_none());
    let again = community.nodes[0].ok(IpcRequest::VoiceLeave).await;
    assert_eq!(again["left"], false);

    // Teardown (lock) ends the session.
    community.nodes[1].ok(IpcRequest::Lock).await;
    assert!(community.nodes[1].host.voice().session().is_none());
}
