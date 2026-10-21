//! The 1:1 steps the friend scenarios share: a request through an invite,
//! its acceptance, and the reads that show what each side holds.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Netsim;

use crate::harness::{eventually, SimNode};

/// How long a 1:1 step may take in simulated time.
pub const LIMIT: Duration = Duration::from_secs(180);

/// The node's unlocked identity.
pub fn key(node: &SimNode) -> String {
    node.public_key.clone().expect("an unlocked identity")
}

pub async fn friends(node: &SimNode) -> Vec<rekindle_types::dto::friends::Friend> {
    serde_json::from_value(node.ok(IpcRequest::FriendList).await).expect("the friend list")
}

pub async fn is_accepted_friend(node: &SimNode, key: &str) -> bool {
    friends(node).await.iter().any(|f| {
        f.public_key == key
            && f.friendship_state == rekindle_types::friend::FriendshipState::Accepted
    })
}

pub async fn has_request_from(node: &SimNode, key: &str) -> bool {
    let requests: Vec<rekindle_types::dto::friends::PendingRequest> =
        serde_json::from_value(node.ok(IpcRequest::FriendRequests).await).expect("the requests");
    requests.iter().any(|r| r.public_key == key)
}

/// Whether `node` holds `body` from `peer`.
pub async fn holds_message(node: &SimNode, peer: &str, body: &str) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 50,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history.iter().any(|m| m.body == body && !m.is_own)
}

pub async fn send(node: &SimNode, peer: &str, body: &str) {
    node.ok(IpcRequest::DmSend {
        peer_key: peer.to_string(),
        body: body.to_string(),
        reply_to: None,
    })
    .await;
}

/// `requester` asks `target` through an invite of `target`'s; returns once
/// `target` holds the request.
pub async fn request(sim: &Netsim, requester: &SimNode, target: &SimNode) {
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(target.ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    requester
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let from = key(requester);
    assert!(
        eventually(sim, LIMIT, || has_request_from(target, &from)).await,
        "node {} holds node {}'s request",
        target.index,
        requester.index
    );
}

/// `accepter` accepts `requester`'s request (its review committed).
pub async fn accept(accepter: &SimNode, requester: &SimNode) {
    accepter
        .ok(IpcRequest::FriendAccept {
            public_key: key(requester),
        })
        .await;
}

/// `requester` asks and `accepter` accepts; both hold the friendship.
pub async fn befriend(sim: &Netsim, requester: &SimNode, accepter: &SimNode) {
    request(sim, requester, accepter).await;
    accept(accepter, requester).await;
    let theirs = key(accepter);
    assert!(eventually(sim, LIMIT, || is_accepted_friend(requester, &theirs)).await);
    assert!(is_accepted_friend(accepter, &key(requester)).await);
}

/// A message each way between `a` and `b`, each read by the other.
pub async fn talk(sim: &Netsim, a: &SimNode, b: &SimNode, tag: &str) {
    for (from, to) in [(a, b), (b, a)] {
        let body = format!("{tag}: {} to {}", from.index, to.index);
        send(from, &key(to), &body).await;
        let sender = key(from);
        assert!(
            eventually(sim, LIMIT, || holds_message(to, &sender, &body)).await,
            "node {} reads {body:?}",
            to.index
        );
    }
}

/// The records whose watched changes reached `reader` that carry no chain
/// retire mark yet: the writer's chain records among them (a record's
/// `RETIRE_THROUGH` subkey is empty until its first retire). A change comes
/// from the replica that held the watch (Veilid's ValueChanged is the
/// replica's RPC), not from the writer's node, or from the reader's own
/// catch-up inspection (`ChangeInspect`, plan E2.B S6).
pub fn unretired(sim: &rekindle_netsim::Netsim, reader: usize) -> Vec<String> {
    use rekindle_types::dht_layout::dm_participant;
    let prefix = format!("{reader}<-");
    let mut keys: Vec<String> = sim
        .trace()
        .of(reader)
        .filter_map(|e| {
            let rest = e.strip_prefix(prefix.as_str())?;
            let (_, key) = rest
                .split_once(" ValueChange ")
                .or_else(|| rest.split_once(" ChangeInspect "))?;
            Some(key.to_string())
        })
        .filter(|k| sim.network_get(k, dm_participant::RETIRE_THROUGH).is_none())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Whether one of `keys` now carries a retire mark: the writer's chain is
/// retired on the network.
pub fn retired(sim: &rekindle_netsim::Netsim, keys: &[String]) -> bool {
    use rekindle_types::dht_layout::dm_participant;
    keys.iter()
        .any(|k| sim.network_get(k, dm_participant::RETIRE_THROUGH).is_some())
}
