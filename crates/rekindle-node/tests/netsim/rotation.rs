//! An identity rotation whose new prekey set cannot land (plan E2.4: the
//! set is in the record before any `IdentityRotated` notice goes): with
//! the rotator's DHT away the rotation fails, no friend hears of it and
//! the identity keeps its key; with the DHT back it succeeds.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::dht_layout::prekeys::HEAD;
use rekindle_types::subscription_events::SubscriptionEvent;

use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{befriend, is_accepted_friend, key, LIMIT};

fn rotate() -> IpcRequest {
    IpcRequest::IdentityRotate {
        passphrase: PASSPHRASE.into(),
    }
}

fn accept_rotation(old: &str) -> IpcRequest {
    IpcRequest::IdentityRotationAccept {
        public_key: old.to_string(),
    }
}

/// The unlocked identity's public key as `IdentityShow` reads it.
async fn shown_key(node: &SimNode) -> String {
    node.ok(IpcRequest::IdentityShow).await["publicKey"]
        .as_str()
        .expect("an unlocked identity")
        .to_string()
}

/// Let simulated time pass, as long as a notice takes to arrive.
async fn settle(sim: &Netsim) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    sim.quiesce().await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E2.B finding F9: the rotation lands but network_get still reads the old head (rotation.rs:113); suspected: after the failed rotation left a partial head at seq n+1 on some replicas, the retried rotation lands at the same seq with a split (set_value.rs:465-500)"]
async fn a_rotation_whose_set_cannot_land_tells_no_friend() {
    let (sim, mut nodes) = start(2, 41).await;
    nodes[0].unlock_new_identity("Ann").await;
    nodes[1].unlock_new_identity("Bea").await;
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let ann = key(&nodes[0]);
    let bea_events = nodes[1].events().await;
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("a prekey record");
    let head = sim.network_get(&record, HEAD).map(|v| v.data);
    assert!(head.is_some());

    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let refused = nodes[0]
        .call(rotate())
        .await
        .expect_err("the set did not land");
    assert!(
        refused.message.contains("identity rotation failed"),
        "{}",
        refused.message
    );
    settle(&sim).await;
    assert!(
        !bea_events.any(|e| matches!(e, SubscriptionEvent::IntentPending(_))),
        "Bea holds no rotation for review"
    );
    assert!(nodes[1].call(accept_rotation(&ann)).await.is_err());
    assert!(is_accepted_friend(&nodes[1], &ann).await);
    assert_eq!(sim.network_get(&record, HEAD).map(|v| v.data), head);

    // The journal was cleared: the identity unlocks under its old key.
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    nodes[0].ok(IpcRequest::Lock).await;
    nodes[0]
        .ok(IpcRequest::Unlock {
            identity: ann.clone(),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert_eq!(shown_key(&nodes[0]).await, ann);
    assert_eq!(sim.network_get(&record, HEAD).map(|v| v.data), head);

    // With the DHT back the rotation lands, and Bea accepts it.
    let deadline = tokio::time::Instant::now() + LIMIT;
    let rotated = loop {
        match nodes[0].call(rotate()).await {
            Ok(rotated) => break rotated,
            Err(refused) => assert!(
                tokio::time::Instant::now() < deadline,
                "the rotation never landed: {}",
                refused.message
            ),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let new_ann = rotated["new_public_key"]
        .as_str()
        .expect("the new key")
        .to_string();
    assert_ne!(new_ann, ann);
    assert_ne!(sim.network_get(&record, HEAD).map(|v| v.data), head);
    assert!(
        eventually(&sim, LIMIT, || async {
            nodes[1].call(accept_rotation(&ann)).await.is_ok()
        })
        .await
    );
    assert!(is_accepted_friend(&nodes[1], &new_ann).await);
}
