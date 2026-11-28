//! Auto-away ends with its reason (plan E7.4): turning it off, a threshold
//! above our idle time, or no idle reading restores the status it replaced
//! at once; a manual Away stays. Its transition reaches the own-presence
//! source every window reads (`PresenceGet` and `SelfChanged`).

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::HostApi;
use rekindle_types::presence::{IdleState, SessionStatus};
use rekindle_types::subscription_events::{PresenceEvent, StreamFrame, SubscriptionEvent};

use super::apply;
use crate::host::direct_tests::ann;
use crate::host::game_totals_tests::run;

#[tokio::test]
async fn turning_auto_away_off_restores_the_status_it_replaced_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let presence = &host.ctx.own_presence;
    presence.auto_away();
    assert_eq!(presence.status(), SessionStatus::Away);

    let (answer, _) = run(&host, IpcRequest::SetAutoAway { minutes: 0 }).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(data["autoAwayMinutes"], 0);
    assert_eq!(
        presence.status(),
        SessionStatus::Online,
        "restored at once, not at the idle stage's next tick"
    );
    assert!(presence.view().idle != IdleState::AutoAway);
}

#[tokio::test]
async fn a_manual_away_stays_when_auto_away_is_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let presence = &host.ctx.own_presence;
    presence.set_status(SessionStatus::Away).await;
    let (answer, _) = run(&host, IpcRequest::SetAutoAway { minutes: 0 }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert_eq!(
        presence.status(),
        SessionStatus::Away,
        "a picked Away stays"
    );
}

#[tokio::test]
async fn a_threshold_above_our_idle_time_or_no_reading_ends_auto_away() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let presence = &host.ctx.own_presence;
    apply(presence, Some(700), 10);
    assert_eq!(presence.status(), SessionStatus::Away);
    apply(presence, Some(700), 15);
    assert_eq!(
        presence.status(),
        SessionStatus::Online,
        "raised above our idle time"
    );
    apply(presence, Some(700), 10);
    assert_eq!(presence.status(), SessionStatus::Away);
    apply(presence, None, 10);
    assert_eq!(
        presence.status(),
        SessionStatus::Online,
        "no reading: auto-away cannot hold"
    );
}

/// The buddy list hydrates our status from `PresenceGet` and follows
/// `SelfChanged`: the auto-away transition reaches both.
#[tokio::test]
async fn the_auto_away_transition_reaches_the_own_presence_source() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let mut stream = host.subscribe(None).await.unwrap();
    host.ctx.own_presence.auto_away();
    let seen = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = stream.next().await {
            let StreamFrame::Event(s) = frame else {
                continue;
            };
            if let SubscriptionEvent::Presence(PresenceEvent::SelfChanged { snapshot, .. }) =
                &s.event
            {
                if snapshot.status.as_deref() == Some("away") {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(seen, "the auto-away reached SelfChanged");
    let (answer, _) = run(&host, IpcRequest::PresenceGet).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(data["status"], "away");
}
