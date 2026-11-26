//! A group member's first contact to a member who is not its friend has a
//! durable copy (plan decision 67 item 3, part 3b; design
//! `design-decision67-rv2c-2026-11-25.md` §3, F2). Bea's PreKeyInit
//! carrying her `GroupDmAccept` to Cat goes by `app_call` and is written to
//! her row of the group's rendezvous record. With every call to Cat lost
//! (`Fault::CallHangs`), Cat still opens it from her column, reads Bea's
//! posts, and answers; Bea's first-contact row settles `delivered` only on
//! that answer (Cat's frame under the session), never on her write. Locked
//! while Bea writes, Cat finds the entry when her column is held. With both
//! paths up, the copy heard twice is applied once.

use std::time::Duration;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use crate::group_dm::{reads, send, view};
use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{befriend, key, LIMIT};

/// Ann (node 0) befriends Bea and Cat, who are not friends, and creates
/// the group; both hold the invite. The group id.
async fn invited(sim: &Netsim, nodes: &mut [SimNode]) -> String {
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(sim, &nodes[1], &nodes[0]).await;
    befriend(sim, &nodes[2], &nodes[0]).await;
    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDm(GroupDmRequest::Create {
                name: "raid night".to_string(),
                members: vec![key(&nodes[1]), key(&nodes[2])],
                origin_peer: None,
            }))
            .await,
    )
    .expect("the group is created");
    for node in &nodes[1..] {
        assert!(
            eventually(sim, LIMIT, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
    }
    gid
}

async fn accept(node: &SimNode, gid: &str) {
    node.ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
        group_id: gid.to_string(),
    }))
    .await;
}

/// Whether `node` holds an unsettled first-contact row to `peer` (a
/// settled one reads `delivered` or has left the outbox).
async fn first_contact_open(node: &SimNode, peer: &str) -> bool {
    let owner = key(node);
    let stream = format!("friend_handshake:{peer}");
    let Ok(db) = node.host.identity_stores().db.current() else {
        return false;
    };
    db.call(move |conn| rekindle_db::repo::outbox::prefix_rows(conn, &owner, "friend_handshake:"))
        .await
        .is_ok_and(|rows| {
            rows.iter().any(|r| {
                r.stream == stream
                    && r.client_id.starts_with("PreKeyInit:")
                    && !matches!(r.state, SendState::Delivered | SendState::Failed)
            })
        })
}

/// With every call to Cat lost, Bea's first contact to Cat stays unsettled
/// though its rendezvous write landed: the write never settles it (F2).
async fn unsettled_while_calls_hang(sim: &Netsim, nodes: &[SimNode]) {
    let cat = key(&nodes[2]);
    assert!(
        eventually(sim, LIMIT, || first_contact_open(&nodes[1], &cat)).await,
        "Bea's first contact to Cat is a first-contact row"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(
        first_contact_open(&nodes[1], &cat).await,
        "the rendezvous write does not settle it"
    );
}

/// Bea's posts reach Cat, and Cat's reach Bea, whose first contact to Cat
/// settles on Cat's answer.
async fn talk_and_settle(sim: &Netsim, nodes: &[SimNode], gid: &str) {
    send(&nodes[1], gid, "Bea says hi").await;
    assert!(
        eventually(sim, LIMIT * 2, || reads(&nodes[2], gid, "Bea says hi")).await,
        "Cat holds Bea's key and chain from the rendezvous copy"
    );
    send(&nodes[2], gid, "Cat says hi").await;
    assert!(
        eventually(sim, LIMIT * 2, || reads(&nodes[1], gid, "Cat says hi")).await,
        "Bea reads Cat's answer"
    );
    let cat = key(&nodes[2]);
    assert!(
        eventually(sim, LIMIT, || async {
            !first_contact_open(&nodes[1], &cat).await
        })
        .await,
        "Bea's first contact to Cat settles on Cat's answer"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_member_first_contact_durable() {
    let (sim, mut nodes) = start(3, 223).await;
    let gid = invited(&sim, &mut nodes).await;
    sim.fault(Fault::CallHangs {
        node: 2,
        hangs: true,
    });
    accept(&nodes[1], &gid).await;
    unsettled_while_calls_hang(&sim, &nodes).await;
    accept(&nodes[2], &gid).await;
    talk_and_settle(&sim, &nodes, &gid).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_member_first_contact_lands_after_unlock() {
    let (sim, mut nodes) = start(3, 227).await;
    let gid = invited(&sim, &mut nodes).await;
    sim.fault(Fault::CallHangs {
        node: 2,
        hangs: true,
    });
    nodes[2].ok(IpcRequest::Lock).await;
    accept(&nodes[1], &gid).await;
    unsettled_while_calls_hang(&sim, &nodes).await;
    nodes[2]
        .ok(IpcRequest::Unlock {
            identity: key(&nodes[2]),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    accept(&nodes[2], &gid).await;
    talk_and_settle(&sim, &nodes, &gid).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_first_contact_heard_twice_is_applied_once() {
    let (sim, mut nodes) = start(3, 229).await;
    let gid = invited(&sim, &mut nodes).await;
    let events = nodes[2].events().await;
    // Cat, still invited, opens Bea's call copy; her accept holds her
    // column, whose read finds the same init again.
    accept(&nodes[1], &gid).await;
    accept(&nodes[2], &gid).await;
    talk_and_settle(&sim, &nodes, &gid).await;
    // Long enough for the rendezvous copy to land after the call's.
    tokio::time::sleep(Duration::from_secs(120)).await;
    let bea = key(&nodes[1]);
    let joined = events.matching(|e| {
        matches!(e, SubscriptionEvent::GroupDm(GroupDmEvent::Joined { group_id, member })
            if *group_id == gid && member.public_key == bea)
    });
    assert_eq!(joined, 1, "Bea's key applied once at Cat");
}
