//! A status picked while our public internet is down (plan E7.13, audit
//! 2026-11-18): it is taken and kept, and when the network turns ready our
//! presence publishes a **new** beat with it at once, not at the next 60 s
//! heartbeat. Beats are never held (`rekindle-record-pool` `durable.rs`: a
//! late beat would claim reachability we did not have), so the beat is built
//! at the ready edge from the status held then. Veilid's DHT follows the
//! PublicInternet domain's readiness (0.5.8 `storage_manager/mod.rs:1096-1104`).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, PresenceSnapshot, SubscriptionEvent};

use crate::harness::{eventually, start};
use crate::pair::{befriend, key};

/// Well inside the 60 s heartbeat (`rekindle_presence::HEARTBEAT_INTERVAL_SECS`):
/// a friend sees the status because the ready edge published it.
const AFTER_READY: Duration = Duration::from_secs(10);

fn shows_away(event: &SubscriptionEvent, friend: &str) -> bool {
    matches!(
        event,
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key,
            snapshot: PresenceSnapshot { status: Some(status), .. },
            ..
        }) if peer_key == friend && status == "away"
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_status_set_offline_goes_out_as_a_new_beat_on_ready() {
    let (sim, mut nodes) = start(2, 93).await;
    for (node, name) in nodes.iter_mut().zip(["Uma", "Vic"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let uma = key(&nodes[0]);
    let events = nodes[1].events().await;

    sim.fault(Fault::InternetReady {
        node: 0,
        ready: false,
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    let view = nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    assert_eq!(
        view["status"], "away",
        "the status is taken while not ready"
    );
    // Past any in-flight write's retries: the status waits for the network.
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert!(
        !events.any(|event| shows_away(event, &uma)),
        "nothing reached Vic while Uma's internet was down"
    );

    sim.fault(Fault::InternetReady {
        node: 0,
        ready: true,
    });
    assert!(
        eventually(&sim, AFTER_READY, || async {
            events.any(|event| shows_away(event, &uma))
        })
        .await,
        "Vic sees Uma Away right after Uma's network is ready"
    );
    let got = nodes[0].ok(IpcRequest::PresenceGet).await;
    assert_eq!(got["status"], "away", "the status was kept");
}
