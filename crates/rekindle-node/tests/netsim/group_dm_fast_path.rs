//! Real-app finding GD1 (rekindle-38, layout-friends fleet): Ann befriends
//! Bea and Cat, who are not friends, and creates a group. Bea accepts
//! first, so her `GroupDmAccept`, the PreKeyInit opening her pairwise
//! session with Cat (plan E2.6), reaches Cat before Cat accepts. Cat
//! refuses it only for now (retry later, nothing opened), and Bea's hints
//! wait for a session Cat answered, so no hint rides an init Cat never
//! stored ("no session with the peer decrypts the message"). With every
//! member's group chain held back, each member's post reaches the other
//! two over the direct fast path alone.

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::latency::MESSAGE;
use rekindle_netsim::{Fault, Writes};

use crate::group_dm::{own_chain, reads, send, view};
use crate::harness::{eventually, start};
use crate::pair::{befriend, key, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_who_is_no_friend_gets_each_post_over_the_fast_path() {
    let (sim, mut nodes) = start(3, 233).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    befriend(&sim, &nodes[2], &nodes[0]).await;
    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDm(GroupDmRequest::Create {
                name: "fast path".to_string(),
                members: vec![key(&nodes[1]), key(&nodes[2])],
                origin_peer: None,
            }))
            .await,
    )
    .expect("the group is created");
    for node in &nodes[1..] {
        assert!(
            eventually(&sim, LIMIT, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
    }
    // Bea accepts first: her accept to Cat arrives while Cat's invite is
    // unanswered.
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    tokio::time::sleep(MESSAGE.max * 4).await;
    assert!(
        !crate::logs::logged(2, &["we have not answered the group's invite"]).is_empty(),
        "Bea's accept reached Cat before Cat accepted"
    );
    nodes[2]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    // Every member's chain is held back: a post arrives only as a hint.
    for node in &nodes {
        let chain = own_chain(node, &gid).expect("the member's group chain");
        sim.fault(Fault::WriteFails {
            node: node.index,
            of: Writes::Record(chain),
        });
    }
    for (i, author) in ["Ann", "Bea", "Cat"].iter().enumerate() {
        let body = format!("{author} on the fast path");
        send(&nodes[i], &gid, &body).await;
        for reader in nodes.iter().filter(|n| n.index != i) {
            assert!(
                eventually(&sim, LIMIT * 2, || reads(reader, &gid, &body)).await,
                "node {} reads {body:?} with every chain held back",
                reader.index
            );
        }
    }
    let dropped: Vec<String> = crate::logs::warned("ratchet step failed")
        .into_iter()
        .filter(|l| l.contains("GroupContent"))
        .collect();
    assert!(dropped.is_empty(), "no hint was dropped: {dropped:?}");
}
