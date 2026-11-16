//! A/B round 5 finding R5-5: Bea's DmText to Ann was lost while its chain
//! append landed, and Ann read it only at Bea's re-hint (~42 s later).
//!
//! - The chain is authoritative (plan E2.5, D23) and Ann watches Bea's
//!   participant record once its `StreamRecord` reached her, so a landed
//!   entry reaches her through that watch: the append's sets, the replica's
//!   next value-change tick (Veilid 0.5.8 `SEND_VALUE_CHANGES_INTERVAL_SECS`
//!   = 1, `storage_manager/constants.rs:14`), the `ValueChanged` message and
//!   her diff's one `HEAD` read and page read. The diff read a second
//!   subkey, the retire mark, empty until the first retire, whose fanout
//!   ran to its deadline (~11 s) on every diff; the mark now rides in
//!   `HEAD` (`rekindle_records::chain`).
//! - A chain announced by a `StreamRecord` lost with its hint is read only
//!   once the outbox re-asks: by design (plan E2.5), a `StreamRecord` is a
//!   `dm` row settled only by the receiver's `DmAck`, so its loss is
//!   retried at the first re-ask, `DM_FIRST_REASK` to 1.5× it.

use std::time::Duration;

use rekindle_netsim::latency::{GET, MESSAGE, SET};
use rekindle_netsim::{Fault, Netsim};
use rekindle_outbox::DM_FIRST_REASK;
use rekindle_types::message::SendState;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, holds_message, key, send, LIMIT};

/// A landed entry's way to the watching reader: a page append is two sets
/// (`chain::append`), then the replica's value-change tick (1 s), the
/// `ValueChanged` message, and the diff's `HEAD` and page reads.
fn chain_bound() -> Duration {
    SET.max * 2 + Duration::from_secs(1) + MESSAGE.max + GET.max * 2
}

async fn own_state(node: &SimNode, peer: &str, body: &str, state: SendState) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(rekindle_ipc::protocol::IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history
        .iter()
        .any(|m| m.is_own && m.body == body && m.send_state == state)
}

async fn reads(sim: &Netsim, reader: &SimNode, writer: &SimNode, body: &str) {
    let from = key(writer);
    assert!(
        eventually(sim, LIMIT, || holds_message(reader, &from, body)).await,
        "node {} reads {body:?}",
        reader.index
    );
}

/// Ann (node 0) and Bea (node 1), friends, and a node holding records.
async fn friends(seed: u64) -> (Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(3, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    (sim, nodes)
}

/// Bea's every app message is dropped from the send of `body` until its
/// entry is on her chain (its row `Sent`), so its hint is lost and only
/// the chain carries it. When the send was.
async fn send_with_lost_hint(sim: &Netsim, nodes: &[SimNode], body: &str) -> tokio::time::Instant {
    let ann = key(&nodes[0]);
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let sent = tokio::time::Instant::now();
    send(&nodes[1], &ann, body).await;
    assert!(
        eventually(sim, LIMIT, || own_state(
            &nodes[1],
            &ann,
            body,
            SendState::Sent
        ))
        .await,
        "Bea's entry landed on her chain"
    );
    sim.fault(Fault::DropFrom {
        node: 1,
        per_mille: 0,
    });
    sent
}

/// The live order: Bea's first message at the accept, Ann's reply, then
/// Bea's second message, whose hint is lost: Ann reads it by the watch on
/// Bea's chain within [`chain_bound`], long before any re-ask.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_hint_after_both_spoke_arrives_by_the_chain_watch() {
    let (sim, nodes) = friends(158).await;
    send(&nodes[1], &key(&nodes[0]), "p1").await;
    reads(&sim, &nodes[0], &nodes[1], "p1").await;
    send(&nodes[0], &key(&nodes[1]), "m1").await;
    reads(&sim, &nodes[1], &nodes[0], "m1").await;
    // Live: 21 s between Ann's reply and Bea's second message.
    tokio::time::sleep(Duration::from_secs(21)).await;
    let sent = send_with_lost_hint(&sim, &nodes, "p2").await;
    let (bea, bound) = (key(&nodes[1]), chain_bound());
    assert!(bound < DM_FIRST_REASK);
    let left = bound.saturating_sub(sent.elapsed());
    assert!(
        eventually(&sim, left, || holds_message(&nodes[0], &bea, "p2")).await,
        "Ann reads p2 by Bea's chain within {bound:?} of its send (none by {:?})",
        sent.elapsed()
    );
}

/// Ann spoke first; Bea's first message creates her chain, and the
/// `StreamRecord` announcing it is lost with the hint. Designed (plan
/// E2.5): the `StreamRecord` is a `dm` row settled only by Ann's `DmAck`,
/// so Ann learns the chain at the outbox's first re-ask, drawn from
/// `[DM_FIRST_REASK, 1.5 × DM_FIRST_REASK]`, and not sooner.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_first_hint_and_its_stream_record_arrive_at_the_first_reask() {
    let (sim, nodes) = friends(159).await;
    send(&nodes[0], &key(&nodes[1]), "m1").await;
    reads(&sim, &nodes[1], &nodes[0], "m1").await;
    tokio::time::sleep(Duration::from_secs(21)).await;
    let sent = send_with_lost_hint(&sim, &nodes, "p1").await;
    let bea = key(&nodes[1]);
    let before = DM_FIRST_REASK.saturating_sub(sent.elapsed());
    assert!(
        !eventually(&sim, before, || holds_message(&nodes[0], &bea, "p1")).await,
        "nothing announces Bea's chain before the first re-ask"
    );
    let window = DM_FIRST_REASK * 3 / 2 + MESSAGE.max * 2 + GET.max * 2;
    let left = window.saturating_sub(sent.elapsed());
    assert!(
        eventually(&sim, left, || holds_message(&nodes[0], &bea, "p1")).await,
        "Ann reads p1 by the first re-ask's window {window:?} (none by {:?})",
        sent.elapsed()
    );
}
