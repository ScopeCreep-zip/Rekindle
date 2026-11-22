//! The Servers view through the Host (plan E7.2 G3, SK2-G §3): the view
//! composes favourites, recent visits and a friend's presence with each
//! row's verdicts, and its worker pushes it once per coalescing period
//! after each trigger, a favourite's removal included.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::Caller;
use rekindle_types::dto::servers::{
    Joinability, ServerCategoryKind as Kind, ServerRefusal, ServerRow, ServerVerdict, ServersView,
};
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::subscription_events::{
    FriendEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};

use super::infoview_tests::ann_and_bob;
use super::Host;

const CS2: u32 = 4181;
const FAV: &str = "198.51.100.2:27015";
const BOBS: &str = "203.0.113.7:27015";

async fn call(host: &Host, request: IpcRequest) -> serde_json::Value {
    match rekindle_ipc::HostApi::call(host, Caller::shell(), request).await {
        IpcResponse::Ok { data, .. } => data,
        other => panic!("{other:?}"),
    }
}

async fn view(host: &Host) -> ServersView {
    serde_json::from_value(call(host, IpcRequest::ServersView).await).unwrap()
}

async fn add_favourite(host: &Host) {
    let add = IpcRequest::FavouriteServerAdd {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
        label: "Scrims".into(),
    };
    call(host, add).await;
}

/// The Host's own launch check for a row it judges itself.
fn launch_check(game_id: u32, address: &str) -> Joinability {
    Joinability::of(super::journal_entry::not_joinable(
        Some(game_id),
        Some(address),
    ))
}

fn only_row(view: &ServersView, kind: Kind) -> &ServerRow {
    let category = view.categories.iter().find(|c| c.kind == kind).unwrap();
    assert_eq!(category.rows.len(), 1, "{kind:?}: {:?}", category.rows);
    assert!(category.community.is_none());
    &category.rows[0]
}

/// Bob plays on a server he shares, we hold a favourite and we visited
/// the same server: three categories in order, each row with its source,
/// its Join and the Host's add-favourite and remove verdicts.
#[tokio::test]
async fn the_view_composes_favourites_recent_and_friends_with_their_verdicts() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    add_favourite(&host).await;
    super::servers::visited(&host.ctx, CS2, &ServerAddress::parse(FAV).unwrap()).await;
    let view = view(&host).await;
    let kinds: Vec<Kind> = view.categories.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [Kind::Favourites, Kind::Recent, Kind::Friends]);

    let fav = only_row(&view, Kind::Favourites);
    assert_eq!((fav.game_id, fav.address.as_str()), (Some(CS2), FAV));
    assert_eq!(
        (fav.label.as_str(), fav.source),
        ("Scrims", JoinSource::Favourite)
    );
    assert_eq!(fav.join.launch_source, JoinSource::Favourite);
    assert_eq!(fav.join.server.as_deref(), Some(FAV));
    assert_eq!(fav.join.joinability, launch_check(CS2, FAV));
    assert_eq!(
        fav.add_favourite,
        ServerVerdict::refused(ServerRefusal::AlreadyFavourite)
    );
    assert_eq!(fav.remove, ServerVerdict::Allowed);
    assert!(fav.friend.is_none() && fav.last_seen.is_none());

    let recent = only_row(&view, Kind::Recent);
    assert_eq!(
        (recent.address.as_str(), recent.source),
        (FAV, JoinSource::Recent)
    );
    assert_eq!(recent.join.launch_source, JoinSource::Recent);
    assert_eq!(recent.label, "");
    assert!(recent.last_seen.is_some());
    assert_eq!(recent.join.joinability, launch_check(CS2, FAV));
    assert_eq!(
        recent.add_favourite,
        ServerVerdict::refused(ServerRefusal::AlreadyFavourite)
    );
    assert_eq!(recent.remove, ServerVerdict::Allowed);

    let friend = only_row(&view, Kind::Friends);
    assert_eq!(
        (friend.game_id, friend.address.as_str()),
        (Some(4_100), BOBS)
    );
    assert_eq!(friend.source, JoinSource::FriendPresence);
    assert_eq!(friend.friend.as_ref().unwrap().public_key, bob);
    // Bob's presence verdict as his GameInfoState holds it, not ours.
    assert_eq!(friend.join.joinability, Joinability::Joinable);
    assert_eq!(friend.add_favourite, ServerVerdict::Allowed);
    assert_eq!(
        friend.remove,
        ServerVerdict::refused(ServerRefusal::NotRemovable)
    );

    // Forgetting the visit empties Recent, which is still sent.
    let remove = IpcRequest::RecentServerRemove {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
    };
    let after: ServersView = serde_json::from_value(call(&host, remove).await).unwrap();
    let recent = after
        .categories
        .iter()
        .find(|c| c.kind == Kind::Recent)
        .unwrap();
    assert!(recent.rows.is_empty());
}

/// The `ServersViewChanged` pushes seen in `window`.
async fn pushes(stream: &mut rekindle_events::EventStream, window: Duration) -> Vec<ServersView> {
    let mut seen = Vec::new();
    let end = tokio::time::Instant::now() + window;
    while let Ok(Some(frame)) = tokio::time::timeout_at(end, stream.next()).await {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::System(SystemEvent::ServersViewChanged { view }) = s.event {
                seen.push(view);
            }
        }
    }
    seen
}

/// Two shown events in one period push the view once; a favourite's
/// removal (no journal event of its own) pushes it once more, without the
/// favourite.
#[tokio::test]
async fn each_trigger_pushes_the_view_once_per_period() {
    let (_dirs, host, _owner, bob) = ann_and_bob().await;
    add_favourite(&host).await;
    let mut stream = host.ctx.journal.subscribe(None);
    // The fixture's and the add's own push settle first.
    pushes(&mut stream, Duration::from_millis(600)).await;

    for nick in ["A", "B"] {
        host.ctx
            .publish_event(SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
                peer_key: bob.clone(),
                nickname: Some(nick.into()),
            }));
    }
    assert_eq!(
        pushes(&mut stream, Duration::from_millis(900)).await.len(),
        1
    );

    let remove = IpcRequest::FavouriteServerRemove {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
    };
    call(&host, remove).await;
    let seen = pushes(&mut stream, Duration::from_millis(900)).await;
    assert_eq!(seen.len(), 1, "a removed favourite pushes the view once");
    let favourites = &seen[0].categories[0];
    assert_eq!(favourites.kind, Kind::Favourites);
    assert!(favourites.rows.is_empty());
}
