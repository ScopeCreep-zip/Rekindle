//! Game servers (plan E3.1 H1): a server the creator adds reaches another
//! member's list over the mesh, and its removal takes it off again.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::game_server::GameServerInfo;

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

async fn servers(node: &SimNode, community: &str) -> Vec<GameServerInfo> {
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::GameServerList {
            community: community.into(),
        }))
        .await;
    serde_json::from_value(listed).expect("a server list")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_game_server_one_member_adds_is_seen_by_another() {
    let community = Community::of(2, 163).await;
    // Every join-time write settles.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();

    let added = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::GameServerAdd {
            community: key.clone(),
            game_id: "4100".into(),
            label: "Friday CTF".into(),
            address: "10.0.0.1:27015".into(),
        }))
        .await;
    let id = added.as_str().expect("the server's id").to_owned();
    assert!(
        servers(&community.nodes[0], &key)
            .await
            .iter()
            .any(|s| s.id == id),
        "the adder lists its server at once"
    );
    let seen = eventually(&community.sim, LIMIT, || async {
        servers(&community.nodes[1], &key)
            .await
            .iter()
            .any(|s| s.id == id && s.label == "Friday CTF" && s.address == "10.0.0.1:27015")
    })
    .await;
    assert!(seen, "the other member lists the server");

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::GameServerRemove {
            community: key.clone(),
            server_id: id.clone(),
        }))
        .await;
    let gone = eventually(&community.sim, LIMIT, || async {
        servers(&community.nodes[1], &key).await.is_empty()
    })
    .await;
    assert!(gone, "the removal reaches the other member");
}
