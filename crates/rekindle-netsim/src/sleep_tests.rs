use std::time::Duration;

use rekindle_records::port::{PortEvent, RecordSchema, RouteClass, SetOutcome};
use rekindle_types::config::SafetyProfile;

use crate::tests::{attached, recv};
use crate::{Fault, Netsim};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_sleeping_node_issues_no_dht_call_and_no_send_until_it_wakes() {
    let (sim, mut nodes) = Netsim::nodes(2, 41);
    let pump = sim.start();
    nodes[1].ports.routes.want_own();
    let blob = tokio::time::timeout(
        Duration::from_secs(5),
        nodes[1].ports.routes.available_own(RouteClass::General),
    )
    .await
    .unwrap();
    attached(&mut nodes[1]).await;
    let target = nodes[0].ports.routes.import(&blob).unwrap();
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();

    sim.fault(Fault::Sleep {
        node: 0,
        asleep: true,
    });
    let writer = std::sync::Arc::clone(&pool);
    let set = tokio::spawn(async move { writer.set(lease, 0, b"v".to_vec(), None).await });
    let messages = nodes[0].ports.messages.clone();
    let send = tokio::spawn(async move {
        messages
            .send(&target, &SafetyProfile::default_dht(), b"hi".to_vec())
            .await
    });
    // Under the remote route cache's 5-minute idle expiry.
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(!set.is_finished() && !send.is_finished(), "both parked");
    assert_eq!(sim.app_sets(0), 0, "no DHT call left the node");
    assert_eq!(sim.network_get(&key, 0), None);
    assert!(nodes[1].events.try_recv().is_err(), "nothing was sent");

    sim.fault(Fault::Sleep {
        node: 0,
        asleep: false,
    });
    assert_eq!(set.await.unwrap().unwrap(), SetOutcome::Landed);
    send.await.unwrap().unwrap();
    assert!(
        matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message, .. } if message == b"hi")
    );
    assert_eq!(sim.app_sets(0), 1);
    pump.abort();
}
