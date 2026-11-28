//! Live finding L-DM1 (Linux, 3 of 30 runs): the first burst for a new
//! chain (the hint of Ida's first message and her `StreamRecord`, the
//! one frame that names her chain to Jon) is lost in transit with no error,
//! and the route carries her next frame fine. Her `StreamRecord` goes as an
//! `app_call` (`dm::outbox::record_call`), so its loss is seen within the
//! call timeout and it is sent again at once; Jon then diffs her chain and
//! shows the message well before the first re-ask (`DM_FIRST_REASK`, 30 s),
//! the only recovery an `app_message` has. Decision 67 names the initial
//! chain in the friendship handshake, so this exercises a replacement.

use std::time::Duration;

use rekindle_netsim::latency::{GET, MESSAGE, SET};
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::dm_participant::HEAD;
use rekindle_types::message::{SendState, CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};
use rekindle_vault::typed::owner_keys::forget_participant_owner;

use crate::friend_watch::own_state;
use crate::group_dm::own_chain;
use crate::harness::{eventually, start};
use crate::pair::{befriend, holds_message, key, send, LIMIT};

/// How long everything Ida sends is lost: past her first message's hint
/// and the `StreamRecord` its chain's creation queues (a record create and
/// a set).
const LOSS: Duration = Duration::from_secs(4);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_replacement_chains_lost_first_burst_arrives_well_before_the_first_re_ask() {
    let (sim, mut nodes) = start(3, 89).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (ida, jon) = (key(&nodes[0]), key(&nodes[1]));
    let initial = own_chain(&nodes[0], &jon).expect("the request named Ida's initial chain");
    send(&nodes[0], &jon, "on the initial chain").await;
    assert!(
        eventually(&sim, LIMIT, || own_state(
            &nodes[0],
            &jon,
            "on the initial chain",
            |state| { matches!(state, SendState::Delivered | SendState::Read) }
        ))
        .await,
        "Jon acknowledged the initial chain before it was replaced"
    );
    // Losing the saved writer exercises the same replacement path as a
    // peer identity rotation. The first send creates a fresh record.
    {
        let stores = nodes[0].host.identity_stores();
        let vault = stores.vault.lock();
        forget_participant_owner(vault.as_ref().expect("Ida's unlocked vault"), &jon)
            .expect("forget the initial chain writer");
    }
    assert!(own_chain(&nodes[0], &jon).is_none());
    let calls = sim.app_calls(0);
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    let started = tokio::time::Instant::now();
    send(&nodes[0], &jon, "one").await;
    tokio::time::sleep(LOSS).await;
    let replacement = own_chain(&nodes[0], &jon).expect("the first send created a replacement");
    assert_ne!(replacement, initial);
    assert!(sim.network_get(&replacement, HEAD).is_some());
    assert!(
        sim.app_calls(0) > calls,
        "the announcement was attempted during loss"
    );
    assert!(
        sim.watched_subkeys(1, &replacement).is_empty(),
        "Jon has never subscribed to the replacement chain"
    );
    assert!(
        !holds_message(&nodes[1], &ida, "one").await,
        "the burst was lost"
    );
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    // The lost call's timeout, its retry's first step (at most 1 s), the
    // call, Jon's diff of her chain (a read and its page), and a set's
    // worth of slack: far under the first re-ask.
    let bound = Duration::from_millis(CALL_TIMEOUT_MS)
        + Duration::from_secs(1)
        + MESSAGE.max * 2
        + GET.max * 2
        + SET.max;
    let first_reask = Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS));
    assert!(LOSS + bound < first_reask / 2);
    assert!(
        eventually(&sim, bound, || holds_message(&nodes[1], &ida, "one")).await,
        "Jon shows it within {bound:?} of the loss ending (after {:?})",
        started.elapsed()
    );
}
