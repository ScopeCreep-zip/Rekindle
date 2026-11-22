//! A subject's sections, in order, from Host state (design §2.3–§2.8). A
//! friend's page never lists communities (threat model I8, I11).

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::friends::{group_label, Friend};
use rekindle_types::dto::infoview::{
    InfoviewAction, InfoviewCommunityRow, InfoviewPlayingRow, InfoviewSection, InfoviewServerRow,
    InfoviewSubject, InfoviewView, JoinSource,
};
use rekindle_types::event::EventInfo;
use rekindle_types::friend::UserStatus;
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::game_server::GameServerInfo;

use crate::daemon::dispatch::{adapter, runtime_refused, DaemonContext};

/// Feed entries on our own page; "View More" pages the rest.
const FEED: u16 = 10;

/// `subject`'s view; `subject` is resolved (`super::resolved`).
pub(super) async fn view(
    ctx: &DaemonContext,
    subject: &InfoviewSubject,
) -> Result<InfoviewView, IpcResponse> {
    let sections = match subject {
        // `super::resolved` never yields `Selection`.
        InfoviewSubject::Own | InfoviewSubject::Selection => own(ctx).await?,
        InfoviewSubject::Friend { public_key } => friend(ctx, public_key).await?,
        InfoviewSubject::FriendGroup { group_id } => vec![group(ctx, *group_id)?],
        InfoviewSubject::Community { community } => community_page(ctx, community).await?,
    };
    Ok(InfoviewView {
        subject: subject.clone(),
        sections,
    })
}

/// Our own page: the feed, our communities, friends in game. IV3's
/// `InstalledGamesGet` (`wt/iv3`) adds the installed games after these at
/// merge.
async fn own(ctx: &DaemonContext) -> Result<Vec<InfoviewSection>, IpcResponse> {
    Ok(vec![
        activity(ctx).await?,
        communities(ctx).await?,
        InfoviewSection::NowPlaying {
            rows: playing(ctx, |_| true)?,
        },
    ])
}

async fn activity(ctx: &DaemonContext) -> Result<InfoviewSection, IpcResponse> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let mut rows = db
        .call(move |conn| rekindle_db::repo::activity::page(conn, &owner, None, FEED + 1))
        .await
        .map_err(|e| IpcResponse::error(500, format!("activity feed not read: {e}")))?;
    let more_rows = rows.len() > usize::from(FEED);
    rows.truncate(usize::from(FEED));
    let more = more_rows
        .then(|| {
            rows.last()
                .map(|r| InfoviewAction::ActivityMore { before: r.id })
        })
        .flatten();
    Ok(InfoviewSection::Activity {
        entries: crate::host::activity::items(ctx, rows),
        more,
    })
}

/// Our communities, each with its per-channel unread counts summed here.
async fn communities(ctx: &DaemonContext) -> Result<InfoviewSection, IpcResponse> {
    let host = adapter(ctx);
    let mut rows = Vec::new();
    for overview in crate::daemon::dispatch::community::overviews(ctx)? {
        let counts = host
            .unread_counts(&overview.anchor)
            .await
            .map_err(|e| runtime_refused(&e, "unread counts"))?;
        let next_event = if overview.removed.is_some() {
            None
        } else {
            events(ctx, &overview.anchor).await?.into_iter().next()
        };
        rows.push(InfoviewCommunityRow {
            unread: counts.iter().map(|c| c.unread_count).sum(),
            mentions: counts.iter().map(|c| c.mention_count).sum(),
            next_event,
            action: InfoviewAction::OpenCommunity {
                community: overview.anchor.clone(),
            },
            anchor: overview.anchor,
            name: overview.name,
        });
    }
    Ok(InfoviewSection::Communities { rows })
}

/// The community's events not yet ended, soonest first; none when we may
/// not see its channels. An event's times are Unix seconds: the composer
/// enters them so (`src/components/community/CreateEventModal.tsx:75`,
/// `Math.floor(d.getTime() / 1000)`) and the community window compares
/// them so (`src/windows/community_window/state.ts:191`).
async fn events(ctx: &DaemonContext, anchor: &str) -> Result<Vec<EventInfo>, IpcResponse> {
    match adapter(ctx).events(anchor).await {
        Ok(mut events) => {
            let now = ctx.clock.now_ms() / 1000;
            events.retain(|e| e.end_time.unwrap_or(e.start_time) >= now);
            events.sort_by_key(|e| e.start_time);
            Ok(events)
        }
        Err(GovernanceRuntimeError::PermissionDenied) => Ok(Vec::new()),
        Err(e) => Err(runtime_refused(&e, "events")),
    }
}

fn accepted(f: &FriendState) -> bool {
    f.friendship_state == FriendshipState::Accepted
}

/// Accepted friends `keep` selects that are in game, by game then name.
fn playing(
    ctx: &DaemonContext,
    keep: impl Fn(&FriendState) -> bool,
) -> Result<Vec<InfoviewPlayingRow>, IpcResponse> {
    let mut rows = ctx.require_session(|s| {
        s.friends
            .values()
            .filter(|f| accepted(f) && keep(f))
            .filter_map(|f| {
                let game = f.game_info.as_ref()?;
                Some(InfoviewPlayingRow {
                    friend: Friend::of(f),
                    game_id: game.game_id,
                    game_name: game.game_name.clone(),
                    icon_key: game.icon_key.clone(),
                    elapsed_seconds: game.elapsed_seconds,
                    server_info: game.server_info.clone(),
                    join: InfoviewAction::JoinServer {
                        game_id: Some(game.game_id),
                        server: game.server_address.clone(),
                        source: JoinSource::FriendPresence,
                        joinable: game.joinable,
                        joinable_reason: game.joinable_reason,
                    },
                })
            })
            .collect::<Vec<_>>()
    })?;
    rows.sort_by(|a, b| {
        (&a.game_name, &a.friend.display_name).cmp(&(&b.game_name, &b.friend.display_name))
    });
    Ok(rows)
}

/// A friend's page (design §2.7): header, bio, their game with Join. No
/// community row: which communities we share is never computed (I8, I11).
///
/// Not composed on this base, because the Host holds none of it yet: the
/// call with them (calls still run in the desktop shell until E4 moves them
/// into the Host), file transfers with them (no transfer table exists), and
/// their recent media (no direct message row carries an attachment record;
/// `rekindle_db::repo::messages::attachments` reads channel rows only).
/// Each joins this page at the step that gives the Host its source.
async fn friend(ctx: &DaemonContext, key: &str) -> Result<Vec<InfoviewSection>, IpcResponse> {
    let state = ctx
        .require_session(|s| s.friends.get(key).cloned())?
        .ok_or_else(|| IpcResponse::error(404, "not a friend"))?;
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let peer = key.to_owned();
    let (blocked, bio) = db
        .call(move |conn| {
            Ok((
                rekindle_db::repo::blocked::is_blocked(conn, &owner, &peer)?,
                rekindle_db::repo::bio::friend(conn, &owner, &peer)?,
            ))
        })
        .await
        .map_err(|e| IpcResponse::error(500, format!("friend not read: {e}")))?;
    let mut sections = vec![InfoviewSection::FriendHeader {
        friend: Box::new(Friend::of(&state)),
        blocked,
        actions: vec![
            InfoviewAction::OpenChat {
                public_key: key.to_owned(),
            },
            InfoviewAction::OpenProfile {
                public_key: key.to_owned(),
            },
        ],
    }];
    if let Some(text) = bio.filter(|b| !b.is_empty()) {
        sections.push(InfoviewSection::Bio { text });
    }
    sections.push(InfoviewSection::NowPlaying {
        rows: playing(ctx, |f| f.public_key == key)?,
    });
    Ok(sections)
}

/// A friend group's members (design §2.8), online first.
fn group(ctx: &DaemonContext, group_id: Option<i64>) -> Result<InfoviewSection, IpcResponse> {
    ctx.require_session(|s| {
        let mut members: Vec<&FriendState> = s
            .friends
            .values()
            .filter(|f| f.group_id == group_id)
            .collect();
        members.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        let label = group_label(members.first().and_then(|f| f.group.as_deref()));
        let (online, offline): (Vec<_>, Vec<_>) = members
            .into_iter()
            .partition(|f| accepted(f) && f.status != UserStatus::Offline);
        InfoviewSection::GroupMembers {
            label,
            online: online.into_iter().map(Friend::of).collect(),
            offline: offline.into_iter().map(Friend::of).collect(),
        }
    })
}

/// A community's page (design §2.8): header, events, game servers.
async fn community_page(
    ctx: &DaemonContext,
    community: &str,
) -> Result<Vec<InfoviewSection>, IpcResponse> {
    let anchor = ctx.resolve_community(community)?.anchor;
    let overview = crate::daemon::dispatch::community::overviews(ctx)?
        .into_iter()
        .find(|o| o.anchor == anchor)
        .ok_or_else(|| IpcResponse::error(404, "not a member"))?;
    let mut sections = vec![InfoviewSection::CommunityHeader {
        anchor: anchor.clone(),
        name: overview.name,
        description: overview.description,
        member_count: overview.member_count,
        online_count: u32::try_from(crate::host::adapters::presence::view::present_count(
            ctx, &anchor,
        ))
        .unwrap_or(u32::MAX),
        removed: overview.removed.clone(),
    }];
    if overview.removed.is_some() {
        return Ok(sections);
    }
    sections.push(InfoviewSection::Events {
        events: events(ctx, &anchor).await?,
    });
    sections.push(pinned(ctx, &anchor).await?);
    let servers = match adapter(ctx).game_servers(&anchor).await {
        Ok(servers) => servers,
        Err(GovernanceRuntimeError::PermissionDenied) => Vec::new(),
        Err(e) => return Err(runtime_refused(&e, "game servers")),
    };
    sections.push(InfoviewSection::GameServers {
        servers: servers.into_iter().map(server_row).collect(),
    });
    Ok(sections)
}

/// A community's game server as its row: `Join` always, its source the
/// community, judged by the Host's one launch check
/// (`journal_entry::not_joinable`, the check a friend's `GameInfoState`
/// carries): a game id that is not numeric names nothing to launch.
fn server_row(s: GameServerInfo) -> InfoviewServerRow {
    let game_id = s.game_id.parse().ok();
    let joinable_reason = crate::host::journal_entry::not_joinable(game_id, Some(&s.address));
    InfoviewServerRow {
        join: InfoviewAction::JoinServer {
            game_id,
            server: Some(s.address.clone()),
            source: JoinSource::Community,
            joinable: joinable_reason.is_none(),
            joinable_reason,
        },
        label: s.label,
        address: s.address,
        game_id: s.game_id,
    }
}

/// Pins on our community page: the newest this many.
const PINS: usize = 10;

/// The community's pinned messages across the channels we may view, newest
/// pin first. A channel we may not view adds none.
async fn pinned(ctx: &DaemonContext, anchor: &str) -> Result<InfoviewSection, IpcResponse> {
    let host = adapter(ctx);
    let mut pins = Vec::new();
    for channel in crate::daemon::dispatch::channel::channel_overviews(ctx, anchor) {
        match host.channel_pins(anchor, &channel.id).await {
            Ok(found) => pins.extend(found),
            Err(GovernanceRuntimeError::PermissionDenied) => {}
            Err(e) => return Err(runtime_refused(&e, "channel pins")),
        }
    }
    pins.sort_by(|a, b| b.pinned_at.cmp(&a.pinned_at));
    pins.truncate(PINS);
    Ok(InfoviewSection::Pinned { pins })
}

#[cfg(test)]
mod tests {
    use rekindle_types::friend::NotJoinableReason;

    use super::*;

    /// A community's server row always carries `Join`, sourced from the
    /// community, even when its game id is not numeric: then it is not
    /// joinable and says why; a launchable game on the server is joinable.
    #[test]
    fn a_community_server_row_joins_from_the_community() {
        let info = |game_id: &str| GameServerInfo {
            id: format!("gs_{}", "0".repeat(32)),
            game_id: game_id.into(),
            label: "Frag Night".into(),
            address: "203.0.113.7:27015".into(),
            added_by: "ab".repeat(32),
            created_at: 1,
        };
        assert_eq!(
            server_row(info("4100")).join,
            InfoviewAction::JoinServer {
                game_id: Some(4_100),
                server: Some("203.0.113.7:27015".into()),
                source: JoinSource::Community,
                joinable: true,
                joinable_reason: None,
            }
        );
        assert_eq!(
            server_row(info("quake")).join,
            InfoviewAction::JoinServer {
                game_id: None,
                server: Some("203.0.113.7:27015".into()),
                source: JoinSource::Community,
                joinable: false,
                joinable_reason: Some(NotJoinableReason::CannotLaunch),
            }
        );
    }
}
