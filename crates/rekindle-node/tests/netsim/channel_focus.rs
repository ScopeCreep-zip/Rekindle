//! Active-channel dispatch and the local member's served location.

use rekindle_ipc::protocol::{
    ChannelMessageRequest, CommunityRequest, IpcRequest, LifecycleRequest,
};
use rekindle_types::dto::community::CommunityMemberView;
use rekindle_types::presence::{SessionLocation, SessionStatus};

use crate::harness::{eventually, Community};
use crate::pair::LIMIT;

async fn focus(c: &Community, channel: Option<String>, kind: &str) {
    c.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::ChannelSetActive {
                community: c.anchor.clone(),
                channel,
                kind: kind.into(),
            },
        )))
        .await;
}

async fn member(c: &Community, observer: usize, pseudonym: &str) -> Option<CommunityMemberView> {
    let value = c.nodes[observer]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                anchor: c.anchor.clone(),
            },
        )))
        .await;
    let members: Vec<CommunityMemberView> = serde_json::from_value(value).expect("member list");
    members.into_iter().find(|m| m.pseudonym_key == pseudonym)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn channel_focus_serves_own_location_and_keeps_it_private_by_default() {
    let c = Community::of(2, 9302).await;
    let me = c.pseudonym(0).await;
    let channel_id = c.general_id(0).await;

    focus(&c, Some(channel_id.clone()), "text").await;
    assert_eq!(
        member(&c, 0, &me).await.expect("own member").location,
        Some(SessionLocation::Text { channel_id }),
    );
    c.nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Busy,
        })
        .await;
    assert!(
        eventually(&c.sim, LIMIT, || async {
            member(&c, 1, &me).await.is_some_and(|m| m.status == "busy")
        })
        .await,
        "the peer sees the status sent after focus"
    );
    assert_eq!(
        member(&c, 1, &me).await.expect("peer member").location,
        None,
        "the peer's member list omits location with sharing off by default"
    );

    focus(&c, None, "text").await;
    assert_eq!(
        member(&c, 0, &me).await.expect("own member").location,
        None,
        "clearing focus removes our own location"
    );
}
