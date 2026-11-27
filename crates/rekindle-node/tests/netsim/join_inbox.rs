//! Durable join discovery through the inviter's watched inbox.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

async fn join(community: &Community, node: usize, invite: String) {
    community.nodes[node]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite },
        )))
        .await;
}

async fn lists_joiner(community: &Community, pseudonym: &str) -> bool {
    let response = community.nodes[0]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                anchor: community.anchor.clone(),
            },
        )))
        .await;
    response
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row["pseudonymKey"].as_str() == Some(pseudonym))
        })
}

async fn contains(community: &Community, pseudonym: &str) -> bool {
    eventually(&community.sim, LIMIT, || lists_joiner(community, pseudonym)).await
}

async fn holds_key(community: &Community, node: usize) -> bool {
    eventually(&community.sim, LIMIT, || async {
        let held = community.mek_list(node).await;
        !(held.is_null()
            || held.as_array().is_some_and(Vec::is_empty)
            || held.as_object().is_some_and(serde_json::Map::is_empty))
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn join_inbox_nobody_online() {
    let mut community = Community::create(4, 7930).await;
    let link = community.invite().await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    join(&community, 1, link).await;
    let joiner = community.pseudonym(1).await;
    // The DHT row needs a delivery attempt while its writer is unlocked.
    tokio::time::sleep(Duration::from_secs(180)).await;
    community.nodes[1].ok(IpcRequest::Lock).await;

    crate::devices::unlock(&community, 0).await;
    assert!(
        contains(&community, &joiner).await,
        "the inviter merges with the joiner still locked"
    );
    community.nodes[0].ok(IpcRequest::Lock).await;

    crate::devices::unlock(&community, 1).await;
    assert!(
        holds_key(&community, 1).await,
        "the key drop reaches the joiner with the inviter still locked"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn join_inbox_route_dead() {
    let mut community = Community::create(4, 7931).await;
    let link = community.invite().await;
    // The outgoing app route is unavailable. DHT publication and watches
    // continue to work through their separate transport.
    community.sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    join(&community, 1, link).await;
    assert!(contains(&community, &community.pseudonym(1).await).await);
    assert!(holds_key(&community, 1).await);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn join_inbox_concurrent() {
    let mut community = Community::create(6, 7932).await;
    let link = community.invite_for(5).await;
    for node in 1..6 {
        community.sim.fault(Fault::DropFrom {
            node,
            per_mille: 1000,
        });
    }
    futures::future::join_all((1..6).map(|node| join(&community, node, link.clone()))).await;
    for node in 1..6 {
        assert!(
            contains(&community, &community.pseudonym(node).await).await,
            "joiner {node} was not discovered"
        );
    }
}

async fn inbox_key(community: &Community, url: &str) -> String {
    use base64::Engine as _;
    use rekindle_types::invite::{InviteLink, InviteSecrets};
    let link = InviteLink::parse(url).expect("minted invite link");
    let deps = community.nodes[0].host.governance();
    let value = rekindle_governance_runtime::invite_secrets::fetch_invite_secrets(
        &deps,
        link.secrets_record_key.as_str(),
    )
    .await
    .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .unwrap();
    let secrets =
        rekindle_secrets::invite::decrypt_invite_secrets(link.invite_code.as_str(), &bytes)
            .unwrap();
    serde_json::from_slice::<InviteSecrets>(&secrets)
        .unwrap()
        .join_inbox
        .unwrap()
        .join_inbox
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn join_inbox_arm_race_uses_the_bootstrap_row() {
    let mut community = Community::create(4, 7933).await;
    let link = community.invite().await;
    let inbox = inbox_key(&community, &link).await;
    community.nodes[0].ok(IpcRequest::Lock).await;
    community.sim.fault(Fault::WatchDeath {
        node: 0,
        key: inbox.clone(),
    });
    community.sim.pause_watch_placement(0, &inbox);
    crate::devices::unlock(&community, 0).await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            community.sim.watch_requested(0, &inbox)
        })
        .await,
        "the watch request reached the local node"
    );
    // Let the initial catch-up read finish before another node writes.
    tokio::time::sleep(Duration::from_secs(10)).await;

    community.sim.fault(Fault::CallHangs {
        node: 0,
        hangs: true,
    });
    join(&community, 1, link).await;
    let member = community.pseudonym(1).await;
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(
        !lists_joiner(&community, &member).await,
        "remote watch is still delayed"
    );
    community.sim.fault(Fault::CallHangs {
        node: 0,
        hangs: false,
    });
    assert!(
        contains(&community, &member).await,
        "bootstrap retry delivered the join"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn join_inbox_retired_after_its_invite_is_revoked() {
    let community = Community::create(4, 7934).await;
    let invite = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.anchor.clone(),
                max_uses: 2,
                expires_seconds: None,
            },
        )))
        .await;
    let link = invite["url"].as_str().unwrap();
    let inbox = inbox_key(&community, link).await;
    assert!(community.sim.watch_requested(0, &inbox));
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteRevoke {
                community: community.anchor.clone(),
                invite_code: invite["inviteId"].as_str().unwrap().to_owned(),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            !community.sim.watch_requested(0, &inbox)
        })
        .await,
        "the revoke retires the inbox's session lease"
    );
}
