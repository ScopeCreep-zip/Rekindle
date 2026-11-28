//! Invisible shows no game (prod research batch 7 G1): entering Invisible
//! writes our GAME_INFO subkey empty, so a friend sees the game stop, and
//! leaving it publishes the game we still hold again.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{
    GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use crate::harness::{eventually, start};
use crate::pair::{befriend, key, LIMIT};

fn game_of<'a>(event: &'a SubscriptionEvent, friend: &str) -> Option<&'a GameActivity> {
    match event {
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key,
            snapshot: PresenceSnapshot { game, .. },
            ..
        }) if peer_key == friend => game.as_ref(),
        _ => None,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn going_invisible_clears_the_game_and_coming_back_restores_it() {
    let (sim, mut nodes) = start(2, 92).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let rae = key(&nodes[0]);
    let events = nodes[1].events().await;
    let playing = |events: &crate::harness::Events| {
        events.matching(|event| {
            matches!(
                game_of(event, &rae),
                Some(GameActivity::Playing { game_name, .. }) if game_name == "CS2"
            )
        })
    };
    nodes[0]
        .ok(IpcRequest::GamePresenceSet {
            game_name: "CS2".into(),
            game_id: Some(7),
            elapsed_seconds: 0,
            server_address: None,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { playing(&events) >= 1 }).await,
        "Ann sees Rae playing"
    );
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Invisible,
        })
        .await;
    let stopped =
        || async { events.any(|event| matches!(game_of(event, &rae), Some(GameActivity::Idle))) };
    assert!(
        eventually(&sim, LIMIT, stopped).await,
        "Ann sees Rae's game stop when Rae goes Invisible"
    );
    let before = playing(&events);
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Online,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { playing(&events) > before }).await,
        "Ann sees Rae playing again once Rae is Online"
    );
}
