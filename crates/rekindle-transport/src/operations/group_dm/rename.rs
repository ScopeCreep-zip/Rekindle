//! Renaming a group (plan E7.2 G7, SK2-G §6): only its creator, whose
//! signature made the roster (E2.6), names it. The rename is a
//! `GroupDmRename` sealed inside a `GroupContent` entry's inner and
//! committed to our group stream with no echo, so it rides our chain like
//! a message and every member reads it from there (or from its hint). A
//! member applies it only when the entry's writer is the group's creator.

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupDmRename, GroupId};
use rekindle_dm::group::group_id_hex;
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use super::content::held;
use super::{friend_error, parse_group_id};
use crate::operations::dm::outbox as dm_outbox;
use crate::operations::dm::{client_id_text, mint_client_id};
use crate::operations::friend::{state, FriendError, FriendHost};

/// Rename `group_id` (32 hex) to `name`: ours is set at once and the
/// rename is committed to our group stream for every member.
///
/// # Errors
/// Not a group we hold, we are not its creator, the name is out of
/// bounds, or the entry could not be committed.
pub async fn rename(host: &dyn FriendHost, group_id: &str, name: &str) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    rekindle_dm::group::check_name(name).map_err(|e| friend_error(&e))?;
    let group = held(host, &id).await?;
    if group.roster.creator().identity != host.group_dm()?.own_identity() {
        return Err(FriendError::Invalid(
            "only the group's creator renames it".into(),
        ));
    }
    let payload = dm_outbox::payload(&DirectBody::GroupDmRename(GroupDmRename {
        group_id: id,
        name: name.to_owned(),
    }))?;
    let client_id = client_id_text(&mint_client_id(host));
    host.outbox()?
        .enqueue_group_control(group_id, client_id, payload)
        .await?;
    set_name(host, &id, name).await
}

/// `writer`'s opened `GroupDmRename` in `group_id`: applied only when
/// `writer` is the group's creator and it names this group.
pub(super) async fn on_rename(
    host: &dyn FriendHost,
    group_id: &GroupId,
    writer: &str,
    rename: &GroupDmRename,
) -> Result<(), FriendError> {
    if rename.group_id != *group_id {
        tracing::info!(from = %writer, "group rename names another group; refused");
        return Ok(());
    }
    let group = held(host, group_id).await?;
    if group.roster.creator().identity != writer {
        tracing::info!(from = %writer, "group rename by a non-creator; refused");
        return Ok(());
    }
    set_name(host, group_id, &rename.name).await
}

/// Store `name` as the group's; a change is published as `Renamed`.
async fn set_name(
    host: &dyn FriendHost,
    group_id: &GroupId,
    name: &str,
) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let (gid, new) = (group_id_hex(group_id), name.to_owned());
    let (id, stored) = (gid.clone(), new.clone());
    let changed = state::db_call!(host, "rename group", move |conn| {
        rekindle_db::repo::group_dms::set_name(conn, &owner, &id, &stored)
    })
    .await?;
    if changed {
        host.publish(SubscriptionEvent::GroupDm(GroupDmEvent::Renamed {
            group_id: gid,
            name: new,
        }));
    }
    Ok(())
}
