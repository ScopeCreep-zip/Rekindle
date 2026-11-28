//! Compose the Servers view (SK2-G §3) from what this identity holds:
//! `favourite_servers`, `recent_servers`, accepted friends' presence and
//! their held game invites, and each joined community's server list. Every
//! row carries the Host's Join, add-favourite and remove verdicts.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rekindle_db::repo::{favourite_servers, recent_servers};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::servers::{
    Joinability, ServerCategory, ServerCategoryKind as Kind, ServerCommunity, ServerFriend,
    ServerInfo, ServerJoin, ServerRefusal, ServerRow, ServerVerdict, ServersView,
};
use rekindle_types::friend::FriendshipState;
use rekindle_types::game_server::{JoinSource, ServerAddress};

use crate::daemon::dispatch::{adapter, runtime_refused, DaemonContext};

type Favourites = BTreeSet<(u32, String)>;

/// What every row is judged against: our favourites, and what the last
/// refresh learned (plan E7.2 G4).
struct Known {
    favourites: Favourites,
    info: HashMap<super::refresh::InfoKey, ServerInfo>,
}

/// What a row is before its verdicts.
struct Draft {
    game_id: Option<u32>,
    /// The name when the game database has none.
    fallback_name: String,
    address: String,
    label: String,
    source: JoinSource,
    friend: Option<ServerFriend>,
    last_seen: Option<i64>,
    /// A friend's presence verdict (its `joinable_reason`), copied as their
    /// `GameInfoState` holds it; `None` judges the row with the Host's launch check.
    verdict: Option<Joinability>,
}

impl Draft {
    fn row(self, known: &Known) -> ServerRow {
        let favourites = &known.favourites;
        let info = known
            .info
            .get(&(self.game_id, self.address.clone()))
            .cloned()
            .unwrap_or(ServerInfo::NotQueried);
        let entry = self
            .game_id
            .and_then(|id| rekindle_game_detect::GameDatabase::shared().lookup_by_id(id));
        let joinability = self.verdict.unwrap_or_else(|| {
            Joinability::of(crate::host::journal_entry::not_joinable(
                self.game_id,
                Some(&self.address),
            ))
        });
        let add_favourite = match self.game_id {
            None => ServerVerdict::refused(ServerRefusal::NoNumericGame),
            Some(_) if ServerAddress::parse(&self.address).is_err() => {
                ServerVerdict::refused(ServerRefusal::BadAddress)
            }
            Some(id) if favourites.contains(&(id, self.address.clone())) => {
                ServerVerdict::refused(ServerRefusal::AlreadyFavourite)
            }
            Some(_) => ServerVerdict::Allowed,
        };
        let remove = match self.source {
            JoinSource::Favourite | JoinSource::Recent => ServerVerdict::Allowed,
            _ => ServerVerdict::refused(ServerRefusal::NotRemovable),
        };
        ServerRow {
            game_id: self.game_id,
            game_name: entry.map_or(self.fallback_name, |e| e.name.clone()),
            icon_key: entry.and_then(rekindle_game_detect::database::GameEntry::icon_key),
            join: ServerJoin {
                game_id: self.game_id,
                server: Some(self.address.clone()),
                launch_source: self.source,
                joinability,
            },
            address: self.address,
            label: self.label,
            source: self.source,
            friend: self.friend,
            last_seen: self.last_seen,
            info,
            add_favourite,
            remove,
        }
    }
}

fn category(kind: Kind, rows: Vec<Draft>, known: &Known) -> ServerCategory {
    ServerCategory {
        kind,
        community: None,
        rows: rows.into_iter().map(|d| d.row(known)).collect(),
    }
}

/// The view, categories in order: favourites, recent, friends, then one
/// per community we are in (a removed membership lists none).
///
/// # Errors
/// 401 without a session, 500 when the tables cannot be read, the
/// runtime's refusal when a community's list cannot be read.
pub(crate) async fn compose(ctx: &DaemonContext) -> Result<ServersView, IpcResponse> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let (favs, recents) = db
        .call(move |conn| {
            Ok((
                favourite_servers::list(conn, &owner)?,
                recent_servers::list(conn, &owner)?,
            ))
        })
        .await
        .map_err(|e| IpcResponse::error(500, format!("servers not read: {e}")))?;
    let favourites: Favourites = favs
        .iter()
        .map(|f| (f.game_id, f.address.clone()))
        .collect();
    let known = Known {
        favourites,
        info: ctx.servers.refresh.info(),
    };
    let draft = |game_id: u32, address: String, source: JoinSource| Draft {
        game_id: Some(game_id),
        fallback_name: format!("Game {game_id}"),
        address,
        label: String::new(),
        source,
        friend: None,
        last_seen: None,
        verdict: None,
    };
    let favourite_rows = favs
        .into_iter()
        .map(|f| Draft {
            label: f.label,
            ..draft(f.game_id, f.address, JoinSource::Favourite)
        })
        .collect();
    let recent_rows = recents
        .into_iter()
        .map(|r| Draft {
            last_seen: Some(r.last_seen),
            ..draft(r.game_id, r.address, JoinSource::Recent)
        })
        .collect();
    let mut categories = vec![
        category(Kind::Favourites, favourite_rows, &known),
        category(Kind::Recent, recent_rows, &known),
        category(Kind::Friends, friends(ctx)?, &known),
    ];
    for overview in crate::daemon::dispatch::community::overviews(ctx)? {
        let servers = if overview.removed.is_some() {
            Vec::new()
        } else {
            match adapter(ctx).game_servers(&overview.anchor).await {
                Ok(servers) => servers,
                Err(
                    GovernanceRuntimeError::PermissionDenied
                    | GovernanceRuntimeError::PermissionRequired(_),
                ) => Vec::new(),
                Err(e) => return Err(runtime_refused(&e, "game servers")),
            }
        };
        let rows = servers
            .into_iter()
            .map(|s| Draft {
                game_id: s.game_id.parse().ok(),
                fallback_name: s.game_id,
                address: s.address,
                label: s.label,
                source: JoinSource::Community,
                friend: None,
                last_seen: None,
                verdict: None,
            })
            .collect();
        let mut c = category(Kind::Community, rows, &known);
        c.community = Some(ServerCommunity {
            anchor: overview.anchor,
            name: overview.name,
        });
        categories.push(c);
    }
    Ok(ServersView {
        categories,
        refresh: ctx.servers.refresh.verdict(),
    })
}

/// Accepted friends' servers: their presence's server (its verdict as
/// their `GameInfoState` holds it), then their held invites, newest first.
/// An invite from someone no longer an accepted friend is not shown.
fn friends(ctx: &DaemonContext) -> Result<Vec<Draft>, IpcResponse> {
    let invites = ctx.servers.invites();
    ctx.require_session(|s| {
        let accepted: BTreeMap<&str, ServerFriend> = s
            .friends
            .values()
            .filter(|f| f.friendship_state == FriendshipState::Accepted)
            .map(|f| {
                (
                    f.public_key.as_str(),
                    ServerFriend {
                        public_key: f.public_key.clone(),
                        display_name: f.display_name.clone(),
                    },
                )
            })
            .collect();
        let mut rows: Vec<Draft> = s
            .friends
            .values()
            .filter(|f| accepted.contains_key(f.public_key.as_str()))
            .filter_map(|f| {
                let game = f.game_info.as_ref()?;
                let address = game.server_address.clone()?;
                Some(Draft {
                    game_id: Some(game.game_id),
                    fallback_name: game.game_name.clone(),
                    address,
                    label: String::new(),
                    source: JoinSource::FriendPresence,
                    friend: accepted.get(f.public_key.as_str()).cloned(),
                    last_seen: None,
                    verdict: Some(Joinability::of(game.joinable_reason)),
                })
            })
            .collect();
        rows.sort_by(|a, b| (&a.fallback_name, &a.address).cmp(&(&b.fallback_name, &b.address)));
        rows.extend(invites.into_iter().filter_map(|i| {
            let friend = accepted.get(i.peer_key.as_str())?.clone();
            Some(Draft {
                game_id: Some(i.game_id),
                fallback_name: format!("Game {}", i.game_id),
                address: i.server,
                label: String::new(),
                source: JoinSource::Invite,
                friend: Some(friend),
                last_seen: None,
                verdict: None,
            })
        }));
        rows
    })
}
