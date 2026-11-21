//! A stretch of a member's group chain that retires before we read the
//! chain is a history gap only if we lack one of its posts (plan E2.6, G4).
//! Bea is off the DHT but reachable, so Ann's posts reach her as hints (the
//! entry itself, stored on receipt, `group_dm/receive.rs` `on_hint`) while
//! her chain diff reads nothing; Ann's chain then retires the stretch by
//! age. Holding every post, Bea's diff over the retire mark stores no gap
//! row and publishes no gap; missing one post (detached for it), it does:
//! a hidden real gap is worse than a false one.

use rekindle_netsim::{Fault, Netsim};
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use crate::group_dm::{history, own_chain, reads, send, three_members};
use crate::harness::{eventually, start, SimNode};
use crate::pair::{key, retired, unretired_chain, LIMIT};

/// What Bea's diff logs when it meets a retire mark past her cursor.
const RETIRED_PAST: &str = "group chain retired past our cursor";

/// Ann sends `body`; Cid reads it and Ann notes it sent on her chain.
async fn post(sim: &Netsim, nodes: &[SimNode], gid: &str, body: &str) {
    send(&nodes[0], gid, body).await;
    assert!(eventually(sim, LIMIT, || reads(&nodes[2], gid, body)).await);
    assert!(
        eventually(sim, LIMIT, || async {
            history(&nodes[0], gid).await.iter().any(|m| {
                m.is_own
                    && m.body == body
                    && m.send_state == rekindle_types::message::SendState::Sent
            })
        })
        .await,
        "Ann's {body:?} is noted on her chain"
    );
}

async fn has_gap(node: &SimNode, gid: &str, ann: &str) -> bool {
    history(node, gid)
        .await
        .iter()
        .any(|m| m.history_gap_through.is_some() && m.sender_id == ann)
}

/// Bea holds Ann's stretch by hint (missing one post when `miss_one`), the
/// stretch retires, Bea's DHT returns and her diff meets the mark. Whether
/// Bea stored or was shown a gap for Ann.
async fn hint_stretch(seed: u64, miss_one: bool) -> bool {
    let (sim, mut nodes) = start(3, seed).await;
    let gid = three_members(&sim, &mut nodes).await;
    let ann = key(&nodes[0]);
    let bea_events = nodes[1].events().await;
    let keys = vec![own_chain(&nodes[0], &gid).expect("Ann's group chain is saved")];
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    post(&sim, &nodes, &gid, "held by hint 1").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "held by hint 1")).await);
    if miss_one {
        sim.fault(Fault::Detach {
            node: 1,
            detached: true,
        });
        post(&sim, &nodes, &gid, "missed").await;
        // Reattaching brings her DHT back too (`rekindle-netsim`
        // `attach.rs` `detach`): keep her off it before any task runs, so
        // her catch-up does not read the chain the stretch is on.
        sim.fault(Fault::Detach {
            node: 1,
            detached: false,
        });
        sim.fault(Fault::DhtOffline {
            node: 1,
            offline: true,
        });
    }
    post(&sim, &nodes, &gid, "held by hint 2").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "held by hint 2")).await);
    assert!(unretired_chain(&sim, &keys[0]), "no retire mark yet");
    sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 31 * 24 * 60 * 60 * 1000,
    });
    send(&nodes[0], &gid, "after the horizon").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "after the horizon")).await);
    assert!(
        eventually(&sim, LIMIT, || async { retired(&sim, &keys) }).await,
        "Ann's group chain is retired"
    );
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    send(&nodes[0], &gid, "once more").await;
    assert!(
        eventually(&sim, LIMIT * 2, || async {
            !crate::logs::logged(1, &[RETIRED_PAST]).is_empty()
        })
        .await,
        "Bea's diff meets Ann's retire mark: {:#?}",
        crate::logs::logged(1, &["group"])
    );
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "once more")).await);
    for body in ["held by hint 1", "held by hint 2", "after the horizon"] {
        assert!(reads(&nodes[1], &gid, body).await, "Bea holds {body:?}");
    }
    if miss_one {
        assert!(
            !reads(&nodes[1], &gid, "missed").await,
            "the missed post is retired before Bea reads the chain"
        );
    }
    let shown = bea_events.any(|e| {
        matches!(
            e,
            SubscriptionEvent::GroupDm(GroupDmEvent::HistoryGap { member, .. })
                if member.public_key == ann
        )
    });
    let stored = has_gap(&nodes[1], &gid, &ann).await;
    assert_eq!(stored, shown, "a stored gap is a shown one");
    stored
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retired_stretch_held_by_hint_is_no_history_gap() {
    assert!(
        !hint_stretch(95, false).await,
        "Bea holds every post of the stretch: no gap"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retired_stretch_missing_one_hint_is_a_history_gap() {
    assert!(
        hint_stretch(96, true).await,
        "Bea lacks one post of the stretch: a gap"
    );
}
