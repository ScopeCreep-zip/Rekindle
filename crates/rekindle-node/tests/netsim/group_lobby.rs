//! A group DM's game lobby (plan E7.2 G6, SK2-G §5): the host opens it for
//! its shown game, the others join over their group chains, the host
//! launches it on its rich-presence server and every member's view shows
//! `Launched{server}`, then the host closes it for everyone. A member who
//! does not host it cannot launch it.

use rekindle_ipc::protocol::{IpcRequest, LobbyRequest};
use rekindle_types::dto::group_lobby::{LobbyStatus, LobbyView};

use crate::group_dm::three_members;
use rekindle_netsim::Netsim;

use crate::harness::{eventually, start, SimNode};
use crate::pair::LIMIT;

const SERVER: &str = "203.0.113.7:27015";

async fn lobby(node: &SimNode, group_id: &str) -> Option<LobbyView> {
    let value = node
        .ok(IpcRequest::Lobby(LobbyRequest::Get {
            group_id: group_id.to_owned(),
        }))
        .await;
    serde_json::from_value(value).expect("a LobbyView or null")
}

async fn all_see(
    sim: &Netsim,
    nodes: &[SimNode],
    gid: &str,
    what: &str,
    pred: impl Fn(&LobbyView) -> bool,
) {
    for node in nodes {
        assert!(
            eventually(sim, LIMIT, || async {
                lobby(node, gid).await.is_some_and(|l| pred(&l))
            })
            .await,
            "node {} sees the lobby {what}",
            node.index
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lobby_is_opened_joined_launched_and_closed_for_every_member() {
    let (sim, mut nodes) = start(3, 161).await;
    let gid = three_members(&sim, &mut nodes).await;
    nodes[0]
        .ok(IpcRequest::GamePresenceSet {
            game_name: "Team Fortress 2".into(),
            game_id: Some(4181),
            elapsed_seconds: 0,
            server_address: Some(SERVER.into()),
        })
        .await;
    nodes[0]
        .ok(IpcRequest::Lobby(LobbyRequest::Host {
            group_id: gid.clone(),
        }))
        .await;
    all_see(&sim, &nodes, &gid, "open", |l| l.state == LobbyStatus::Open).await;
    let lobby_id = lobby(&nodes[0], &gid).await.unwrap().lobby_id;
    for node in &nodes[1..] {
        node.ok(IpcRequest::Lobby(LobbyRequest::Join {
            group_id: gid.clone(),
            lobby_id: lobby_id.clone(),
        }))
        .await;
    }
    all_see(&sim, &nodes, &gid, "with both members", |l| {
        l.members.len() == 2
    })
    .await;

    let refused = nodes[1]
        .call(IpcRequest::Lobby(LobbyRequest::Launch {
            group_id: gid.clone(),
            lobby_id: lobby_id.clone(),
            server: None,
        }))
        .await;
    assert!(
        refused.is_err(),
        "a member launched a lobby it does not host"
    );

    nodes[0]
        .ok(IpcRequest::Lobby(LobbyRequest::Launch {
            group_id: gid.clone(),
            lobby_id: lobby_id.clone(),
            server: None,
        }))
        .await;
    all_see(
        &sim,
        &nodes,
        &gid,
        "launched on the host's server",
        |l| matches!(&l.state, LobbyStatus::Launched { server } if server.to_string() == SERVER),
    )
    .await;

    nodes[0]
        .ok(IpcRequest::Lobby(LobbyRequest::Close {
            group_id: gid.clone(),
            lobby_id,
        }))
        .await;
    all_see(&sim, &nodes, &gid, "closed", |l| {
        l.state == LobbyStatus::Closed
    })
    .await;
}
