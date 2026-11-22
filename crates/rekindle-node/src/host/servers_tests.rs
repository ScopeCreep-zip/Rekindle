//! The Servers view through the Host (plan E7.2 G3, SK2-G §3): the view
//! composes favourites, recent visits and a friend's presence with each
//! row's verdicts, and its worker pushes it once per coalescing period
//! after each trigger, a favourite's removal included.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse, ServersRequest};
use rekindle_ipc::Caller;
use rekindle_types::dto::servers::{
    Joinability, ServerCategoryKind as Kind, ServerInfo, ServerRefusal, ServerRow, ServerVerdict,
    ServersView,
};
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::intent::{
    Fact, IntentClass, IntentKind, IntentProof, ProofRequired, Recipient,
};
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
    serde_json::from_value(call(host, IpcRequest::Servers(ServersRequest::View)).await).unwrap()
}

async fn add_favourite(host: &Host) {
    let add = IpcRequest::Servers(ServersRequest::FavouriteAdd {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
        label: "Scrims".into(),
    });
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
    let remove = IpcRequest::Servers(ServersRequest::RecentRemove {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
    });
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

    let remove = IpcRequest::Servers(ServersRequest::FavouriteRemove {
        game_id: CS2,
        server: ServerAddress::parse(FAV).unwrap(),
    });
    call(&host, remove).await;
    let seen = pushes(&mut stream, Duration::from_millis(900)).await;
    assert_eq!(seen.len(), 1, "a removed favourite pushes the view once");
    let favourites = &seen[0].categories[0];
    assert_eq!(favourites.kind, Kind::Favourites);
    assert!(favourites.rows.is_empty());
}

/// A game server that never answers; counts the channels opened to it.
#[derive(Default)]
struct Silent {
    opened: std::sync::atomic::AtomicUsize,
}

struct Never;

#[async_trait::async_trait]
impl rekindle_game_detect::server_query::QueryChannel for Never {
    async fn send(&mut self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }

    async fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        std::future::pending().await
    }
}

#[async_trait::async_trait]
impl rekindle_game_detect::server_query::ServerQuery for Silent {
    async fn open(
        &self,
        _: &str,
    ) -> std::io::Result<Box<dyn rekindle_game_detect::server_query::QueryChannel>> {
        self.opened
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(Never))
    }
}

async fn silent_host(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, std::sync::Arc<Silent>) {
    let silent = std::sync::Arc::new(Silent::default());
    let host = super::direct_tests::started_with(dir, silent.clone()).await;
    let (host, _) = super::direct_tests::ann_on(host).await;
    add_favourite(&host).await;
    (host, silent)
}

fn info(view: &ServersView) -> ServerInfo {
    only_row(view, Kind::Favourites).info.clone()
}

fn opened(silent: &Silent) -> usize {
    silent.opened.load(std::sync::atomic::Ordering::SeqCst)
}

const REFRESH: IpcRequest = IpcRequest::Servers(ServersRequest::Refresh);

/// `ServersRefresh` reviewed and acknowledged: its answer and the class.
async fn refresh(host: &Host) -> (serde_json::Value, Option<IntentClass>) {
    match super::game_totals_tests::run(host, REFRESH).await {
        (IpcResponse::Ok { data, .. }, class) => (data, class),
        (other, _) => panic!("{other:?}"),
    }
}

/// Let the Host's spawned work start, move the paused clock `by`, let what
/// came due finish, and give the clock back: no real wait.
async fn elapse(by: Duration) {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::pause();
    tokio::time::advance(by).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
}

/// No view, favourite, elapsed time or unacknowledged review queries a
/// server: only the committed `ServersRefresh` (a query shows our IP
/// address to it).
#[tokio::test]
async fn nothing_queries_without_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let (host, silent) = silent_host(&dir).await;
    elapse(Duration::from_secs(60)).await;
    let shown = view(&host).await;
    assert_eq!(info(&shown), ServerInfo::NotQueried);
    assert_eq!(shown.refresh, ServerVerdict::Allowed);
    let reviewed = rekindle_ipc::HostApi::call(&*host, Caller::shell(), REFRESH).await;
    assert!(
        matches!(reviewed, IpcResponse::IntentRequired(_)),
        "{reviewed:?}"
    );
    elapse(Duration::from_secs(60)).await;
    assert_eq!(info(&view(&host).await), ServerInfo::NotQueried);
    assert_eq!(opened(&silent), 0);
}

/// A refresh marks the row querying and refuses a second refresh; a server
/// silent for the query timeout is unanswered, and a refresh may start
/// again. The timeout runs on the paused clock.
#[tokio::test]
async fn refresh_times_out_to_unanswered_and_is_refused_while_running() {
    let dir = tempfile::tempdir().unwrap();
    let (host, silent) = silent_host(&dir).await;
    let started = serde_json::json!({ "started": true });
    assert_eq!(
        refresh(&host).await,
        (started.clone(), Some(IntentClass::TrustChanging))
    );
    let key = (Some(CS2), FAV.to_owned());
    let refreshing = &host.ctx.servers.refresh;
    assert_eq!(refreshing.info().get(&key), Some(&ServerInfo::Querying));
    assert_eq!(
        refreshing.verdict(),
        ServerVerdict::refused(ServerRefusal::Refreshing)
    );
    assert_eq!(
        refresh(&host).await.0,
        serde_json::json!({ "started": false })
    );
    elapse(super::servers::refresh::QUERY_TIMEOUT + Duration::from_millis(100)).await;
    let done = view(&host).await;
    assert_eq!(
        info(&done),
        ServerInfo::Unanswered {
            reason: "No reply within 2 s".into()
        }
    );
    assert_eq!(done.refresh, ServerVerdict::Allowed);
    assert_eq!(refresh(&host).await.0, started);
    elapse(Duration::ZERO).await;
    assert_eq!(opened(&silent), 2);
}

/// Refresh is TrustChanging (r9 §7.2: our IP address leaves the app): its
/// review names, one fact each, exactly the servers it will ask, the two
/// A2S rows and not the Dota 2 row, which is never contacted.
#[tokio::test]
async fn the_refresh_review_names_each_server_it_will_ask() {
    let dir = tempfile::tempdir().unwrap();
    let (host, silent) = silent_host(&dir).await;
    for (game_id, address) in [(4100, "198.51.100.3:27015"), (4102, "198.51.100.4:27015")] {
        call(
            &host,
            IpcRequest::Servers(ServersRequest::FavouriteAdd {
                game_id,
                server: ServerAddress::parse(address).unwrap(),
                label: "Ours".into(),
            }),
        )
        .await;
    }
    let IpcResponse::IntentRequired(review) =
        rekindle_ipc::HostApi::call(&*host, Caller::shell(), REFRESH).await
    else {
        panic!("Refresh is reviewed")
    };
    assert_eq!(review.kind, IntentKind::RefreshServers);
    assert_eq!(review.class, IntentClass::TrustChanging);
    assert_eq!(review.proof, ProofRequired::Acknowledge);
    let mut named: Vec<(String, String)> = review
        .facts
        .iter()
        .map(|f| match f {
            Fact::IpAddressLeaves {
                to: Recipient::GameServer { game, address },
            } => (game.clone(), address.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        [
            ("Counter-Strike 2".to_owned(), FAV.to_owned()),
            (
                "Team Fortress 2".to_owned(),
                "198.51.100.3:27015".to_owned()
            ),
        ]
    );
    assert_eq!(opened(&silent), 0, "the review asks nobody");
    let commit = IpcRequest::IntentCommit {
        intent_id: review.intent_id,
        proof: IntentProof::Acknowledged,
    };
    rekindle_ipc::HostApi::call(&*host, Caller::shell(), commit).await;
    elapse(Duration::ZERO).await;
    assert_eq!(opened(&silent), 2, "only the servers named are asked");
}
