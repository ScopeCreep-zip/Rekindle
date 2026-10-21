//! The groups and invites as the frontends show them, and a group's
//! stored messages (plan E2.6). A member is named by our nickname for them
//! or their fingerprint (`GroupDmDeps::display_name`), never by a name
//! their payload carries.

use rekindle_dm::GroupDmDeps;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::{GroupDmView, GroupDmViewMember};
use rekindle_types::group_dm::GroupDmRosterMember;

use super::{friend_error, parse_group_id};
use crate::operations::friend::{state, FriendError, FriendHost};

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
        });
    }
    for group in groups {
        views.push(GroupDmView {
            group_id: rekindle_dm::group::group_id_hex(&group.group_id),
            name: group.name,
            members: members(deps, group.roster.members()).await,
            epoch: group.epoch,
            pending: false,
        });
    }
    Ok(views)
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
        .map(|row| DmMessage {
            id: row.id,
            message_id: row.message_id,
            is_own: row.sender_key == owner,
            sender_id: row.sender_key,
            body: row.body,
            timestamp: row.timestamp,
            send_state: row.send_state,
            history_gap_through: row.history_gap_through,
        })
        .collect())
}
