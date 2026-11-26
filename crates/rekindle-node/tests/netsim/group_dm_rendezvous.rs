//! A group's rendezvous record (plan decision 67 item 3, design
//! `design-decision67-rv2c-2026-11-25.md` §3): the creator's minted record
//! holds `OPENED` at its owner subkey; each member watches exactly its own
//! column of it from the create or the accept, again after an unlock, and
//! no more once it leaves; the creator's writers leave its vault with it.

use std::collections::BTreeSet;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};
use rekindle_netsim::Netsim;
use rekindle_types::dht_layout::group_rendezvous as layout;

use crate::group_dm::view;
use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{befriend, key, LIMIT};

/// The creator's minted writers' record, while its vault holds them.
fn minted(node: &SimNode, group_id: &str) -> Option<String> {
    let stores = node.host.identity_stores();
    let guard = stores.vault.lock();
    let vault = guard.as_ref().expect("an unlocked vault");
    rekindle_vault::typed::group_rendezvous::load(vault, group_id)
        .expect("the vault read")
        .map(|entry| entry.record_key)
}

/// `node`'s stored slot in the group's record.
async fn slot(node: &SimNode, group_id: &str) -> u8 {
    let owner = key(node);
    let gid = group_id.to_string();
    let db = node
        .host
        .identity_stores()
        .db
        .current()
        .expect("a database");
    db.call(move |conn| rekindle_db::repo::group_dm_rendezvous::get(conn, &owner, &gid))
        .await
        .expect("the place read")
        .expect("a stored place")
        .slot
}

/// Whether `node` watches exactly `want` of `record`, within the limit.
async fn watches(sim: &Netsim, node: usize, record: &str, want: &BTreeSet<u32>) -> bool {
    eventually(sim, LIMIT, || async {
        &sim.watched_subkeys(node, record) == want
    })
    .await
}

fn column(slot: u8) -> BTreeSet<u32> {
    layout::column(slot).into_iter().collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn each_member_watches_its_own_column_from_accept_to_leave() {
    let (sim, mut nodes) = start(3, 211).await;
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
    let record = minted(&nodes[0], &gid).expect("the creator keeps its writers");
    assert_eq!(
        sim.network_get(&record, layout::OWNER_SUBKEY)
            .map(|v| v.data),
        Some(layout::OPENED.to_vec()),
        "the minted record holds OPENED at its owner subkey"
    );
    let ann = column(slot(&nodes[0], &gid).await);
    assert!(
        watches(&sim, 0, &record, &ann).await,
        "Ann watches her column"
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
    let bea = column(slot(&nodes[1], &gid).await);
    assert_ne!(bea, ann, "Bea's column is her own");
    assert!(
        watches(&sim, 1, &record, &bea).await,
        "Bea watches her column after accept"
    );

    // Locked, the watch goes with the pool; the unlock holds it again.
    nodes[1].ok(IpcRequest::Lock).await;
    assert!(
        watches(&sim, 1, &record, &BTreeSet::new()).await,
        "no watch while locked"
    );
    nodes[1]
        .ok(IpcRequest::Unlock {
            identity: key(&nodes[1]),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert!(
        watches(&sim, 1, &record, &bea).await,
        "Bea's column is watched again at unlock"
    );

    // Bea leaves: her watch is released.
    let _ = nodes[1]
        .call(IpcRequest::GroupDm(GroupDmRequest::Leave {
            group_id: gid.clone(),
        }))
        .await;
    assert!(
        watches(&sim, 1, &record, &BTreeSet::new()).await,
        "Bea's watch is released at leave"
    );

    // Ann (the creator) leaves: her watch and her writers go.
    let _ = nodes[0]
        .call(IpcRequest::GroupDm(GroupDmRequest::Leave {
            group_id: gid.clone(),
        }))
        .await;
    assert!(
        watches(&sim, 0, &record, &BTreeSet::new()).await,
        "Ann's watch is released at leave"
    );
    assert_eq!(
        minted(&nodes[0], &gid),
        None,
        "the creator's writers left its vault"
    );
}
