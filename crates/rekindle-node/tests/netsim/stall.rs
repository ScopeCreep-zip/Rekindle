//! A stalled process (plan VU2, Veilid 0.5.8): from 2.5 s on, Veilid sends
//! the unchanged attachment state again (`attachment_manager/mod.rs:557-581`,
//! `:1036-1043`). The node's Host takes the duplicate as no change: it
//! warns of nothing and the pair keeps talking.

use rekindle_netsim::Fault;
use rekindle_types::subscription_events::{NetworkEvent, SubscriptionEvent};

use crate::harness::start;
use crate::logs;
use crate::pair::{befriend, talk};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stall_re_sends_the_attachment_and_nothing_breaks() {
    let (sim, mut nodes) = start(2, 95).await;
    for (node, name) in nodes.iter_mut().zip(["Abe", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    let before = logs::counts();
    for _ in 0..3 {
        sim.fault(Fault::Stall { node: 0, ms: 3_000 });
        sim.quiesce().await;
    }
    talk(&sim, &nodes[0], &nodes[1], "after").await;
    assert_eq!(
        logs::counts(),
        before,
        "a re-sent attachment warns of nothing"
    );
}

/// The attachment changes `events` journaled, as `is_attached` values.
fn attachment_edges(events: &crate::harness::Events) -> Vec<bool> {
    let seen = std::cell::RefCell::new(Vec::new());
    events.matching(|event| {
        if let SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { is_attached, .. }) =
            event
        {
            seen.borrow_mut().push(*is_attached);
        }
        false
    });
    seen.into_inner()
}

/// Plan VU4 (synthesis #6): a stall's re-sent attachment is no edge. The
/// frontends hear no network change (the desktop's OS notification is
/// raised on an attached/detached edge only) and the record pool's held
/// writes keep their backoff (`SharedState::set_attachment` wakes the ready
/// watch only on a changed readiness), while a real detach is heard once
/// each way.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stall_is_no_edge_and_starts_no_write_burst() {
    let (sim, mut nodes) = start(1, 96).await;
    nodes[0].unlock_new_identity("Cy").await;
    // The unlock's own-record writes run on after it returns; the window
    // measured below starts once they are done.
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    sim.quiesce().await;
    let events = nodes[0].events().await;
    let sets = sim.app_sets(0);
    for _ in 0..3 {
        sim.fault(Fault::Stall { node: 0, ms: 3_000 });
        sim.quiesce().await;
    }
    assert!(
        attachment_edges(&events).is_empty(),
        "a re-sent attachment is no network change"
    );
    assert_eq!(sim.app_sets(0), sets, "no write burst per stall");

    for detached in [true, false] {
        sim.fault(Fault::Detach { node: 0, detached });
        sim.quiesce().await;
    }
    let edges = attachment_edges(&events);
    let flips = edges.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(
        (edges.first(), edges.last(), flips),
        (Some(&false), Some(&true), 1),
        "one detached edge and one attached edge: {edges:?}"
    );
}
