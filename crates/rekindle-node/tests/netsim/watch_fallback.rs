//! Watches on Veilid 0.5.8 (plan VU2 item 4): a range-only hint of a subkey
//! already reported waits for a network read of it (report marks,
//! `outbound_watch_state.rs:251-297`), and a push lost while the machine
//! slept surfaces through the fallback silence timer (30 s → 300 s,
//! `outbound_watch_state.rs:347-418`), since 0.5.8 has no tick-lag sweep.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{
    GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use crate::harness::{eventually, start, Events, SimNode};
use crate::pair::{befriend, friends, key, LIMIT};

fn status_seen(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

fn game_seen(events: &Events, peer: &str, name: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key,
            snapshot: PresenceSnapshot { game: Some(GameActivity::Playing { game_name, .. }), .. },
            ..
        }) if peer_key == peer && game_name == name)
    })
}

async fn profile_key(node: &SimNode) -> String {
    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(node.ok(IpcRequest::FriendCode).await).expect("the code");
    code.profile_key
}

/// The S7-style frontier race on a friend's profile: Kim's status (subkey
/// 2) reaches Lev alone, with its value, so Lev's Veilid marks it reported
/// and unread. Kim then writes her game (4) and status (2) together; a
/// replica that coalesces them pushes {2, 4} with no value, and 0.5.8
/// reports only 4 until Lev reads 2 from the network. Lev must still show
/// the new status at that hint, not a fallback inspection later.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_coalesced_hint_still_shows_a_subkey_reported_before() {
    let (sim, mut nodes) = start(2, 131).await;
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
        eventually(&sim, LIMIT, || async { status_seen(&events, &kim, "away") }).await,
        "Lev sees Kim away by the watch"
    );
    let game = IpcRequest::GamePresenceSet {
        game_name: "CS2".into(),
        game_id: Some(7),
        elapsed_seconds: 0,
        server_address: None,
    };
    let busy = IpcRequest::PresenceSet {
        status: SessionStatus::Busy,
    };
    tokio::join!(nodes[0].ok(game), nodes[0].ok(busy));
    assert!(
        eventually(&sim, LIMIT, || async { game_seen(&events, &kim, "CS2") }).await,
        "Lev sees Kim's game"
    );
    // One hint carried both subkeys: the status shows with the game, not
    // 30-300 s later at a fallback inspection.
    assert!(
        eventually(&sim, Duration::from_secs(10), || async {
            status_seen(&events, &kim, "busy")
        })
        .await,
        "Lev shows Kim busy at the hint that showed her game"
    );
}

/// A desktop sleep: Lev's machine is suspended (no tick, nothing reaches
/// it) but never goes Offline, and its wall clock moves on. Kim renames
/// herself meanwhile; the push is lost. On waking no sweep runs (0.5.8
/// dropped the tick-lag sweep), so the rename reaches Lev by his watch's
/// fallback inspection, within its 300 s ceiling.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_missed_in_a_sleep_arrives_by_the_fallback_inspection() {
    let (sim, mut nodes) = start(2, 132).await;
    for (node, name) in nodes.iter_mut().zip(["Kim", "Lev"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let kim = key(&nodes[0]);
    // No poll reads the profile while Lev sleeps or after (D4).
    let profile = profile_key(&nodes[0]).await;
    let shows = |name: &'static str| {
        let (lev, kim) = (&nodes[1], kim.clone());
        async move {
            friends(lev)
                .await
                .iter()
                .any(|f| f.public_key == kim && f.display_name == name)
        }
    };
    assert!(
        eventually(&sim, LIMIT, || shows("Kim")).await,
        "Lev shows Kim"
    );
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: true,
    });
    nodes[0]
        .ok(IpcRequest::ProfileSetNickname {
            display_name: "Kimberly".into(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: false,
    });
    assert!(
        eventually(&sim, Duration::from_secs(330), || shows("Kimberly")).await,
        "Lev shows the rename within the fallback ceiling"
    );
    assert_eq!(sim.app_inspects(1, &profile), 0, "no app inspect polled it");
}
