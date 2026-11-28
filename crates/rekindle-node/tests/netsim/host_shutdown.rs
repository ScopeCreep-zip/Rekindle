//! A Host's exit shuts its transport node down rather than dropping it
//! (plan VU4, A/B R1-16 "transport shutdown with outstanding references —
//! dropping refs=3", C7.27 "database still held at exit"). A dropped node
//! never runs Veilid's `shutdown()`, so its table store and 0.5.8's
//! namespace lock `__veilid_lock` (`table_store/native.rs:113-145`) stay
//! held and no later start in the process can open them.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{
    allocation_entry, Trace, ALLOCATION_CUT, ALLOCATION_ENDS, ALLOCATION_STARTS,
};
use rekindle_records::port::RouteClass;

use crate::harness::start;
use crate::logs;
use crate::pair::{befriend, talk};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_exit_shuts_the_transport_down() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Dee", "Eli"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "bye").await;
    for node in &nodes {
        node.host.shutdown().await.expect("the host stops");
    }
    assert_eq!(
        logs::warned("outstanding references"),
        Vec::<String>::new(),
        "every node was shut down, none dropped"
    );
}

/// Polls the logout's spawned allocations get to reach their route test.
const YIELDS: usize = 16;

/// Live A/B round 3 R3-6: logout releases the session's routes and wants
/// General and Media again at once (C7.9b: wanted from node-ready, locked or
/// not), so the exit right after it finds their allocations in flight. Each
/// allocation runs its route's test (Veilid 0.5.8 `veilid_api/api.rs:812-842`)
/// and the exit awaits it rather than cutting it: a cut drops Veilid's
/// `DurationRecorder` unfinished, logged as an ERROR
/// (`logging/duration_recorder.rs:84-90`). `Sending` is the session's
/// (RH1r): it is not allocated again after the logout.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_exit_right_after_logout_awaits_the_allocations_in_flight() {
    let (sim, mut nodes) = start(1, 98).await;
    nodes[0].unlock_new_identity("Fay").await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let node = &nodes[0];
    let before = sim.trace().of(node.index).count();
    node.ok(IpcRequest::Logout).await;
    let since = |trace: &Trace, event: &str, class: RouteClass| {
        let entry = allocation_entry(event, class);
        trace
            .of(node.index)
            .skip(before)
            .filter(|e| *e == entry)
            .count()
    };
    let in_flight = |trace: &Trace| -> usize {
        [RouteClass::General, RouteClass::Media]
            .into_iter()
            .map(|c| since(trace, ALLOCATION_STARTS, c) - since(trace, ALLOCATION_ENDS, c))
            .sum()
    };
    // The logout's reply comes before the allocations it wanted first run
    // (they are spawned, and this runtime has one thread): let them start,
    // as they have by the time a live exit request arrives. No simulated
    // time passes, so none can end.
    for _ in 0..YIELDS {
        if in_flight(&sim.trace()) == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        in_flight(&sim.trace()),
        2,
        "General and Media are allocating again at the exit"
    );
    node.host.shutdown().await.expect("the host stops");
    let after = sim.trace();
    let cut: Vec<&str> = after
        .of(node.index)
        .filter(|e| e.starts_with(ALLOCATION_CUT))
        .collect();
    assert!(cut.is_empty(), "an allocation was cut at the exit: {cut:?}");
}
