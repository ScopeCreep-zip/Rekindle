//! A presence key lost on the way is attempted again (live A/B finding
//! 36): to an accepted friend the `FriendsKey` is an entry of our DM chain
//! (plan RV2c), re-hinted until the friend's `DmAck`, so the friend's
//! presence lights up once it lands, not at the next rotation.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Writes};
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start, Events};
use crate::pair::{accept, befriend, is_accepted_friend, key, request, LIMIT};

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

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

/// Linux live trace (a): the accepter's status is on her profile before the
/// requester holds her presence key, so the read at the accept drops it.
/// The requester shows her online once the key lands, from the record, not
/// at her next beat (none lands here: her STATUS writes fail throughout).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_status_that_lands_before_the_key_is_shown_when_the_key_does() {
    let (sim, mut nodes) = start(2, 92).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let (rae, ann) = (key(&nodes[0]), key(&nodes[1]));
    let events = nodes[0].events().await;
    request(&sim, &nodes[0], &nodes[1]).await;
    sim.fault(status_fails(1));
    // Rae takes Ann's accept but her answer is lost, and so is every frame
    // of hers: her first contact is never proven while it lasts.
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    // The accept's second path (plan RV2) is held back too: Ann's write of
    // it into Rae's reply inbox lands below consensus, so the stored write
    // cannot prove her first contact either.
    sim.fault(reply_slot_fails(1));
    accept(&nodes[1], &nodes[0]).await;
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await);
    // Plan RV2c: Ann's key is an entry of her DM chain, which waits for no
    // proof of her first contact (the HOL1 wait this test once asserted
    // was the receipted row's); Rae holds the session the accept opened,
    // so she opens it with every frame of hers still lost.
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &ann, "online") }).await,
        "Rae shows Ann online once her key lands, her own frames all lost"
    );
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    // Heal ends the reply-slot fault; Ann's STATUS writes keep failing.
    sim.fault(Fault::Heal);
    sim.fault(status_fails(1));
    assert!(eventually(&sim, LIMIT, || is_accepted_friend(&nodes[1], &rae)).await);
}

fn status_fails(node: usize) -> Fault {
    Fault::WriteFails {
        node,
        of: Writes::Subkey {
            subkey: rekindle_types::dht_layout::profile::STATUS,
            record: None,
        },
    }
}

fn reply_slot_fails(node: usize) -> Fault {
    Fault::WriteFails {
        node,
        of: Writes::Subkey {
            subkey: rekindle_types::dht_layout::request_inbox::REQUEST,
            record: None,
        },
    }
}
