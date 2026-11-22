//! The Servers view's rows and keys (plan E7.2 G3): each action follows the
//! Host's verdict, a refused one is shown and pressed as its reason.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rekindle_types::dto::servers::{
    Joinability, ServerCategory, ServerCategoryKind as Kind, ServerFriend, ServerJoin,
    ServerRefusal, ServerRow, ServerVerdict, ServersView as HostServersView,
};
use rekindle_types::friend::NotJoinableReason;
use rekindle_types::game_server::JoinSource;

use super::*;

const FAV: &str = "198.51.100.2:27015";
const BOBS: &str = "203.0.113.7:27015";

fn row(source: JoinSource, address: &str, joinability: Joinability) -> ServerRow {
    ServerRow {
        game_id: Some(4181),
        game_name: "Counter-Strike 2".into(),
        icon_key: None,
        address: address.into(),
        label: String::new(),
        source,
        friend: None,
        last_seen: None,
        join: ServerJoin {
            game_id: Some(4181),
            server: Some(address.into()),
            launch_source: source,
            joinability,
        },
        add_favourite: ServerVerdict::Allowed,
        remove: ServerVerdict::Allowed,
    }
}

/// A favourite (already one, removable), a recent visit, and Bob's server
/// he cannot be joined on (not removable).
fn host_view() -> HostServersView {
    let mut fav = row(JoinSource::Favourite, FAV, Joinability::Joinable);
    fav.label = "Scrims".into();
    fav.add_favourite = ServerVerdict::refused(ServerRefusal::AlreadyFavourite);
    let recent = row(JoinSource::Recent, FAV, Joinability::Joinable);
    let mut bob = row(
        JoinSource::FriendPresence,
        BOBS,
        Joinability::NotJoinable {
            reason: NotJoinableReason::CannotLaunch,
        },
    );
    bob.friend = Some(ServerFriend {
        public_key: "ab12ab12ab12ab12ab12ab12ab12ab12".into(),
        display_name: "Bob".into(),
    });
    bob.remove = ServerVerdict::refused(ServerRefusal::NotRemovable);
    let category = |kind, rows| ServerCategory {
        kind,
        community: None,
        rows,
    };
    HostServersView {
        categories: vec![
            category(Kind::Favourites, vec![fav]),
            category(Kind::Recent, vec![recent]),
            category(Kind::Friends, vec![bob]),
        ],
    }
}

fn key(view: &mut ServersView, c: char) -> Option<Action> {
    view.handle_focused_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn loaded() -> ServersView {
    let mut view = ServersView::new();
    view.on_command_result(CommandResult::ServersViewLoaded { view: host_view() })
        .unwrap();
    view
}

fn line_text(r: &ServerRow) -> String {
    row_line(r)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

/// The first row is selected, past its header; `j` skips the next header.
/// A favourite joins with its own source, `f` on it says it already is
/// one, `x` removes the favourite; on the recent row `x` forgets the visit.
#[test]
fn rows_act_on_their_allowed_verdicts_and_skip_headers() {
    let mut view = loaded();
    let Ok(Some(Action::LaunchGame {
        game_id,
        server,
        source,
    })) = view.update(Action::Select)
    else {
        panic!("Enter joins the favourite");
    };
    assert_eq!(
        (game_id, server.unwrap().as_str(), source),
        (4181, FAV, JoinSource::Favourite)
    );
    let Some(Action::ShowToast { message, level }) = key(&mut view, 'f') else {
        panic!("a refused add says why");
    };
    assert_eq!(message, "Add to favourites: already a favourite");
    assert_eq!(level, ToastLevel::Warning);
    assert!(matches!(
        key(&mut view, 'x'),
        Some(Action::RemoveFavourite { game_id: 4181, ref server }) if server.as_str() == FAV
    ));

    key(&mut view, 'j');
    assert!(matches!(
        key(&mut view, 'x'),
        Some(Action::RemoveRecentServer { game_id: 4181, ref server }) if server.as_str() == FAV
    ));
    assert!(matches!(
        key(&mut view, 'f'),
        Some(Action::AddFavourite { game_id: 4181, .. })
    ));
}

/// Bob's row cannot be joined or removed: both say why, neither asks.
#[test]
fn a_refused_join_or_remove_is_its_reason_never_a_request() {
    let mut view = loaded();
    key(&mut view, 'j');
    key(&mut view, 'j');
    let Ok(Some(Action::ShowToast { message, .. })) = view.update(Action::Select) else {
        panic!("a refused join says why");
    };
    assert_eq!(message, "Join: Rekindle cannot start this game on a server");
    let Some(Action::ShowToast { message, .. }) = key(&mut view, 'x') else {
        panic!("a refused remove says why");
    };
    assert_eq!(
        message,
        "Remove: only a favourite or a recent server can be removed"
    );
    assert!(matches!(
        key(&mut view, 'f'),
        Some(Action::AddFavourite { .. })
    ));
}

/// A row shows every action, a refused one with its reason.
#[test]
fn a_row_shows_each_action_disabled_with_its_reason() {
    let view = host_view();
    let fav = line_text(&view.categories[0].rows[0]);
    assert!(
        fav.contains("Counter-Strike 2  198.51.100.2:27015  \"Scrims\""),
        "{fav}"
    );
    assert!(
        fav.ends_with("[Join]  [Add to favourites: already a favourite]  [Remove]"),
        "{fav}"
    );
    let bob = line_text(&view.categories[2].rows[0]);
    assert!(bob.contains("Bob (ab12"), "{bob}");
    assert!(
        bob.contains("[Join: Rekindle cannot start this game on a server]"),
        "{bob}"
    );
    assert!(
        bob.ends_with("[Remove: only a favourite or a recent server can be removed]"),
        "{bob}"
    );
}

/// The Host's push replaces the list: the removed favourite is gone and
/// the selection stays on a row.
#[test]
fn the_pushed_view_replaces_the_list() {
    let mut view = loaded();
    let mut pushed = host_view();
    pushed.categories[0].rows.clear();
    view.on_subscription_event(&SubscriptionEvent::System(
        SystemEvent::ServersViewChanged { view: pushed },
    ))
    .unwrap();
    assert_eq!(view.rows(), 2);
    assert_eq!(view.selected().unwrap().source, JoinSource::Recent);
}
