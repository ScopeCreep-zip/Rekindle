//! Real-app finding GDI1 (rekindle-38, `skins` spec): Bea asks Ann, Ann
//! accepts and at once creates a group with Bea. Her `GroupDmInvite`
//! (wrapped in the PreKeyInit her accept opened) can reach Bea before the
//! accept does, while Bea still holds her own request as `PendingOut`.
//! Bea's admission refuses it for now and answers "retry later", never
//! the NAK that ended Ann's row for good, so Ann's `friend_handshake`
//! stream sends it again after the accept lands: Bea holds the invite
//! once, joins once, and the group carries both ways.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::MESSAGE;
use rekindle_netsim::Fault;
use rekindle_types::dto::group_dm::GroupDmView;

use crate::group_dm::{reads, send, view};
use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, befriend, is_accepted_friend, key, request, LIMIT};

async fn copies_of(node: &SimNode, group_id: &str) -> usize {
    let groups: Vec<GroupDmView> =
        serde_json::from_value(node.ok(IpcRequest::GroupDmList).await).expect("the group list");
    groups.iter().filter(|g| g.group_id == group_id).count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_invite_sent_right_after_the_accept_lands_once_the_accept_does() {
    let (sim, mut nodes) = start(3, 131).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[2], &nodes[0]).await;
    request(&sim, &nodes[1], &nodes[0]).await;
    // Ann's first accept call is lost on the way (the question dropped,
    // her own deadline ends it, and the row waits for its retry); her
    // invite, committed at once after it, goes while the accept waits, so
    // it reaches Bea first, as in the real app.
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    accept(&nodes[0], &nodes[1]).await;
    tokio::time::sleep(MESSAGE.max * 2).await;
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDmCreate {
                name: "right after".to_string(),
                members: vec![key(&nodes[1]), key(&nodes[2])],
                origin_peer: None,
            })
            .await,
    )
    .expect("the group is created");
    tokio::time::sleep(MESSAGE.max * 2).await;
    assert!(
        !is_accepted_friend(&nodes[1], &key(&nodes[0])).await,
        "the invite reached Bea before the accept did"
    );
    for node in &nodes[1..] {
        assert!(
            eventually(&sim, LIMIT * 2, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
    }
    for node in &nodes[1..] {
        node.ok(IpcRequest::GroupDmAccept {
            group_id: gid.clone(),
        })
        .await;
    }
    send(&nodes[1], &gid, "from Bea").await;
    for node in [&nodes[0], &nodes[2]] {
        assert!(
            eventually(&sim, LIMIT * 2, || reads(node, &gid, "from Bea")).await,
            "node {} reads Bea's post",
            node.index
        );
    }
    send(&nodes[0], &gid, "from Ann").await;
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[1], &gid, "from Ann")).await,
        "Bea reads Ann's post"
    );
    for node in &nodes {
        assert_eq!(
            copies_of(node, &gid).await,
            1,
            "node {} joined once",
            node.index
        );
    }
}
