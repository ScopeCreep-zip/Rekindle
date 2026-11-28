//! Group departures are local immediately, retried to non-friends, and
//! scoped to the admission they withdraw from.

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupDmDecline, GroupDmLeave};
use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::RouteClass;
use rekindle_transport::operations::dm::{handshake, send_to_peer_call};
use rekindle_transport::operations::group_dm::parse_group_id;

use crate::group_dm::{three_members, view};
use crate::harness::{eventually, start, SimNode};
use crate::pair::{key, LIMIT};

#[derive(Clone, Copy)]
enum Departure {
    Leave,
    Decline,
}

impl Departure {
    fn request(self, group_id: String) -> GroupDmRequest {
        match self {
            Self::Leave => GroupDmRequest::Leave { group_id },
            Self::Decline => GroupDmRequest::Decline { group_id },
        }
    }

    fn frame(self, group_id: [u8; 16], membership_generation: [u8; 16]) -> DirectBody {
        match self {
            Self::Leave => DirectBody::GroupDmLeave(GroupDmLeave {
                group_id,
                membership_generation,
                epoch: 0,
            }),
            Self::Decline => DirectBody::GroupDmDecline(GroupDmDecline {
                group_id,
                membership_generation,
            }),
        }
    }
}

async fn pending(node: &SimNode, gid: &str) -> bool {
    view(node, gid).await.is_some_and(|v| v.pending)
}

async fn accept(node: &SimNode, gid: &str) {
    node.ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
        group_id: gid.to_owned(),
    }))
    .await;
}

async fn setup(kind: Departure) -> (Netsim, Vec<SimNode>, String) {
    let (sim, mut nodes) = start(3, 281).await;
    // This also establishes Cat and Bea's pairwise session, so both
    // scenarios exercise a ratchet departure over the handshake stream.
    let mut gid = three_members(&sim, &mut nodes).await;
    if matches!(kind, Departure::Decline) {
        gid = serde_json::from_value(
            nodes[0]
                .ok(IpcRequest::GroupDm(GroupDmRequest::Create {
                    name: "another room".to_owned(),
                    members: vec![key(&nodes[1]), key(&nodes[2])],
                    origin_peer: None,
                }))
                .await,
        )
        .expect("the second group id");
        for node in &nodes[1..] {
            assert!(eventually(&sim, LIMIT, || pending(node, &gid)).await);
        }
        accept(&nodes[1], &gid).await;
    }
    (sim, nodes, gid)
}

async fn roster_has(node: &SimNode, gid: &str, member: &str) -> bool {
    view(node, gid)
        .await
        .expect("the remaining member still holds the room")
        .members
        .iter()
        .any(|m| m.key == member)
}

async fn withdrawn(sim: &Netsim, nodes: &[SimNode], gid: &str) {
    let cat = key(&nodes[2]);
    for node in &nodes[..2] {
        assert!(
            eventually(sim, LIMIT * 4, || async {
                !roster_has(node, gid, &cat).await
            })
            .await,
            "node {} removed Cat from its roster",
            node.index
        );
    }
}

async fn handshake_attempts(node: &SimNode, peer: &str, kind: &str) -> u32 {
    let owner = key(node);
    let stream = format!("friend_handshake:{peer}");
    let db = node
        .host
        .identity_stores()
        .db
        .current()
        .expect("the outbox DB");
    db.call(move |conn| rekindle_db::repo::outbox::prefix_rows(conn, &owner, &stream))
        .await
        .expect("the handshake rows")
        .into_iter()
        .filter(|row| row.client_id.starts_with(kind))
        .map(|row| row.attempts)
        .max()
        .unwrap_or(0)
}

async fn retry_after_route_death(kind: Departure) {
    let (sim, nodes, gid) = setup(kind).await;
    let bea = key(&nodes[1]);
    let cat = key(&nodes[2]);
    let prefix = handshake::client_prefix(
        kind.frame(parse_group_id(&gid).expect("a group id"), [0; 16])
            .type_id(),
    );
    // The published route acts alive to its owner while losing traffic
    // from outside. Cat's first departure attempt goes into that route.
    sim.fault(Fault::OutsideDead {
        node: 1,
        class: RouteClass::General,
    });
    nodes[2]
        .ok(IpcRequest::GroupDm(kind.request(gid.clone())))
        .await;
    assert!(view(&nodes[2], &gid).await.is_none(), "Cat leaves locally");
    let retried = eventually(&sim, LIMIT, || async {
        handshake_attempts(&nodes[2], &bea, &prefix).await >= 2
    })
    .await;
    assert!(
        roster_has(&nodes[1], &gid, &cat).await,
        "Bea missed the departure"
    );
    // The owner now learns of the dead route and publishes a replacement.
    sim.fault(Fault::RouteDeath { node: 1 });
    withdrawn(&sim, &nodes, &gid).await;
    assert!(
        retried,
        "the departure remained queued after two lost attempts"
    );
}

async fn ignores_previous_admission(kind: Departure) {
    let (sim, nodes, gid) = setup(kind).await;
    let id = parse_group_id(&gid).expect("a group id");
    let cat = key(&nodes[2]);
    let ann = key(&nodes[0]);
    let host = nodes[0].host.friend_host();
    let deps = host.group_dm().expect("Ann's group store");
    let initial = deps.group(&id).await.expect("read group").expect("group");
    let generation = initial
        .roster
        .member(&cat)
        .expect("Cat on initial roster")
        .membership_generation;
    let old = kind.frame(id, generation);

    nodes[2]
        .ok(IpcRequest::GroupDm(kind.request(gid.clone())))
        .await;
    withdrawn(&sim, &nodes, &gid).await;
    nodes[0]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Invite {
            group_id: gid.clone(),
            members: vec![cat.clone()],
        }))
        .await;
    assert!(eventually(&sim, LIMIT, || pending(&nodes[2], &gid)).await);
    accept(&nodes[2], &gid).await;
    assert!(eventually(&sim, LIMIT, || roster_has(&nodes[1], &gid, &cat)).await);

    let added = deps.group(&id).await.expect("read group").expect("group");
    assert_ne!(
        added
            .roster
            .member(&cat)
            .expect("Cat on grown roster")
            .membership_generation,
        generation
    );
    // Re-encrypt the old body so the Double Ratchet accepts it and the
    // lifecycle sees it. Replaying identical ciphertext only tests the ratchet.
    let cat_host = nodes[2].host.friend_host();
    let receipt = send_to_peer_call(cat_host.as_ref(), &ann, &old)
        .await
        .expect("Ann receipts the old-generation departure");
    assert!(receipt.is_none(), "Ann sends an empty receipt");
    assert!(
        !eventually(&sim, LIMIT, || async {
            !roster_has(&nodes[0], &gid, &cat).await
        })
        .await,
        "the departure from the previous admission must leave Cat on Ann's roster"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_dm_leave_retries_after_route_death() {
    retry_after_route_death(Departure::Leave).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_dm_decline_retries_after_route_death() {
    retry_after_route_death(Departure::Decline).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_dm_late_leave_preserves_readded_member() {
    ignores_previous_admission(Departure::Leave).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_dm_late_decline_preserves_readded_member() {
    ignores_previous_admission(Departure::Decline).await;
}
