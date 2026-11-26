//! The state the skin capabilities read besides the selection and the
//! stream (moved out of `signals.rs`, which holds the signal inputs and
//! their publication).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_transport::operations::group_dm;
use rekindle_types::friend::FriendshipState;

use super::capability::{Group, LobbyCaps, SkinWorld};
use crate::daemon::dispatch::DaemonContext;

/// The state the capabilities read besides the selection and the stream:
/// the session's friends and communities, our shown game, the
/// own-records stage, and our favourite servers, groups and group invites in the database.
pub(crate) async fn load_world(ctx: &DaemonContext) -> SkinWorld {
    let mut world = SkinWorld::default();
    if let Some(session) = ctx.session.read().as_ref() {
        for f in session.friends.values() {
            if f.friendship_state == FriendshipState::Accepted {
                world.accepted_friends += 1;
                let server = f.game_info.as_ref().and_then(|g| g.server_address.as_ref());
                world.friend_servers += usize::from(server.is_some());
            }
        }
        world.communities = session.communities.len();
    }
    world.own_game = ctx.own_presence.view().game.map(|g| g.game_id);
    world.records_held = ctx.friend_stores.own_records.get()
        == rekindle_transport::operations::friend::OwnRecordsStage::Held;
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return world;
    };
    let now = ctx.clock.now_ms_i64();
    let read = db
        .call(move |conn| {
            use rekindle_db::repo::{
                favourite_servers, group_dm_invites, group_dms, group_lobby, transfers,
            };
            let favourites = favourite_servers::list(conn, &owner)?;
            let groups = group_dms::list(conn, &owner)?;
            let invites = group_dm_invites::list(conn, &owner, now)?;
            let attached: BTreeSet<String> = groups
                .iter()
                .filter_map(|g| g.origin_peer.clone())
                .collect();
            let mut joined = Vec::with_capacity(groups.len());
            for g in groups {
                let others = g.roster.iter().filter(|m| m.identity != owner).count();
                let creator = g.roster.first().is_some_and(|m| m.identity == owner);
                let lobby = group_lobby::current(conn, &owner, &g.group_id)?.map(|row| {
                    let [join, leave, launch, _] = group_dm::lobby::verdicts(&row, &owner);
                    LobbyCaps {
                        open: row.state == group_lobby::LobbyState::Open,
                        join: join.allowed(),
                        leave: leave.allowed(),
                        launch: launch.allowed(),
                    }
                });
                joined.push((
                    g.group_id,
                    Group::Joined {
                        others,
                        creator,
                        lobby,
                    },
                ));
            }
            let pending = invites.into_iter().map(|i| (i.group_id, Group::Pending));
            let favourites: BTreeSet<(u32, String)> = favourites
                .into_iter()
                .map(|f| (f.game_id, f.address))
                .collect();
            let groups: BTreeMap<String, Group> = pending.chain(joined).collect();
            let transfers: BTreeMap<String, _> = transfers::list(conn, &owner)?
                .into_iter()
                .map(|t| (t.attachment_id.clone(), t))
                .collect();
            Ok((favourites, groups, attached, transfers))
        })
        .await;
    match read {
        Ok((favourites, groups, attached, transfers)) => {
            world.favourites = favourites;
            world.groups = groups;
            world.attached = attached;
            world.transfers = transfers;
        }
        Err(e) => tracing::warn!(error = %e, "skin signals: database state unread"),
    }
    world
}
