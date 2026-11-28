//! Group DM accepts (plan E2.6, live finding G1): a member's chain is on
//! the network before its key leaves, and content that reaches a member
//! before the poster's accept is shown once the accept lands.

use std::time::Duration;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::{Fault, Netsim};

use crate::group_dm::{own_chain, reads, send, view};
use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, LIMIT};

/// `node`'s own group chain is on the network: its `HEAD` holds the empty
/// head (nothing appended), so a reader handed the key finds the record.
fn chain_on_network(sim: &Netsim, node: &SimNode, group_id: &str) -> bool {
    use rekindle_types::dht_layout::dm_participant;
    own_chain(node, group_id)
        .and_then(|k| sim.network_get(&k, dm_participant::HEAD))
        .is_some_and(|head| {
            rekindle_codec::capnp_codec::chain::decode_chain_head(&head.data)
                .is_ok_and(|h| h.is_none())
        })
}

/// G1 fault 3: a member's chain record is on the network before the group
/// (the creator's) or the accept (a member's) names it. Veilid creates a
/// record locally and writes nothing (0.5.8 `create_record.rs`), so a
/// reader opening a chain nobody wrote got `KeyNotFound` and waited on
/// retry backoff; the empty `HEAD` is written at creation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_chain_is_on_the_network_before_its_key_leaves() {
    let (sim, mut nodes) = start(3, 96).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    befriend(&sim, &nodes[2], &nodes[0]).await;
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
    assert!(
        chain_on_network(&sim, &nodes[0], &gid),
        "the creator's chain is on the network when the group names it"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            view(&nodes[1], &gid).await.is_some_and(|v| v.pending)
        })
        .await,
        "Bea holds the invite"
    );
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    assert!(
        chain_on_network(&sim, &nodes[1], &gid),
        "Bea's chain is on the network when her accept names it"
    );
}

/// G1 fault 2: Bea's post reaches Cat while Bea's accept to Cat is stuck
/// (every call to Cat hangs; Bea's app messages still arrive), so Cat
/// holds content from a member whose accept it has not seen. Once the
/// accept lands, Cat reads the post.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_post_that_arrives_before_the_posters_accept_is_shown_once_it_lands() {
    let (sim, mut nodes) = start(3, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    befriend(&sim, &nodes[2], &nodes[0]).await;
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
            eventually(&sim, LIMIT, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
    }
    nodes[2]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    send(&nodes[2], &gid, "from Cat").await;
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[0], &gid, "from Cat")).await,
        "Ann reads Cat"
    );
    sim.fault(Fault::CallHangs {
        node: 2,
        hangs: true,
    });
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    send(&nodes[1], &gid, "from Bea").await;
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[0], &gid, "from Bea")).await,
        "Ann reads Bea"
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    sim.fault(Fault::CallHangs {
        node: 2,
        hangs: false,
    });
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[2], &gid, "from Bea")).await,
        "Cat reads Bea once her accept lands"
    );
}

/// Linux run 2: the creator's `GroupDmInvite` went once per invitee as an
/// unreceipted `app_message`, so a lost one was never sent again. Nothing
/// Ann sends arrives for 30 s after she creates the group; both invitees
/// still come to hold the invite, and Bea becomes a member.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_invite_is_sent_again_until_received() {
    let (sim, mut nodes) = start(3, 98).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    befriend(&sim, &nodes[2], &nodes[0]).await;
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
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
    tokio::time::sleep(Duration::from_secs(30)).await;
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
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
    nodes[1]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    send(&nodes[1], &gid, "from Bea").await;
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[0], &gid, "from Bea")).await,
        "Ann reads Bea's post"
    );
}

/// Finding GA1 (round 15): Bea's accept goes to Cat, who is not her
/// friend, over a session opened from Cat's prekey record. Bea's DHT goes
/// offline the moment her chain is stored, so Cat's record cannot be read
/// when the accept is framed: the accept is still committed (no "cannot
/// be opened yet" refusal), and Cat reads Bea once the record reads.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_waits_for_a_members_unreadable_prekeys() {
    let (sim, mut nodes) = start(3, 99).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[1], &nodes[0]).await;
    befriend(&sim, &nodes[2], &nodes[0]).await;
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
            eventually(&sim, LIMIT, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
    }
    nodes[2]
        .ok(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        }))
        .await;
    let offline_once_chained = async {
        while own_chain(&nodes[1], &gid).is_none() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        sim.fault(Fault::DhtOffline {
            node: 1,
            offline: true,
        });
    };
    let (accepted, ()) = tokio::join!(
        nodes[1].call(IpcRequest::GroupDm(GroupDmRequest::Accept {
            group_id: gid.clone(),
        })),
        offline_once_chained
    );
    let refused = accepted.err().map(|r| (r.code, r.message));
    assert_eq!(
        refused, None,
        "the accept is committed, never refused for not yet"
    );
    assert!(
        view(&nodes[1], &gid).await.is_some_and(|v| !v.pending),
        "Bea is a member at once"
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    sim.fault(Fault::Heal);
    send(&nodes[1], &gid, "from Bea").await;
    assert!(
        eventually(&sim, LIMIT * 2, || reads(&nodes[2], &gid, "from Bea")).await,
        "Cat reads Bea once her waiting accept lands"
    );
}
