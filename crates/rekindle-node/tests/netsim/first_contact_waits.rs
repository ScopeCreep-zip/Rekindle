//! A first contact never meets a prekey record without its signed prekey
//! (live finding L-FR3, plan E2.4): the record is named by the profile only
//! once its head is on the network, and an accept whose bundle is not
//! readable yet waits as a `friend_handshake` row instead of failing.

use std::time::Duration;

use rekindle_netsim::{Fault, Records, Writes};
use rekindle_types::dht_layout::prekeys::{HEAD, SLOT_COUNT};
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request, LIMIT};

fn failed(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::HandshakeFailed { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

/// Ann's head misses consensus at creation: her profile names her record
/// only once a re-push landed it, so the record is complete when named.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_record_is_named_only_with_its_head() {
    let (sim, mut nodes) = start(2, 61).await;
    let record_count = u16::try_from(SLOT_COUNT + 1).expect("a DFLT count");
    sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Subkey {
            subkey: HEAD,
            record: Some(Records::CreatedDflt(record_count)),
        },
    });
    let heal = async {
        tokio::time::sleep(Duration::from_secs(25)).await;
        sim.fault(Fault::Heal);
    };
    let ((), ()) = tokio::join!(nodes[0].unlock_new_identity("Ann"), heal);
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("Ann's prekey record");
    assert!(
        sim.network_get(&record, HEAD).is_some(),
        "the record is named with its signed prekey on the network"
    );
    nodes[1].unlock_new_identity("Bea").await;
    let ann = key(&nodes[0]);
    request(&sim, &nodes[1], &nodes[0]).await;
    accept(&nodes[0], &nodes[1]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &ann)).await,
        "Bea's request is accepted"
    );
}

/// Ann accepts while she cannot read Rae's prekey record: the accept waits
/// (no failure shown) and lands once the record reads.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_waits_for_a_readable_record() {
    let (sim, mut nodes) = start(2, 62).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[0], &nodes[1]).await;
    let ann_events = nodes[1].events().await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    accept(&nodes[1], &nodes[0]).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert!(!is_accepted_friend(&nodes[0], &ann).await);
    sim.fault(Fault::Heal);
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "the waiting accept lands once Rae's record reads"
    );
    assert_eq!(
        ann_events.count(&rae, failed),
        0,
        "a record not readable yet is no failure"
    );
}
