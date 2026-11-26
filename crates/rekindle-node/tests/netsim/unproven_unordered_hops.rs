//! Plan VU2 (`research-delivery-2026-11-25.md`, DL1/DL2): 1:1 control
//! traffic is sent ordered, as Veilid intends (plan VU decision 2). A
//! receiver's General route is Ordered and Veilid proved its hops on that
//! ordering alone (0.5.8 fork `route_allocate.rs:580-588`); every hop
//! re-applies the sender's sequencing (`rpc_route.rs:26-29`, `:87-90`), so
//! an unordered send crosses UDP hop pairs nobody proved and is lost
//! ([`Fault::UnprovenUnorderedHops`]). In the real app unordered 1:1 calls
//! to such routes arrived 8 in 48 and the group invite and a new friend's
//! presence key were lost; ordered, 119 in 122 arrived.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::latency::SET;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start};
use crate::pair::{befriend, holds_message, key, send, LIMIT};

const BODY: &str = "over ordered hops";

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn control_sends_cross_a_route_whose_unordered_hops_were_never_proven() {
    let (sim, mut nodes) = start(3, 93).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann", "Cy"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    befriend(&sim, &nodes[0], &nodes[2]).await;
    let (rae, ann, cy) = (key(&nodes[0]), key(&nodes[1]), key(&nodes[2]));
    let events = nodes[1].events().await;
    // Ann's General route loses every unordered send to it.
    sim.fault(Fault::UnprovenUnorderedHops {
        node: 1,
        unproven: true,
    });
    // The DM arrives by the app path, before a DHT write could carry it.
    send(&nodes[0], &ann, BODY).await;
    assert!(
        eventually(&sim, SET.min / 2, || holds_message(&nodes[1], &rae, BODY)).await,
        "Rae's DM reaches Ann over her Ordered route at once"
    );
    // Rae's removal of Cy rotates her presence key; its FriendsKey goes to
    // Ann by app_call only (no durable copy).
    nodes[0]
        .ok(IpcRequest::FriendRemove { public_key: cy })
        .await;
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    let away = || async {
        events.any(|event| {
            matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                peer_key, snapshot, ..
            }) if *peer_key == rae && snapshot.status.as_deref() == Some("away"))
        })
    };
    assert!(
        eventually(&sim, LIMIT, away).await,
        "Ann opens Rae's presence under the rotated key, delivered over ordered hops"
    );
}
