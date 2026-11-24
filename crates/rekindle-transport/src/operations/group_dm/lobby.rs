//! Game lobbies (plan E7.2 G6, SK2-G §5): a lobby is state inside one group
//! DM. Its host opens it, members join and leave, the host launches it on a
//! server or closes it; each step is a lobby kind (0x0127-0x012B) sealed in a
//! `GroupContent` entry's inner and committed to the writer's group stream
//! with no echo, like a rename. Every member folds the entries it reads
//! into `group_lobbies` (`rekindle_db::repo::group_lobby`); an entry from a
//! writer off the roster is dropped (`rekindle_dm::admit`, the E2.6 rule).
//! A launch never starts a member's game: the member's own confirmed
//! `GameLaunch{source: Lobby}` does (threat model: no auto-action on a peer
//! message).

use rekindle_codec::capnp_codec::direct::{
    DirectBody, GroupId, LobbyId, LobbyLaunch, LobbyOpen, LobbyRef,
};
use rekindle_db::repo::group_lobby::{self, LobbyRow, LobbyState};
use rekindle_dm::admission::GroupFrame;
use rekindle_dm::group::group_id_hex;
use rekindle_dm::{AcceptedGroup, Admission};
use rekindle_types::dto::group_lobby::{
    LobbyRefusal as R, LobbyStatus, LobbyVerdict as V, LobbyView,
};
use rekindle_types::game_server::ServerAddress;
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use super::content::held;
use super::parse_group_id;
use crate::operations::dm::outbox as dm_outbox;
use crate::operations::dm::{client_id_text, mint_client_id};
use crate::operations::friend::{state, FriendError, FriendHost};

/// A lobby step we take on a lobby we already read.
#[derive(Debug, Clone)]
pub enum LobbyStep {
    Join,
    Leave,
    Close,
    Launch(ServerAddress),
}

/// `row` as `me` sees it, each action's verdict decided from the fold.
///
/// # Errors
/// A launched row's server is not a `ServerAddress` (a corrupt row).
pub fn view_of(row: LobbyRow, me: &str) -> Result<LobbyView, FriendError> {
    let [join, leave, launch, close] = verdicts(&row, me);
    let state = match row.state {
        LobbyState::Open => LobbyStatus::Open,
        LobbyState::Closed => LobbyStatus::Closed,
        LobbyState::Launched => LobbyStatus::Launched {
            server: row
                .server
                .and_then(|s| ServerAddress::parse(&s).ok())
                .ok_or_else(|| FriendError::Invalid("a launched lobby names no server".into()))?,
        },
    };
    Ok(LobbyView {
        lobby_id: row.lobby_id,
        host_key: row.host_key,
        game_id: row.game_id,
        members: row.members,
        state,
        join,
        leave,
        launch,
        close,
    })
}

/// `row`'s join, leave, launch and close verdicts for `me` (SK2-G §5 (5)).
#[must_use]
pub fn verdicts(row: &LobbyRow, me: &str) -> [V; 4] {
    let host = row.host_key == me;
    let joined = row.members.iter().any(|m| m == me);
    let open = row.state == LobbyState::Open;
    let join = match () {
        () if !open => V::refused(R::NotOpen),
        () if host => V::refused(R::OwnLobby),
        () if joined => V::refused(R::AlreadyIn),
        () => V::Allowed,
    };
    let leave = match () {
        () if host => V::refused(R::OwnLobby),
        () if !joined => V::refused(R::NotIn),
        () if row.state == LobbyState::Closed => V::refused(R::Closed),
        () => V::Allowed,
    };
    let launch = match () {
        () if !host => V::refused(R::NotHost),
        () if !open => V::refused(R::NotOpen),
        () if row.members.is_empty() => V::refused(R::NoMembers),
        () => V::Allowed,
    };
    let close = match () {
        () if !host => V::refused(R::NotHost),
        () if row.state == LobbyState::Closed => V::refused(R::Closed),
        () => V::Allowed,
    };
    [join, leave, launch, close]
}

fn own(host: &dyn FriendHost) -> Result<String, FriendError> {
    Ok(host.group_dm()?.own_identity())
}

/// The lobby `group_id` (32 hex) shows now (`group_lobby::current`).
///
/// # Errors
/// Not a group we hold, or the lobbies cannot be read.
pub async fn get(host: &dyn FriendHost, group_id: &str) -> Result<Option<LobbyView>, FriendError> {
    let id = parse_group_id(group_id)?;
    held(host, &id).await?;
    current(host, &id).await
}

async fn current(host: &dyn FriendHost, id: &GroupId) -> Result<Option<LobbyView>, FriendError> {
    let (owner, gid, me) = (state::owner(host)?, group_id_hex(id), own(host)?);
    let row = state::db_call!(host, "read group lobby", move |conn| {
        group_lobby::current(conn, &owner, &gid)
    })
    .await?;
    row.map(|r| view_of(r, &me)).transpose()
}

async fn lobby(
    host: &dyn FriendHost,
    id: &GroupId,
    lobby_id: &str,
) -> Result<Option<LobbyView>, FriendError> {
    let (owner, gid, me) = (state::owner(host)?, group_id_hex(id), own(host)?);
    let lid = lobby_id.to_owned();
    let row = state::db_call!(host, "read group lobby", move |conn| {
        group_lobby::get(conn, &owner, &gid, &lid)
    })
    .await?;
    row.map(|r| view_of(r, &me)).transpose()
}

/// Open a lobby in `group_id` for `game_id` (our shown game, the caller's
/// check), with us as its host.
///
/// # Errors
/// Not a group we hold, it has an open lobby, or the entry could not be
/// committed.
pub async fn open(host: &dyn FriendHost, group_id: &str, game_id: u32) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    held(host, &id).await?;
    if current(host, &id)
        .await?
        .is_some_and(|l| l.state == LobbyStatus::Open)
    {
        return Err(FriendError::Invalid(
            "the group already has an open lobby".into(),
        ));
    }
    let mut lobby_id: LobbyId = [0; 16];
    host.rng().fill(&mut lobby_id);
    let body = DirectBody::LobbyOpen(LobbyOpen { lobby_id, game_id });
    commit_and_fold(host, &id, body).await
}

/// Take `step` on `lobby_id` (32 hex) of `group_id`, when its verdict in
/// our view allows it.
///
/// # Errors
/// Not a group we hold, no such lobby, the verdict refuses it, or the
/// entry could not be committed.
pub async fn act(
    host: &dyn FriendHost,
    group_id: &str,
    lobby_id: &str,
    step: LobbyStep,
) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    held(host, &id).await?;
    let raw = parse_lobby_id(lobby_id)?;
    let view = lobby(host, &id, &hex::encode(raw))
        .await?
        .ok_or_else(|| FriendError::NotFound("no such lobby".into()))?;
    let verdict = match &step {
        LobbyStep::Join => view.join,
        LobbyStep::Leave => view.leave,
        LobbyStep::Close => view.close,
        LobbyStep::Launch(_) => view.launch,
    };
    if let V::Refused { reason } = verdict {
        return Err(FriendError::Invalid(format!("lobby refused: {reason:?}")));
    }
    let r = LobbyRef { lobby_id: raw };
    let body = match step {
        LobbyStep::Join => DirectBody::LobbyJoin(r),
        LobbyStep::Leave => DirectBody::LobbyLeave(r),
        LobbyStep::Close => DirectBody::LobbyClose(r),
        LobbyStep::Launch(server) => DirectBody::LobbyLaunch(LobbyLaunch {
            lobby_id: raw,
            server,
        }),
    };
    commit_and_fold(host, &id, body).await
}

/// A lobby id from its 32 hex characters.
///
/// # Errors
/// Not 16 bytes of hex.
pub fn parse_lobby_id(text: &str) -> Result<LobbyId, FriendError> {
    hex::decode(text)
        .ok()
        .and_then(|b| LobbyId::try_from(b).ok())
        .ok_or_else(|| FriendError::Invalid("a lobby id is 32 hex characters".into()))
}

async fn commit_and_fold(
    host: &dyn FriendHost,
    id: &GroupId,
    body: DirectBody,
) -> Result<(), FriendError> {
    let payload = dm_outbox::payload(&body)?;
    let client_id = client_id_text(&mint_client_id(host));
    host.outbox()?
        .enqueue_group_control(&group_id_hex(id), client_id, payload)
        .await?;
    let me = own(host)?;
    fold(host, id, &me, &body, host.clock().now_ms_i64()).await
}

/// `writer`'s opened lobby entry in `group_id`, read from its chain:
/// folded only when `rekindle_dm::admit` admits `writer` on the group's
/// roster.
pub(super) async fn on_lobby(
    host: &dyn FriendHost,
    group_id: &GroupId,
    writer: &str,
    body: &DirectBody,
    sent_at_ms: u64,
) -> Result<(), FriendError> {
    let group = held(host, group_id).await?;
    let frame = GroupFrame::Lobby {
        kind: body.type_id(),
        group_id,
    };
    let groups = [AcceptedGroup {
        group_id,
        roster: &group.roster,
    }];
    let admission = rekindle_dm::admit(writer, false, &frame, &groups);
    if admission != Admission::Admit {
        tracing::info!(from = %writer, ?admission, "lobby entry not admitted; dropped");
        return Ok(());
    }
    let at = i64::try_from(sent_at_ms).unwrap_or(i64::MAX);
    fold(host, group_id, writer, body, at).await
}

/// Fold `writer`'s `body` into `group_lobbies`; a change is published as
/// `GroupDmEvent::Lobby` with the touched lobby's view.
async fn fold(
    host: &dyn FriendHost,
    id: &GroupId,
    writer: &str,
    body: &DirectBody,
    at_ms: i64,
) -> Result<(), FriendError> {
    let (owner, gid, wkey) = (state::owner(host)?, group_id_hex(id), writer.to_owned());
    let (lobby_id, body) = match body {
        DirectBody::LobbyOpen(b) => (b.lobby_id, body.clone()),
        DirectBody::LobbyJoin(r) | DirectBody::LobbyLeave(r) | DirectBody::LobbyClose(r) => {
            (r.lobby_id, body.clone())
        }
        DirectBody::LobbyLaunch(b) => (b.lobby_id, body.clone()),
        other => {
            tracing::debug!(kind = ?other.type_id(), "not a lobby kind; not folded");
            return Ok(());
        }
    };
    let lid = hex::encode(lobby_id);
    let lkey = lid.clone();
    let changed = state::db_call!(host, "fold group lobby", move |conn| {
        let (own, grp, lob, wr) = (owner.as_str(), gid.as_str(), lkey.as_str(), wkey.as_str());
        match &body {
            DirectBody::LobbyOpen(b) => {
                group_lobby::open(conn, own, grp, lob, wr, b.game_id, at_ms)
            }
            DirectBody::LobbyJoin(_) => group_lobby::join(conn, own, grp, lob, wr),
            DirectBody::LobbyLeave(_) => group_lobby::leave(conn, own, grp, lob, wr),
            DirectBody::LobbyLaunch(b) => group_lobby::launch(conn, own, grp, lob, wr, &b.server),
            _ => group_lobby::close(conn, own, grp, lob, wr),
        }
    })
    .await?;
    if changed {
        let view = lobby(host, id, &lid).await?;
        host.publish(SubscriptionEvent::GroupDm(GroupDmEvent::Lobby {
            group_id: group_id_hex(id),
            lobby: view,
        }));
    }
    Ok(())
}

#[cfg(test)]
#[path = "lobby_tests.rs"]
mod tests;
