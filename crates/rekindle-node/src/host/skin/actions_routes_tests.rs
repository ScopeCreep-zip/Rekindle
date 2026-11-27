//! Skin action routes: each built-in pack's notifications control reaches the inbox,
//! and search opens in the window's recorded view.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::local_search::LocalSearchScope;
use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_action::{EngineMenu, EngineView, SkinAction as A};
use rekindle_types::dto::skin_run::{ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::dto::skin_tree::ShellNode;

use super::actions::{addressed, resolve, Resolved, Views};
use super::actions_test_support::{run, SELECTED_FRIEND_KEY};
use super::reads::Reads;
use super::SkinState;

fn press_for(nodes: &[ShellNode], matches: &impl Fn(&A) -> bool) -> Option<String> {
    nodes.iter().find_map(|node| {
        if node.triggers.iter().any(|t| t.presses() && matches(&t.run)) {
            Some(node.id.clone())
        } else {
            press_for(&node.children, matches)
        }
    })
}

#[test]
fn every_built_in_skin_routes_its_notifications_control_to_the_inbox() {
    let inbox = ShellOp::OpenPanel(ShellPanel::Notifications);
    let skin = SkinState::default();
    let resolve_press = |window: &str, node: String, action: A| {
        let request = IpcRequest::RunSkinAction {
            window: window.into(),
            node,
            action,
        };
        let Ok(Resolved::Run { run, .. }) = resolve(&skin, &Reads::default(), request) else {
            panic!("the notifications control did not resolve to a shell operation");
        };
        addressed(&skin.launchers, window, run)
    };
    for pack in &rekindle_skin::builtin::PACKS {
        let address = blake3::hash(pack.bytes).to_hex();
        let served = rekindle_skin::serve::stylesheet(pack.bytes, address.as_str()).unwrap();
        let mut mains = served
            .shells
            .iter()
            .filter(|shell| shell.window == SkinWindow::Main);
        let main = mains.clone().find(|shell| shell.popup.is_none()).unwrap();
        let (shell, menu_node) = mains
            .find_map(|shell| {
                press_for(&shell.root, &|a| *a == A::Menu(EngineMenu::File))
                    .map(|node| (shell, node))
            })
            .unwrap_or_else(|| panic!("{} has no Rekindle menu control", pack.id));
        let item = shell
            .menus
            .items(EngineMenu::File)
            .iter()
            .find(|item| item.label == "Notifications")
            .unwrap_or_else(|| panic!("{} has no Notifications menu item", pack.id));
        let (window, expected) = if let Some(popup) = &shell.popup {
            assert!(
                press_for(&main.root, &|a| {
                    matches!(a, A::LaunchPopup(target) if target.popup == popup.name)
                })
                .is_some(),
                "{} has no launcher for {}",
                pack.id,
                popup.name
            );
            (
                format!("popup-buddy-list-{}", popup.name),
                (
                    SkinRun::Launcher {
                        window: "buddy-list".into(),
                    },
                    Some(("buddy-list".into(), inbox.clone())),
                ),
            )
        } else {
            ("buddy-list".into(), (SkinRun::Shell(inbox.clone()), None))
        };
        assert_eq!(
            resolve_press(&window, menu_node, item.run.clone()),
            expected,
            "{}",
            pack.id
        );

        if pack.id == "symbiosis" {
            let node = press_for(&main.root, &|a| *a == A::OpenNotifications);
            assert_eq!(node.as_deref(), Some("ButtonNotificationsTab"));
            assert_eq!(
                resolve_press("buddy-list", node.unwrap(), A::OpenNotifications),
                (SkinRun::Shell(inbox.clone()), None)
            );
        }
    }
    assert_eq!(
        run(&A::ShowView(EngineView::Notifications)),
        SkinRun::Shell(inbox)
    );
}

/// `SearchFor` pressed in `window` (SK2-G §8), as `resolve` maps it.
fn search_for(skin: &SkinState, window: &str) -> SkinRun {
    let pressed = IpcRequest::RunSkinAction {
        window: window.into(),
        node: "SearchButton".into(),
        action: A::SearchFor,
    };
    match resolve(skin, &Reads::default(), pressed) {
        Ok(Resolved::Run { run, .. }) => run,
        other => panic!("{other:?}"),
    }
}

fn open_search(scope: LocalSearchScope) -> SkinRun {
    SkinRun::Shell(ShellOp::OpenSearch { scope })
}

fn show(views: &Views, window: &str, view: EngineView) {
    views.record(window, &SkinRun::Shell(ShellOp::ShowView(view)), None);
}

#[test]
fn search_for_searches_the_windows_recorded_view() {
    let skin = SkinState::default();
    // The buddy list opens on its friends.
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Friends)
    );
    show(&skin.views, "buddy-list", EngineView::Servers);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Servers)
    );
    show(&skin.views, "buddy-list", EngineView::Files);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Files)
    );
    show(&skin.views, "buddy-list", EngineView::Friends);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Friends)
    );
    // A view switch handed on from a popup is its launcher's view.
    skin.views.record(
        "popup-1",
        &SkinRun::Launcher {
            window: "buddy-list".into(),
        },
        Some(&("buddy-list".into(), ShellOp::ShowView(EngineView::Servers))),
    );
    skin.launchers.set("popup-1", Some("buddy-list"));
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Servers)
    );
    assert_eq!(
        search_for(&skin, "popup-1"),
        open_search(LocalSearchScope::Servers)
    );
    // The pictures and videos have nothing local to search yet (G14).
    show(&skin.views, "buddy-list", EngineView::Videos);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        SkinRun::NotBuiltYet {
            action: A::SearchFor,
            message: "Searching pictures and videos isn't in Rekindle yet (plan E7.2 G14)".into(),
        }
    );
}

#[test]
fn search_for_in_a_chat_window_searches_its_conversation() {
    let skin = SkinState::default();
    skin.selection.set(
        "chat-0123456789abcdef",
        Some(SkinSelection::Friend {
            public_key: SELECTED_FRIEND_KEY.into(),
        }),
    );
    assert_eq!(
        search_for(&skin, "chat-0123456789abcdef"),
        open_search(LocalSearchScope::Conversation {
            conversation_id: SELECTED_FRIEND_KEY.into()
        })
    );
    let group = "0a".repeat(16);
    assert_eq!(
        search_for(&skin, &format!("chat-g{group}")),
        open_search(LocalSearchScope::Conversation {
            conversation_id: group
        })
    );
}
