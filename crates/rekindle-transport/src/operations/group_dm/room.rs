//! A room is a group DM (SK2-G §6, plan E7.2 G7). A group created from a
//! friend's chat records that friend (`group_dms.origin_peer`), so the
//! chat's `EnterAttachedRoom` opens it: Xfire's "This chat is now a chat
//! room session", done as a new group, since a 1:1 ratchet session cannot
//! become one.

use super::create;
use crate::operations::friend::{state, FriendError, FriendHost};

/// [`create`], then record `origin_peer` (a member, checked by the
/// request's validation) as the chat the group came from.
///
/// # Errors
/// As [`create`], or the origin could not be stored.
pub async fn create_from(
    host: &dyn FriendHost,
    name: &str,
    members: &[String],
    origin_peer: Option<&str>,
) -> Result<String, FriendError> {
    let group_id = create(host, name, members).await?;
    if let Some(peer) = origin_peer {
        let owner = state::owner(host)?;
        let (gid, peer) = (group_id.clone(), peer.to_owned());
        state::db_call!(host, "store the group's origin chat", move |conn| {
            rekindle_db::repo::group_dms::set_origin_peer(conn, &owner, &gid, &peer)
        })
        .await?;
    }
    Ok(group_id)
}
