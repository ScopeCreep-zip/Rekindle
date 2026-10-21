//! The prekey record over real Hosts (plan E2.4 Verify): what the status
//! reads, a session reset opened from the record, a record not signed by
//! the pinned identity, and a refill a crash lost.

use rekindle_codec::capnp_codec::identity::{
    decode_opk_slot, decode_prekey_record_list, encode_prekey_head,
};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::dht_layout::prekeys::{slot_subkey, HEAD, SLOT_COUNT};
use rekindle_types::dht_layout::profile::PREKEY_RECORDS;
use rekindle_types::subscription_events::{
    CryptoEvent, FriendEvent, NotificationEvent, SubscriptionEvent,
};

use crate::harness::{eventually, start, SimNode, PASSPHRASE};
use crate::pair::{accept, befriend, is_accepted_friend, key, request, talk, LIMIT};

/// The node's prekey record key, once its unlock holds the record.
async fn record_key(node: &SimNode) -> String {
    node.prekey_status()
        .await
        .record_key
        .expect("a prekey record")
}

/// Every slot of `record` as the network holds it, in subkey order.
fn slots(sim: &Netsim, record: &str) -> Vec<Option<Vec<u8>>> {
    (0..SLOT_COUNT)
        .map(|index| sim.network_get(record, slot_subkey(index)).map(|v| v.data))
        .collect()
}

/// Whether the network holds `record`'s head and every slot: the unlock
/// writes the first-contact part before the profile and the later slots
/// after it, in bounded batches (plan E2.B), so a new identity's record
/// fills over a few set latencies.
fn published_in_full(sim: &Netsim, record: &str) -> bool {
    sim.network_get(record, HEAD).is_some() && slots(sim, record).iter().all(Option::is_some)
}

fn mismatch(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::IdentityPinMismatch { peer_key, .. } => Some(peer_key),
        _ => None,
    }
}

/// Whether `node` holds a session reset request for its review.
async fn holds_review(sim: &Netsim, events: &crate::harness::Events) -> bool {
    eventually(sim, LIMIT, || async {
        events.any(|e| matches!(e, SubscriptionEvent::IntentPending(_)))
    })
    .await
}

/// `rekindle keys prekeys status` and the debug `pqxdh_bundle_info` both
/// send `IpcRequest::PrekeyStatus`: on an unlocked host it reads a full
/// record — 100 fresh slots, the record the profile's device list names,
/// and this device's id.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_prekey_status_reads_the_record_the_profile_names() {
    let (sim, mut nodes) = start(1, 35).await;
    nodes[0].unlock_new_identity("Ann").await;
    assert!(
        eventually(&sim, LIMIT, || async {
            let status = nodes[0].prekey_status().await;
            status.fresh_slots == 100
                && status
                    .record_key
                    .is_some_and(|record| published_in_full(&sim, &record))
        })
        .await
    );
    let status = nodes[0].prekey_status().await;
    let record = status.record_key.clone().expect("a record key");
    let device = status.device_id.clone().expect("a device id");
    assert_eq!(hex::decode(&device).map(|d| d.len()), Ok(16));
    assert_eq!(status.consumed_slots, 0);
    assert!(status.spk_id.is_some() && status.pq_lr_id.is_some());
    let lens = status.component_lens.expect("the published head");
    assert_eq!(
        (lens.spk, lens.spk_sig, lens.pq_lr, lens.pq_lr_sig),
        (32, 64, 1184, 64)
    );

    assert!(sim.network_get(&record, HEAD).is_some());
    assert!(slots(&sim, &record).iter().all(Option::is_some));
    let shown: rekindle_types::dto::identity::IdentityShown =
        serde_json::from_value(nodes[0].ok(IpcRequest::IdentityShow).await).expect("the identity");
    let list = sim
        .network_get(&shown.profile_dht_key, PREKEY_RECORDS)
        .expect("the profile's device list");
    let list = decode_prekey_record_list(&list.data).expect("a device list");
    assert_eq!(list.devices.len(), 1);
    assert_eq!(list.devices[0].record_key, record);
    assert_eq!(hex::encode(list.devices[0].device_id), device);
}

/// A reset opens its new session from the requester's record (B2, B11):
/// Ann asks, Bea's host holds the request for her review, her committed
/// accept is a PreKeyInit from a slot of Ann's record, and messages flow
/// both ways on the new session.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_session_reset_completes_from_the_record() {
    let (sim, mut nodes) = start(2, 32).await;
    nodes[0].unlock_new_identity("Ann").await;
    nodes[1].unlock_new_identity("Bea").await;
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before the reset").await;
    let consumed = nodes[0].prekey_status().await.consumed_slots;

    let (ann_events, bea_events) = (nodes[0].events().await, nodes[1].events().await);
    nodes[0]
        .ok(IpcRequest::SessionResetStart {
            public_key: key(&nodes[1]),
        })
        .await;
    assert!(
        holds_review(&sim, &bea_events).await,
        "Bea reviews the reset"
    );
    nodes[1]
        .call_held(IpcRequest::SessionResetAccept {
            public_key: key(&nodes[0]),
        })
        .await
        .expect("Bea accepts the reset");

    // Bea's init consumed a slot of Ann's record, and Ann admitted it.
    assert!(
        eventually(&sim, LIMIT, || async {
            nodes[0].prekey_status().await.consumed_slots == consumed + 1
        })
        .await
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            ann_events.any(|e| {
                matches!(e, SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
                    if title == "Secure session re-established")
            })
        })
        .await
    );
    talk(&sim, &nodes[0], &nodes[1], "after the reset").await;
}

/// B3: a friend's record whose head is signed by another identity — its
/// owner key is not their identity key, so a holder of that key can swap
/// in their own prekeys — opens nothing. Ann's accept of Rae's reset
/// raises `IdentityPinMismatch` and fails; no init reaches Rae, the safety
/// number is the same, the old session still carries messages, and Rae's
/// request is still pending for Ann to decline.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_record_signed_by_another_identity_opens_nothing() {
    let (sim, mut nodes) = start(2, 33).await;
    nodes[0].unlock_new_identity("Ann").await;
    nodes[1].unlock_new_identity("Rae").await;
    befriend(&sim, &nodes[1], &nodes[0]).await;
    talk(&sim, &nodes[0], &nodes[1], "before the swap").await;
    let rae = key(&nodes[1]);
    let safety = nodes[0]
        .ok(IpcRequest::IdentitySafetyNumber { peer: rae.clone() })
        .await;
    let consumed = nodes[1].prekey_status().await.consumed_slots;

    let record = record_key(&nodes[1]).await;
    let owner = sim.owner_keypair(&record).expect("its owner keypair");
    let mallory = rekindle_crypto::Identity::generate();
    let forged = rekindle_crypto::signal::prekey_record::mint_head(
        &mallory,
        &rekindle_utils::OsRandom,
        7,
        8,
        rekindle_utils::timestamp_ms_i64(),
    );
    sim.forge_write(&record, HEAD, encode_prekey_head(&forged.record), &owner)
        .expect("an owner write");

    let ann_events = nodes[0].events().await;
    nodes[1]
        .ok(IpcRequest::SessionResetStart {
            public_key: key(&nodes[0]),
        })
        .await;
    assert!(holds_review(&sim, &ann_events).await);
    let accepted = nodes[0]
        .call_held(IpcRequest::SessionResetAccept {
            public_key: rae.clone(),
        })
        .await;
    assert!(accepted.is_err(), "no session from a swapped record");
    sim.quiesce().await;
    assert_eq!(ann_events.count(&rae, mismatch), 1);
    assert_eq!(nodes[1].prekey_status().await.consumed_slots, consumed);
    assert_eq!(
        nodes[0]
            .ok(IpcRequest::IdentitySafetyNumber { peer: rae.clone() })
            .await,
        safety
    );
    assert!(is_accepted_friend(&nodes[0], &rae).await);
    talk(&sim, &nodes[0], &nodes[1], "after the swap").await;
    // Rae's request is still Ann's to answer.
    nodes[0]
        .call_held(IpcRequest::SessionResetDecline { public_key: rae })
        .await
        .expect("the reset is still pending");
}

/// A kill -9 between the B8 commit and the slot write: the refilled slot's
/// write is lost with the process, nothing re-pushes it, and the next
/// unlock's reconciliation publishes it from the rows.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_refill_lost_in_a_crash_is_published_at_the_next_unlock() {
    let (sim, mut nodes) = start(2, 34).await;
    nodes[0].unlock_new_identity("Rae").await;
    nodes[1].unlock_new_identity("Ann").await;
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[0], &nodes[1]).await;
    let record = record_key(&nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || async { published_in_full(&sim, &record) }).await);
    let before = slots(&sim, &record);

    let rae_events = nodes[0].events().await;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    accept(&nodes[1], &nodes[0]).await;
    // The commit consumed the slot; its refill's write is held.
    assert!(
        eventually(&sim, LIMIT, || async {
            rae_events.any(|e| {
                matches!(
                    e,
                    SubscriptionEvent::Crypto(CryptoEvent::PrekeyRecordPublished { slots, .. })
                        if !slots.is_empty()
                )
            })
        })
        .await
    );
    assert_eq!(nodes[0].prekey_status().await.consumed_slots, 1);
    sim.lose_held(0);
    sim.fault(Fault::Heal);
    sim.quiesce().await;
    assert_eq!(
        slots(&sim, &record),
        before,
        "nothing re-pushed the lost write"
    );

    nodes[0].ok(IpcRequest::Lock).await;
    nodes[0]
        .ok(IpcRequest::Unlock {
            identity: rae,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { slots(&sim, &record) != before }).await,
        "the unlock reconciles the record"
    );
    let after = slots(&sim, &record);
    let changed: Vec<usize> = (0..before.len())
        .filter(|i| before[*i] != after[*i])
        .collect();
    assert_eq!(changed.len(), 1, "only the consumed slot moved");
    let slot = |bytes: &Option<Vec<u8>>| {
        decode_opk_slot(bytes.as_deref().expect("a slot")).expect("a signed slot")
    };
    let (old, new) = (slot(&before[changed[0]]), slot(&after[changed[0]]));
    assert_ne!(old.opk_id, new.opk_id, "the refill holds fresh keys");
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
}
