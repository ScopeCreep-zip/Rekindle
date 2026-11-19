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
