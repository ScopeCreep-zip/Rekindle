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

fn activity_entry(id: i64, text: &str) -> rekindle_types::activity::ActivityListItem {
    rekindle_types::activity::ActivityListItem {
        id,
        at: 0,
        kind: rekindle_types::activity::ActivityKind::StatusMessage {
            friend: "ab12ab12ab12ab12ab12ab12ab12ab12".into(),
            text: text.into(),
        },
        display_name: Some("Ann".into()),
        nickname: None,
        identity_pin_mismatch: false,
    }
}

/// The screen as text, drawn by the App at 100x20.
fn screen(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|line| {
            line.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `ActivityAppended` reaches the open Activity view through the stream
/// handler and the reducer: the new entry goes above the first page, and
/// an entry the view already shows is not added twice.
#[tokio::test]
async fn activity_appended_adds_new_entries_to_the_open_view_once() {
    let (_dir, mut app) = app_on_test_bus().await;
    app.reduce(Action::ShowActivity).unwrap();
    app.reduce(Action::CommandComplete(Box::new(
        crate::action::CommandResult::ActivityLoaded {
            before: None,
            entries: vec![activity_entry(1, "first page")],
        },
    )))
    .unwrap();
    let action = app.subscription_event_to_action(SubscriptionEvent::System(
        rekindle_types::subscription_events::SystemEvent::ActivityAppended {
            entries: vec![
                activity_entry(2, "just now"),
                activity_entry(1, "first page"),
            ],
        },
    ));
    assert!(matches!(action, Some(Action::Render)));
    while let Ok(queued) = app.action_rx.try_recv() {
        if matches!(queued, Action::SubscriptionEvent(_)) {
            app.reduce(queued).unwrap();
        }
    }
    let text = screen(&mut app);
    let new_row = text
        .find("\"just now\"")
        .expect("the appended entry is shown");
    let old_row = text.find("\"first page\"").expect("the first page stays");
    assert!(new_row < old_row, "newest first:\n{text}");
    assert_eq!(text.matches("\"first page\"").count(), 1, "{text}");
    assert!(text.contains("Activity (2)"), "{text}");
}

/// The Host's refusal of `ActivityList` is a toast, not swallowed: the
/// test bus has no daemon behind it, so the request is answered with an
/// error.
#[tokio::test]
async fn a_host_error_on_the_activity_list_is_a_toast() {
    let (_dir, mut app) = app_on_test_bus().await;
    app.reduce(Action::ShowActivity).unwrap();
    let failed = tokio::time::timeout(std::time::Duration::from_secs(5), app.action_rx.recv())
        .await
        .expect("the request is answered")
        .unwrap();
    assert!(
        matches!(failed, Action::CommandFailed { ref context, .. } if context == "activity"),
        "{failed:?}"
    );
    app.reduce(failed).unwrap();
    let toasts = app.notifications.visible();
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert!(toasts[0].0.starts_with("activity: "), "{toasts:?}");
    assert_eq!(toasts[0].1, ToastLevel::Error);
}

/// `ServersViewChanged` reaches the open Servers view through the stream
/// handler and the reducer (plan E7.2 G3): the pushed view replaces the
/// list, rows with their verdicts.
#[tokio::test]
async fn servers_view_changed_replaces_the_open_servers_view() {
    use rekindle_types::dto::servers::{
        Joinability, ServerCategory, ServerCategoryKind, ServerJoin, ServerRefusal, ServerRow,
        ServerVerdict, ServersView,
    };
    use rekindle_types::game_server::JoinSource;
    let (_dir, mut app) = app_on_test_bus().await;
    app.reduce(Action::ShowServers).unwrap();
    assert_eq!(app.breadcrumb(), "Servers");
    let row = ServerRow {
        game_id: Some(4181),
        game_name: "Counter-Strike 2".into(),
        icon_key: None,
        address: "198.51.100.2:27015".into(),
        label: "Scrims".into(),
        source: JoinSource::Favourite,
        friend: None,
        last_seen: None,
        join: ServerJoin {
            game_id: Some(4181),
            server: Some("198.51.100.2:27015".into()),
            launch_source: JoinSource::Favourite,
            joinability: Joinability::Joinable,
        },
        info: rekindle_types::dto::servers::ServerInfo::NotQueried,
        add_favourite: ServerVerdict::refused(ServerRefusal::AlreadyFavourite),
        remove: ServerVerdict::Allowed,
    };
    let view = ServersView {
        categories: vec![ServerCategory {
            kind: ServerCategoryKind::Favourites,
            community: None,
            rows: vec![row],
        }],
        refresh: ServerVerdict::Allowed,
    };
    let action = app.subscription_event_to_action(SubscriptionEvent::System(
        rekindle_types::subscription_events::SystemEvent::ServersViewChanged { view },
    ));
    assert!(matches!(action, Some(Action::Render)));
    while let Ok(queued) = app.action_rx.try_recv() {
        if matches!(queued, Action::SubscriptionEvent(_)) {
            app.reduce(queued).unwrap();
        }
    }
    let text = screen(&mut app);
    assert!(text.contains("Servers (1)"), "{text}");
    assert!(text.contains("Favourites (1)"), "{text}");
    assert!(text.contains("\"Scrims\""), "{text}");
    assert!(
        text.contains("[Add to favourites: already a favourite]"),
        "{text}"
    );
}

/// `InfoviewEvent::Updated` to the TUI's window reaches the open Infoview
/// through the stream handler and the reducer (plan E7.15 IV7): the pushed
/// view replaces the page, sections in the Host's order.
#[tokio::test]
async fn infoview_updated_replaces_the_open_infoview() {
    use rekindle_types::dto::infoview::{
        InfoviewEvent, InfoviewSection, InfoviewSubject, InfoviewView, TUI_WINDOW,
    };
    use rekindle_types::subscription_events::SystemEvent;
    let (_dir, mut app) = app_on_test_bus().await;
    let subject = InfoviewSubject::Friend {
        public_key: "ab".repeat(32),
    };
    app.reduce(Action::ShowInfoview {
        subject: subject.clone(),
    })
    .unwrap();
    assert_eq!(app.breadcrumb(), "Friends > abababab…abab > Infoview");
    let view = InfoviewView {
        subject,
        sections: vec![InfoviewSection::Bio {
            text: "fragging since 2004".into(),
        }],
    };
    let action = app.subscription_event_to_action(SubscriptionEvent::System(
        SystemEvent::Infoview(InfoviewEvent::Updated {
            window: TUI_WINDOW.into(),
            view,
        }),
    ));
    assert!(matches!(action, Some(Action::Render)));
    while let Ok(queued) = app.action_rx.try_recv() {
        if matches!(queued, Action::SubscriptionEvent(_)) {
            app.reduce(queued).unwrap();
        }
    }
    let text = screen(&mut app);
    assert!(text.contains("Bio"), "{text}");
    assert!(text.contains("fragging since 2004"), "{text}");
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
