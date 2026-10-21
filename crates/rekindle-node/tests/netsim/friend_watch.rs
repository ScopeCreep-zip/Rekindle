//! Friend watches (plan E2.5, D23): no poll reads a friend's profile. Their
//! status change arrives by the profile record's watch, and after that
//! watch dies (Veilid's last change, `count == 0`) the pool re-arms it, so
//! the next change arrives too.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start, Events};
use crate::pair::{befriend, key, LIMIT};

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_status_change_arrives_by_watch_and_survives_a_watch_death() {
    let (sim, mut nodes) = start(2, 83).await;
    for (node, name) in nodes.iter_mut().zip(["Kim", "Lev"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let kim = key(&nodes[0]);
    let events = nodes[1].events().await;

    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &kim, "away") }).await,
        "Lev sees Kim away by the watch"
    );

    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendCode).await).expect("the code");
    sim.fault(Fault::WatchDeath {
        node: 1,
        key: code.profile_key,
    });
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Busy,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &kim, "busy") }).await,
        "Lev sees Kim busy after the watch was re-armed"
    );
}
