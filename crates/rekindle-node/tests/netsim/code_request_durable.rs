//! A friend code's request has a durable copy (plan RV2e, lead decision
//! 67). Once the owner's profile is held, its Host issues the live code's
//! request inbox, whose one-time writer is derived from the code's token
//! and whose key the owner's profile publishes (`CODE_INBOX`); the
//! requester sends the sealed `FriendRequest` by `app_call` and writes the
//! same datagram into the slot. With every call to the code's owner lost
//! (`Fault::CallHangs`, the Linux finding: a code request stalled minutes
//! against a healthy-looking General), the owner opens the durable copy
//! through the same admission, the friendship completes, the slot rolls,
//! and the owner's General rotates once (evidence source 6). With both
//! paths up: one request, no rotation. A writer not derived from the token
//! cannot write the slot; a holder's write rolls it, within the roll
//! budget. A block moves the profile, which publishes a fresh slot.

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::{RecordSchema, RouteClass};
use rekindle_route::liveness::{HINT_WINDOW, REPORT_ROTATE_AGE};
use rekindle_types::dht_layout::request_inbox;
use rekindle_types::dto::friends::FriendCodeView;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, has_request_from, is_accepted_friend, key, talk, LIMIT};

fn requested(event: &FriendEvent) -> Option<&str> {
    match event {
        FriendEvent::RequestReceived { from_key, .. } => Some(from_key),
        _ => None,
    }
}

/// `owner`'s friend code, and the code slot its Host issued once its
/// profile was held.
async fn issue(sim: &Netsim, owner: &SimNode) -> (FriendCodeView, String) {
    let code: FriendCodeView =
        serde_json::from_value(owner.ok(IpcRequest::FriendCode).await).expect("the code");
    (code, only_slot(sim, owner.index).await)
}

/// The one code slot `node` watches (it has no invite or open request).
async fn only_slot(sim: &Netsim, node: usize) -> String {
    assert!(
        eventually(sim, LIMIT, || async {
            inboxes_watched_by(sim, node).len() == 1
        })
        .await,
        "node {node} watches one code slot"
    );
    inboxes_watched_by(sim, node)
        .into_iter()
        .next()
        .expect("one slot")
}

async fn add(requester: &SimNode, code: &FriendCodeView) {
    requester
        .ok(IpcRequest::FriendAddFromCode {
            code: code.url.clone(),
            message: "hi".to_string(),
        })
        .await;
}

/// A holder of `code` writes junk into `slot`, which node `owner` watches;
/// the one slot `owner` watches next, or `None` when it still watches
/// `slot` after `wait`.
async fn junk_roll(
    sim: &Netsim,
    owner: usize,
    code: &FriendCodeView,
    slot: &str,
    wait: Duration,
) -> Option<String> {
    let token = rekindle_types::key_format::friend_request_token(&code.token).expect("token");
    let holder = rekindle_secrets::friend_token::inbox_writer(&token);
    let holder = rekindle_crypto::keypair_text::keypair_text_from_signing(&holder);
    sim.forge_write(slot, request_inbox::REQUEST, vec![1, 2, 3], &holder)
        .expect("a holder of the code writes its slot");
    let rolled = eventually(sim, wait, || async {
        let watched = inboxes_watched_by(sim, owner);
        !watched.contains(slot) && watched.len() == 1
    })
    .await;
    rolled.then(|| {
        inboxes_watched_by(sim, owner)
            .into_iter()
            .next()
            .expect("one slot")
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_code_request_lands_from_its_slot_while_every_call_hangs() {
    let (sim, mut nodes) = start(2, 81).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let (code, slot) = issue(&sim, &nodes[1]).await;
    // Ann's General route is old enough for Rae to hold its blob.
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[1].routes.own_id(RouteClass::General);
    assert!(general.is_some(), "Ann's General route is up");
    let events = nodes[1].events().await;
    // Every call to Ann, the code's owner, is lost on the wire.
    sim.fault(Fault::CallHangs {
        node: 1,
        hangs: true,
    });
    add(&nodes[0], &code).await;
    assert!(
        eventually(&sim, LIMIT, || has_request_from(&nodes[1], &rae)).await,
        "Ann opened Rae's request from her code's slot"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            let watched = inboxes_watched_by(&sim, 1);
            !watched.contains(&slot) && !watched.is_empty()
        })
        .await,
        "the used slot rolled: released, a fresh one watched"
    );
    sim.fault(Fault::CallHangs {
        node: 1,
        hangs: false,
    });
    tokio::time::sleep(HINT_WINDOW + Duration::from_secs(5)).await;
    let rotated = nodes[1].routes.own_id(RouteClass::General);
    assert!(
        rotated.is_some() && rotated != general,
        "no call copy came over General: it rotated"
    );
    accept(&nodes[1], &nodes[0]).await;
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "the friendship completes"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the durable request").await;
    assert_eq!(events.count(&rae, requested), 1, "one request");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_paths_up_one_request_and_nothing_rotates() {
    let (sim, mut nodes) = start(2, 83).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let (code, _) = issue(&sim, &nodes[1]).await;
    tokio::time::sleep(REPORT_ROTATE_AGE + Duration::from_secs(10)).await;
    let general = nodes[1].routes.own_id(RouteClass::General);
    let events = nodes[1].events().await;
    add(&nodes[0], &code).await;
    assert!(
        eventually(&sim, LIMIT, || has_request_from(&nodes[1], &rae)).await,
        "Ann holds Rae's request"
    );
    tokio::time::sleep(HINT_WINDOW + Duration::from_secs(5)).await;
    assert_eq!(
        nodes[1].routes.own_id(RouteClass::General),
        general,
        "the call copy came over General: no evidence"
    );
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    assert_eq!(events.count(&rae, requested), 1, "one request, two copies");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn only_the_codes_holders_write_its_slot_and_a_write_rolls_it() {
    let (sim, mut nodes) = start(2, 85).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (code, slot) = issue(&sim, &nodes[1]).await;
    let stranger = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let stranger = rekindle_crypto::keypair_text::keypair_text_from_signing(&stranger);
    assert!(
        sim.forge_write(&slot, request_inbox::REQUEST, vec![1, 2, 3], &stranger)
            .is_err(),
        "a writer not derived from the code's token is refused"
    );
    assert!(
        junk_roll(&sim, 1, &code, &slot, LIMIT).await.is_some(),
        "the written slot rolled once: released, one fresh slot watched"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_junk_writing_holder_gets_no_more_rolls_than_the_budget() {
    use rekindle_route::budget::BUDGET_CAPACITY;
    let (sim, mut nodes) = start(2, 87).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (code, mut slot) = issue(&sim, &nodes[1]).await;
    for n in 0..BUDGET_CAPACITY {
        slot = junk_roll(&sim, 1, &code, &slot, LIMIT)
            .await
            .unwrap_or_else(|| panic!("roll {n} is within the budget"));
    }
    assert!(
        junk_roll(&sim, 1, &code, &slot, Duration::from_secs(30 * 60))
            .await
            .is_none(),
        "past the budget the written slot stays taken"
    );
    // The bucket refills by the day; the paced roll is retried hourly.
    tokio::time::sleep(Duration::from_secs(23 * 60 * 60)).await;
    assert!(
        eventually(&sim, Duration::from_secs(3 * 60 * 60), || async {
            let watched = inboxes_watched_by(&sim, 1);
            !watched.contains(&slot) && watched.len() == 1
        })
        .await,
        "the paced roll comes with the day's refill"
    );
}

/// The roll budget is the identity's, stored with it (plan RV2e): a holder
/// who spent it gets no fresh bucket from the owner's restart; the next
/// roll still waits for the day's refill.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restart_does_not_refill_the_roll_budget() {
    use rekindle_route::budget::BUDGET_CAPACITY;
    let (sim, mut nodes) = start(2, 88).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (code, mut slot) = issue(&sim, &nodes[1]).await;
    for n in 0..BUDGET_CAPACITY {
        slot = junk_roll(&sim, 1, &code, &slot, LIMIT)
            .await
            .unwrap_or_else(|| panic!("roll {n} is within the budget"));
    }
    let ann = nodes.remove(1).restart(&sim, Duration::from_secs(30)).await;
    ann.ok(IpcRequest::Unlock {
        identity: key(&ann),
        passphrase: crate::harness::PASSPHRASE.into(),
    })
    .await;
    let slot = only_slot(&sim, 1).await;
    assert!(
        junk_roll(&sim, 1, &code, &slot, Duration::from_secs(30 * 60))
            .await
            .is_none(),
        "after the restart the spent budget still holds the written slot"
    );
    tokio::time::sleep(Duration::from_secs(23 * 60 * 60)).await;
    assert!(
        eventually(&sim, Duration::from_secs(3 * 60 * 60), || async {
            let watched = inboxes_watched_by(&sim, 1);
            !watched.contains(&slot) && watched.len() == 1
        })
        .await,
        "the paced roll comes with the day's refill"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_block_moves_the_profile_and_publishes_a_fresh_slot_on_it() {
    let (sim, mut nodes) = start(2, 89).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (before, slot) = issue(&sim, &nodes[1]).await;
    nodes[1]
        .ok(IpcRequest::FriendBlock {
            public_key: key(&nodes[0]),
        })
        .await;
    let after: FriendCodeView =
        serde_json::from_value(nodes[1].ok(IpcRequest::FriendCode).await).expect("the code");
    assert_ne!(
        after.profile_key, before.profile_key,
        "the block moved the profile"
    );
    assert!(
        eventually(&sim, LIMIT, || async {
            !inboxes_watched_by(&sim, 1).contains(&slot)
        })
        .await,
        "the old profile's slot is released"
    );
    let fresh = only_slot(&sim, 1).await;
    let published = sim
        .network_get(
            &after.profile_key,
            rekindle_types::dht_layout::profile::CODE_INBOX,
        )
        .expect("the new profile publishes its code slot");
    assert_eq!(published.data, fresh.as_bytes(), "naming the fresh slot");
}

/// The request-inbox-shaped records (one owner subkey, one one-time
/// writer) the node watches: its code's slot among them.
fn inboxes_watched_by(sim: &Netsim, node: usize) -> BTreeSet<String> {
    sim.watched_by(node)
        .into_iter()
        .filter(|(_, schema)| {
            matches!(schema, RecordSchema::Smpl { owner_subkeys, members }
                if *owner_subkeys == request_inbox::OWNER_SUBKEY_COUNT && members.len() == 1)
        })
        .map(|(key, _)| key)
        .collect()
}
