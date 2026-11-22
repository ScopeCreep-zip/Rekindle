//! The subscription-event reducer over a real bus: the `App` runs on a
//! `DaemonClient` connected to a `BusServer` this test serves on a
//! temporary socket (`DaemonClient::from_bus`), so no daemon is started.

use std::sync::Arc;

use rekindle_ipc::registry::ClearanceRegistry;
use rekindle_ipc::server::BusServer;
use rekindle_ipc::{generate_keypair, BusClient};
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};
use uuid::Uuid;

use crate::action::{Action, ToastLevel};
use crate::app::App;
use crate::keybinds::KeymapStore;
use crate::theme::ThemeManager;

/// An `App` on a bus served from a temporary socket; the directory lives as
/// long as the app.
async fn app_on_test_bus() -> (tempfile::TempDir, App) {
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server = Arc::new(
        BusServer::bind(
            &sock,
            server_kp.into_inner(),
            Arc::new(tokio::sync::RwLock::new(ClearanceRegistry::new())),
        )
        .unwrap(),
    );
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    let client_kp = generate_keypair().unwrap();
    let bus = BusClient::connect(Uuid::now_v7(), &sock, &server_pub, client_kp.as_inner())
        .await
        .unwrap();
    let client = Arc::new(rekindle_client::DaemonClient::from_bus(bus));
    let config = rekindle_client::config::UserConfig::default();
    let theme = ThemeManager::load(&config.tui.theme).unwrap();
    let keymap = KeymapStore::load().unwrap();
    (dir, App::new(client, &config, theme, keymap))
}

#[tokio::test]
async fn a_system_alert_is_shown_as_a_warning_toast() {
    let (_dir, mut app) = app_on_test_bus().await;
    let action = app.subscription_event_to_action(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "Memory lock refused".into(),
            body: "Secrets may be swapped to disk.".into(),
            subject: None,
        },
    ));
    assert!(matches!(action, Some(Action::Render)));
    assert_eq!(
        app.notifications.visible(),
        vec![(
            "Memory lock refused: Secrets may be swapped to disk.".to_string(),
            ToastLevel::Warning
        )],
    );
}

/// A security alert the daemon raised while unlocking, before the TUI
/// connected (keys not in locked memory), is the first frame of the TUI's
/// subscription and is shown as a toast (plan E7.1).
#[tokio::test]
async fn a_standing_alert_raised_before_the_tui_connected_is_shown() {
    let journal = Arc::new(rekindle_events::SessionJournal::with_rng(Arc::new(
        rekindle_utils::random::OsRandom,
    )));
    journal.begin();
    journal.publish_standing(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "Keys not in locked memory".into(),
            body: "raise ulimit -l".into(),
            subject: None,
        },
    ));
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = generate_keypair().unwrap();
    let mut registry = ClearanceRegistry::new();
    registry.register(
        "tui".into(),
        client_kp.public().try_into().unwrap(),
        rekindle_ipc::message::SecurityLevel::Authenticated,
        rekindle_ipc::AgentType::System,
        vec![],
    );
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server = Arc::new(
        BusServer::bind(
            &sock,
            server_kp.into_inner(),
            Arc::new(tokio::sync::RwLock::new(registry)),
        )
        .unwrap(),
    );
    server.start_event_delivery(journal);
    let accept = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = accept.run().await;
    });
    let bus = BusClient::connect(Uuid::now_v7(), &sock, &server_pub, client_kp.as_inner())
        .await
        .unwrap();
    let client = Arc::new(rekindle_client::DaemonClient::from_bus(bus));
    let mut frames = client.take_event_receiver().unwrap();
    client.subscribe_all(None).await.unwrap();
    let config = rekindle_client::config::UserConfig::default();
    let theme = ThemeManager::load(&config.tui.theme).unwrap();
    let keymap = KeymapStore::load().unwrap();
    let mut app = App::new(client, &config, theme, keymap);
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
        .await
        .expect("the standing alert is sent on subscribe")
        .unwrap();
    app.handle_frame(frame).await;
    assert_eq!(
        app.notifications.visible(),
        vec![(
            "Keys not in locked memory: raise ulimit -l".to_string(),
            ToastLevel::Warning
        )],
    );
}

/// Plan RV1: the Host's route pause reaches the TUI's status line through
/// the subscription reducer.
#[tokio::test]
async fn the_route_pause_reaches_the_status_line() {
    use rekindle_types::subscription_events::{NetworkEvent, RoutePauseCause};
    let (_dir, mut app) = app_on_test_bus().await;
    let action =
        app.subscription_event_to_action(SubscriptionEvent::Network(NetworkEvent::RoutePaused {
            cause: RoutePauseCause::Exhausted,
        }));
    assert!(matches!(action, Some(Action::Render)));
    assert_eq!(
        app.route_alert.text().as_deref(),
        Some("inbound route paused after repeated failures: possible attack")
    );
}
