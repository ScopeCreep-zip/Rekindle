//! The game stage's switch (plan E7.2): with detection off no scanner task
//! exists, and `SetGameDetection` starts and stops it with no restart.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::start_if_enabled;
use crate::host::direct_tests::ann;
use crate::host::game_totals_tests::run;

async fn set_detection(host: &crate::host::Host, owner: [u8; 32], on: bool) {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::preferences::set_game_detection_enabled(c, &o, on))
        .await
        .unwrap();
}

/// The privacy bug (plan E7.2 finding): the Host scanned whatever the
/// setting said. Off at unlock, the stage starts no scanner.
#[tokio::test]
async fn with_detection_off_the_scanner_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    set_detection(&host, owner, false).await;
    let scope = host.ctx.unlock_scope_or_closed();
    let started = start_if_enabled(&host.ctx, &scope, hex::encode(owner))
        .await
        .unwrap();
    assert!(!started);
    assert!(!host.ctx.game.is_scanning(), "no scanner task while off");

    set_detection(&host, owner, true).await;
    assert!(start_if_enabled(&host.ctx, &scope, hex::encode(owner))
        .await
        .unwrap());
    assert!(host.ctx.game.is_scanning());
    host.ctx.game.stop().await;
    assert!(!host.ctx.game.is_scanning());
}

/// Turning detection on and off through the Host takes effect at once,
/// with no restart, and is Routine (local only).
#[tokio::test]
async fn set_game_detection_starts_and_stops_the_scanner_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;

    let (answer, class) = run(&host, IpcRequest::SetGameDetection { enabled: true }).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(data["gameDetectionEnabled"], true);
    assert_ne!(
        class,
        Some(rekindle_types::intent::IntentClass::TrustChanging),
        "detection is local only"
    );
    assert!(host.ctx.game.is_scanning());

    let (answer, _) = run(&host, IpcRequest::SetGameDetection { enabled: false }).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(data["gameDetectionEnabled"], false);
    assert!(!host.ctx.game.is_scanning(), "stopped at once");
}

fn cs2() -> rekindle_game_detect::DetectedGame {
    rekindle_game_detect::DetectedGame {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        process_name: "cs2.exe".into(),
        started_at_epoch_ms: 1_000,
        rich_presence: None,
    }
}

fn shown(host: &crate::host::Host) -> Option<u32> {
    host.ctx.own_presence.view().game.map(|game| game.game_id)
}

/// Detection is split from sharing (plan E7.2): a detected game is shown
/// to friends only while `share_current_game` is on (off by default), never
/// while it is hidden, and turning either back follows at once; a game set
/// by hand is left alone.
#[tokio::test]
async fn a_detected_game_is_shown_only_while_shared_and_not_hidden() {
    use rekindle_game_detect::GameDetectorPublisher as _;
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let ctx = &host.ctx;
    let publisher = crate::host::game::Publisher::new(
        ctx.own_presence.clone(),
        crate::host::game::SessionLog::new(
            std::sync::Arc::clone(&ctx.db),
            hex::encode(owner),
            std::sync::Arc::clone(&ctx.clock),
            std::sync::Arc::new(crate::host::adapters::friends::HostFriends::of(ctx)),
        ),
    );

    publisher.publish_game_status(Some(cs2())).await;
    assert_eq!(shown(&host), None, "sharing is off by default");

    let (answer, class) = run(&host, IpcRequest::SetShareCurrentGame { enabled: true }).await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    assert_eq!(
        class,
        Some(rekindle_types::intent::IntentClass::TrustChanging)
    );
    // The handler reapplies through the running stage; this publisher is
    // not it, so apply as the stage would.
    publisher.reapply().await;
    assert_eq!(shown(&host), Some(4181));

    let o = hex::encode(owner);
    ctx.db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::hidden_games::hide(c, &o, 4181))
        .await
        .unwrap();
    publisher.reapply().await;
    assert_eq!(shown(&host), None, "a hidden game is never shown");

    // A game set by hand is not the detector's to clear.
    let (answer, _) = run(
        &host,
        IpcRequest::GamePresenceSet {
            game_name: "Quake".into(),
            game_id: Some(12),
            elapsed_seconds: 0,
            server_address: None,
        },
    )
    .await;
    assert!(matches!(answer, IpcResponse::Ok { .. }), "{answer:?}");
    publisher.withdraw().await;
    assert_eq!(shown(&host), Some(12));
}

/// Through the Host: the hide list round-trips, game id 0 is refused, and
/// unhiding (it widens what friends see) is TrustChanging.
#[tokio::test]
async fn the_hide_list_round_trips_through_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = ann(&dir).await;
    let (answer, _) = run(&host, IpcRequest::GameHide { game_id: 4181 }).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(data[0]["gameId"], 4181);
    assert_eq!(data[0]["gameName"], "Counter-Strike 2");
    let (answer, class) = run(&host, IpcRequest::GameUnhide { game_id: 4181 }).await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert!(data.as_array().unwrap().is_empty());
    assert_eq!(
        class,
        Some(rekindle_types::intent::IntentClass::TrustChanging)
    );
    let (answer, _) = run(&host, IpcRequest::GameHide { game_id: 0 }).await;
    assert!(matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
}
