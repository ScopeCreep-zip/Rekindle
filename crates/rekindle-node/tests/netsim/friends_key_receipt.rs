//! A presence key is receipted (live A/B finding 36): the `FriendsKey` row
//! goes by `app_call` and settles only on its receipt, so one lost on the
//! way is attempted again and the friend's presence lights up once it
//! lands, not at the next rotation.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start};
use crate::pair::{befriend, key, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_presence_key_is_attempted_again_until_it_lands() {
    let (sim, mut nodes) = start(3, 91).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann", "Cy"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    befriend(&sim, &nodes[0], &nodes[2]).await;
    let (rae, cy) = (key(&nodes[0]), key(&nodes[2]));
    let events = nodes[1].events().await;
    // Rae's removal of Cy rotates her presence key; its FriendsKey to Ann
    // is lost, as is every other frame meanwhile.
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    nodes[0]
        .ok(IpcRequest::FriendRemove { public_key: cy })
        .await;
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    sim.fault(Fault::DropMessages { per_mille: 0 });
    let away = || async {
        events.any(|event| {
            matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                peer_key, snapshot, ..
            }) if *peer_key == rae && snapshot.status.as_deref() == Some("away"))
        })
    };
    assert!(
        eventually(&sim, LIMIT * 20, away).await,
        "Ann opens Rae's presence under the key that was attempted again"
    );
}
