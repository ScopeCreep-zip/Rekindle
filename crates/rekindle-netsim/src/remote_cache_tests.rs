//! The importer's remote route cache against the pinned Veilid
//! ([`crate::routes`], [`crate::loopback`]): an answered call proves a
//! remote route, and the management task's listing keeps an import alive.

use std::time::Duration;

use rekindle_records::port::{PortError, PortEvent, RouteClass};
use rekindle_types::config::SafetyProfile;

use crate::routes_tests::{dead_remote_within, own_blob};
use crate::tests::attached;
use crate::{Fault, Netsim};

/// An attached importer's listing touches its import on every tick
/// (Veilid 0.5.8 `route_spec_store/mod.rs:313-324` peeks each unexpired
/// entry, `route_spec_store_cache.rs:348-357` `peek_remote_route` touches
/// it): an import unused by sends for 10 minutes still resolves.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_import_never_expires_while_the_management_task_runs() {
    let (sim, mut nodes) = Netsim::nodes(2, 41);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    tokio::time::sleep(Duration::from_secs(600)).await;
    let sent = nodes[0]
        .ports
        .messages
        .send(&target, &SafetyProfile::default_dht(), vec![1])
        .await;
    assert!(
        !matches!(sent, Err(PortError::RouteUnusable(_))),
        "the listing kept it: {sent:?}"
    );
}

/// An answered call over an imported route proves it as a passed test
/// does (Veilid 0.5.8 `rpc_processor/mod.rs:1821-1827`,
/// `route_stats.rs:489-498` `record_answer_received` sets
/// `last_known_valid_ts`), so its next remote route test comes
/// [`crate::loopback::ROUTE_MIN_IDLE`] after the answer, not after the
/// last test: a route that dies from outside right after the answer is
/// released only at that later test.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_answered_call_puts_off_the_next_remote_route_test() {
    let (sim, mut nodes) = Netsim::nodes(2, 42);
    let _pump = sim.start();
    let blob = own_blob(&nodes[1]).await;
    attached(&mut nodes[0]).await;
    attached(&mut nodes[1]).await;
    let mut callee = nodes.remove(1);
    let answering = tokio::spawn(async move {
        while let Some(event) = callee.events.recv().await {
            if let PortEvent::AppCall { call_id, .. } = event {
                let _ = callee.ports.messages.reply(call_id, b"ok".to_vec()).await;
            }
        }
    });
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    // The first test proves it on the next tick; the next is due 30 s on.
    tokio::time::sleep(Duration::from_secs(25)).await;
    let reply = nodes[0]
        .ports
        .messages
        .call(&target, &SafetyProfile::default_dht(), b"hi".to_vec())
        .await;
    assert_eq!(reply.unwrap(), b"ok".to_vec());
    sim.fault(Fault::OutsideDead {
        node: 1,
        class: RouteClass::General,
    });
    // Without the answer the test due near 31 s fails and releases the
    // route at its answer wait's end, before 40 s.
    let quiet = Duration::from_secs(14);
    assert!(
        dead_remote_within(&mut nodes[0], quiet).await.is_empty(),
        "the answer proved the route: no test before 30 s after it"
    );
    let cadence = crate::loopback::ROUTE_MIN_IDLE + crate::RPC_TIMEOUT + Duration::from_secs(2);
    assert_eq!(
        dead_remote_within(&mut nodes[0], cadence).await,
        vec![target.as_str().to_string()]
    );
    answering.abort();
}
