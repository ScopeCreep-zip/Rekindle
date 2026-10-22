//! Pins (plan E3.1 E3b): a message the creator pins reaches another
//! member's pin list over the mesh, and its unpin takes it off again.

use std::time::Duration;

use rekindle_ipc::protocol::PinRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::PinnedMessageView;

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);
const CHANNEL: &str = "pins-channel";
const MESSAGE: &str = "pinned-message";

async fn pins(node: &SimNode, community: &str) -> Vec<PinnedMessageView> {
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::Pins(
            PinRequest::ChannelPins {
                community: community.into(),
                channel_id: CHANNEL.into(),
            },
        )))
        .await;
    serde_json::from_value(listed).expect("a pin list")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn pin_converges() {
    let community = Community::of(2, 164).await;
    // Every join-time write settles.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Pins(
            PinRequest::MessagePin {
                community: key.clone(),
                channel_id: CHANNEL.into(),
                message_id: MESSAGE.into(),
            },
        )))
        .await;
    assert!(
        pins(&community.nodes[0], &key)
            .await
            .iter()
            .any(|p| p.message_id == MESSAGE),
        "the pinner lists its pin at once"
    );
    let seen = eventually(&community.sim, LIMIT, || async {
        pins(&community.nodes[1], &key)
            .await
            .iter()
            .any(|p| p.message_id == MESSAGE && p.channel_id == CHANNEL)
    })
    .await;
    assert!(seen, "the other member lists the pin");

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Pins(
            PinRequest::MessageUnpin {
                community: key.clone(),
                channel_id: CHANNEL.into(),
                message_id: MESSAGE.into(),
            },
        )))
        .await;
    let gone = eventually(&community.sim, LIMIT, || async {
        pins(&community.nodes[1], &key).await.is_empty()
    })
    .await;
    assert!(gone, "the unpin reaches the other member");
}
