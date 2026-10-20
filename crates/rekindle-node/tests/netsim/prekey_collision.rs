//! Two first contacts on one slot (plan E2.4 Verify, D18's B5/B6): two
//! friends accept requests from the same requester, and both open their
//! session from the same one-time slot of her prekey record. Exactly one
//! init consumes it; the other is refused with a signed `PreKeyRejected`
//! and resent once, visibly (`FriendEvent::HandshakeRetrying`), and its
//! session opens too.
//!
//! The collision is forced, not hoped for: B2 picks uniformly among the
//! slots a record lists, so two readers of a full record collide one time
//! in a hundred. Here Rae's record lists **one** usable slot. Its other 99
//! subkeys are emptied by a writer holding the record's owner key — that
//! key is not her identity key, and any client may list fewer slots, so
//! the pick is only ever among what the record shows. Both accepters read
//! the record before her refill of the consumed slot is visible, because
//! Rae's DHT writes fail while her private route still delivers (she is
//! reachable, her writes are below consensus: `Fault::DhtOffline`), so the
//! refill is held. That fixes the outcome whatever order the pump delivers
//! the two inits in: the first one Rae opens consumes the slot, the second
//! names a consumed key. Its sender's retry reads the record again, sees
//! only the slot it already used (B2's `prekey_slots_used`), and opens
//! from the signed prekey and the last-resort key alone (PQXDH §2.5,
//! §3.3). When Rae's writes land again, so does her refill.

use rekindle_netsim::Fault;
use rekindle_types::dht_layout::prekeys::{slot_subkey, FIRST_SLOT, SLOT_COUNT};
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request, talk, LIMIT};

fn retrying(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeRetrying { peer_key } => Some(peer_key),
        _ => None,
    }
}

fn failed(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeFailed { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn two_first_contacts_on_one_slot_open_after_one_retry() {
    let (sim, mut nodes) = start(3, 31).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    let rae = key(&nodes[0]);
    request(&sim, &nodes[0], &nodes[1]).await;
    request(&sim, &nodes[0], &nodes[2]).await;

    // Rae's record lists one slot.
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("Rae's prekey record");
    let owner = sim.owner_keypair(&record).expect("its owner keypair");
    for index in 1..SLOT_COUNT {
        sim.forge_write(&record, slot_subkey(index), Vec::new(), &owner)
            .expect("an owner write");
    }
    let listed = sim
        .network_value(&record, FIRST_SLOT)
        .expect("the listed slot")
        .data;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });

    let (ann_events, bea_events) = (nodes[1].events().await, nodes[2].events().await);
    accept(&nodes[1], &nodes[0]).await;
    accept(&nodes[2], &nodes[0]).await;
    let (ann, bea) = (key(&nodes[1]), key(&nodes[2]));
    assert!(
        eventually(&sim, LIMIT, || async {
            is_accepted_friend(&nodes[0], &ann).await && is_accepted_friend(&nodes[0], &bea).await
        })
        .await,
        "both sessions reach Rae"
    );

    // One first contact went through at once, the other after one retry.
    let retries = (
        ann_events.count(&rae, retrying),
        bea_events.count(&rae, retrying),
    );
    assert!(
        retries == (1, 0) || retries == (0, 1),
        "exactly one retry: {retries:?}"
    );
    assert_eq!(ann_events.count(&rae, failed), 0);
    assert_eq!(bea_events.count(&rae, failed), 0);
    // The slot was consumed once and refilled in the same commit; the
    // retry consumed nothing.
    let status = nodes[0].prekey_status().await;
    assert_eq!((status.consumed_slots, status.fresh_slots), (1, 100));
    assert_eq!(
        sim.network_value(&record, FIRST_SLOT).map(|v| v.data),
        Some(listed.clone()),
        "the refill waits while Rae's writes fail"
    );

    sim.fault(Fault::Heal);
    sim.quiesce().await;
    assert_ne!(
        sim.network_value(&record, FIRST_SLOT).map(|v| v.data),
        Some(listed),
        "the refill lands once her writes do"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the collision").await;
    talk(&sim, &nodes[0], &nodes[2], "after the collision").await;
}
