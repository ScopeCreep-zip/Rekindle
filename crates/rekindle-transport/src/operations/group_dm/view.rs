//! The groups and invites as the frontends show them, and a group's
//! stored messages (plan E2.6). A member is named by the Host
//! (`GroupDmDeps::display_name`, GC1): "You", a friend's name, or the name
//! the creator signed onto the roster marked as not a friend's; never a key.

use rekindle_dm::GroupDmDeps;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::{GroupDmView, GroupDmViewMember};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::{friend_error, parse_group_id};
use crate::operations::friend::{state, FriendError, FriendHost};

/// Why an unanswered invite's room actions wait.
const NOT_JOINED: &str = "accept the invite first";

async fn members(deps: &dyn GroupDmDeps, roster: &[GroupDmRosterMember]) -> Vec<GroupDmViewMember> {
    let mut out = Vec::with_capacity(roster.len());
    for member in roster {
        out.push(GroupDmViewMember {
            key: member.identity.clone(),
            label: deps.display_name(&member.identity).await,
        });
    }
    out
}

/// Every invite we have not answered, newest first, then every group we
/// are in.
///
/// # Errors
/// Not unlocked, or the groups or invites could not be read.
pub async fn list(host: &dyn FriendHost) -> Result<Vec<GroupDmView>, FriendError> {
    let deps = host.group_dm()?;
    let deps = deps.as_ref();
    let invites = deps.invites().await.map_err(|e| friend_error(&e))?;
    let groups = deps.groups().await.map_err(|e| friend_error(&e))?;
    let mut views = Vec::with_capacity(invites.len() + groups.len());
    for pending in invites {
        let invite = pending.invite;
        views.push(GroupDmView {
            group_id: rekindle_dm::group::group_id_hex(&invite.group_id),
            name: invite.name,
            members: members(deps, &invite.roster).await,
            epoch: invite.epoch,
            pending: true,
            origin_peer: None,
            favourite: false,
            rename_refused: Some(NOT_JOINED.to_owned()),
            invite_refused: Some(NOT_JOINED.to_owned()),
        });
    }
    let local = local_flags(host).await?;
    let me = deps.own_identity();
    for group in groups {
        let group_id = rekindle_dm::group::group_id_hex(&group.group_id);
        let (origin_peer, favourite) = local.get(&group_id).cloned().unwrap_or_default();
        views.push(GroupDmView {
            group_id,
            name: group.name,
            members: members(deps, group.roster.members()).await,
            epoch: group.epoch,
            pending: false,
            origin_peer,
            favourite,
            rename_refused: rekindle_dm::rename_refused(&group.roster, &me),
            invite_refused: rekindle_dm::invite_refused(&group.roster, &me),
        });
    }
    Ok(views)
}

/// Each held group's local columns (G7): the chat it came from and
/// whether it is a favourite, by group id.
async fn local_flags(
    host: &dyn FriendHost,
) -> Result<std::collections::HashMap<String, (Option<String>, bool)>, FriendError> {
    let owner = state::owner(host)?;
    state::db_call!(host, "read the groups' local columns", move |conn| {
        rekindle_db::repo::group_dms::list(conn, &owner).map(|rows| {
            rows.into_iter()
                .map(|r| (r.group_id, (r.origin_peer, r.favourite)))
                .collect()
        })
    })
    .await
}

/// The group or pending invite `group_id`, as [`list`] shows it.
///
/// # Errors
/// Not unlocked, a malformed id, or the groups could not be read.
pub async fn find(
    host: &dyn FriendHost,
    group_id: &str,
) -> Result<Option<GroupDmView>, FriendError> {
    parse_group_id(group_id)?;
    Ok(list(host)
        .await?
        .into_iter()
        .find(|v| v.group_id == group_id))
}

/// The group's window displayed its messages (plan E7.1): received ones
/// whose disappearing timer had not started start it now, as a DM's do at
/// `mark_read`; a history load alone starts nothing.
///
/// # Errors
/// A malformed id.
pub async fn mark_read(host: &dyn FriendHost, group_id: &str) -> Result<(), FriendError> {
    parse_group_id(group_id)?;
    crate::operations::dm::disappearing::displayed(host, group_id).await;
    Ok(())
}

/// The newest `limit` messages of `group_id` before `before` (ms), oldest
/// first.
///
/// # Errors
/// A malformed id, or the history could not be read.
pub async fn messages(
    host: &dyn FriendHost,
    group_id: &str,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<DmMessage>, FriendError> {
    parse_group_id(group_id)?;
    let owner = state::owner(host)?;
    let (ok, gid) = (owner.clone(), group_id.to_string());
    let rows = state::db_call!(host, "read group messages", move |conn| {
        rekindle_db::repo::group_dm_messages::group_dm_history(conn, &ok, &gid, limit, before)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| dm_message(row, &owner))
        .collect())
}

/// A stored group row as the group's history answers it (`owner`: ours).
pub(super) fn dm_message(row: rekindle_db::repo::messages::DmRow, owner: &str) -> DmMessage {
    DmMessage {
        id: row.id,
        message_id: row.message_id,
        is_own: row.sender_key == owner,
        sender_id: row.sender_key,
        body: row.body,
        timestamp: row.timestamp,
        send_state: row.send_state,
        history_gap_through: row.history_gap_through,
        expire_seconds: row.expire_seconds,
        expire_at: row.expire_at,
        timer_notice_seconds: row.timer_notice_seconds,
    }
}
