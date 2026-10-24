//! Our local labels for friends: a nickname per friend, and friend groups.
//! Local only — a nickname is never a peer's claim (it is not spoofable),
//! and our friend-list record carries both for our other devices.

use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// Set (or clear, with `None`) our nickname for one friend.
///
/// # Errors
/// `NotFound` when they are no friend; the nickname could not be stored.
pub async fn set_nickname(
    host: &dyn FriendHost,
    public_key: &str,
    nickname: Option<&str>,
) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    if state::friend(host, public_key).is_none() {
        return Err(FriendError::NotFound(format!(
            "{public_key} is not a friend"
        )));
    }
    let (pk, nick) = (public_key.to_string(), nickname.map(str::to_string));
    state::db_call!(host, "store nickname", move |conn| {
        rekindle_db::repo::friends::set(
            conn,
            &owner,
            &pk,
            rekindle_db::repo::friends::Column::Nickname,
            nick,
        )
    })
    .await?;
    state::update(host, public_key, |f| {
        f.nickname = nickname.map(str::to_string);
    });

    // The record half: our friend list, built from local state alone like
    // every other friend-list write (`list_record`), never a network read
    // written back, which could replace a held push with a stale list and
    // drop a new friend from the record (review RV3 problem 6).
    super::list_record::push_soon(host, "a nickname edit");
    host.publish(SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
        peer_key: public_key.to_string(),
        nickname: nickname.map(str::to_string),
    }));
    Ok(())
}

/// Every friend whose group label is `from`, relabelled `to`, each
/// announced.
fn relabel(host: &dyn FriendHost, from: &str, to: Option<&str>) {
    for friend in state::friends(host) {
        if friend.group.as_deref() == Some(from) {
            state::update(host, &friend.public_key, |f| {
                f.group = to.map(str::to_string);
            });
            host.publish(SubscriptionEvent::Friend(FriendEvent::GroupChanged {
                peer_key: friend.public_key,
                group: to.map(str::to_string),
                group_label: rekindle_types::dto::friends::group_label(to),
            }));
        }
    }
}

/// The name of our group `group_id`.
async fn group_name(host: &dyn FriendHost, group_id: i64) -> Result<String, FriendError> {
    let owner = state::owner(host)?;
    state::db_call!(host, "read friend group", move |conn| {
        rekindle_db::repo::friend_groups::name(conn, &owner, group_id)
    })
    .await?
    .ok_or_else(|| FriendError::NotFound(format!("no friend group {group_id}")))
}

/// Create a friend group; its id.
///
/// # Errors
/// The group could not be stored.
pub async fn create_group(host: &dyn FriendHost, name: &str) -> Result<i64, FriendError> {
    let owner = state::owner(host)?;
    let name = name.to_string();
    state::db_call!(host, "create friend group", move |conn| {
        rekindle_db::repo::friend_groups::create(conn, &owner, &name)
    })
    .await
}

/// Rename our friend group `group_id`; its friends are relabelled.
///
/// # Errors
/// `NotFound` when we have no such group.
pub async fn rename_group(
    host: &dyn FriendHost,
    group_id: i64,
    name: &str,
) -> Result<(), FriendError> {
    let old = group_name(host, group_id).await?;
    let owner = state::owner(host)?;
    let new = name.to_string();
    state::db_call!(host, "rename friend group", move |conn| {
        rekindle_db::repo::friend_groups::rename(conn, &owner, group_id, &new)
    })
    .await?;
    relabel(host, &old, Some(name));
    Ok(())
}

/// Delete our friend group `group_id`; its friends fall back to no group
/// (`friends.group_id ... ON DELETE SET NULL`).
///
/// # Errors
/// `NotFound` when we have no such group.
pub async fn delete_group(host: &dyn FriendHost, group_id: i64) -> Result<(), FriendError> {
    let old = group_name(host, group_id).await?;
    let owner = state::owner(host)?;
    state::db_call!(host, "delete friend group", move |conn| {
        rekindle_db::repo::friend_groups::delete(conn, &owner, group_id)
    })
    .await?;
    relabel(host, &old, None);
    Ok(())
}

/// Move a friend into group `group_id`, or out of any group with `None`.
///
/// # Errors
/// `NotFound` when they are no friend or we have no such group.
pub async fn move_to_group(
    host: &dyn FriendHost,
    public_key: &str,
    group_id: Option<i64>,
) -> Result<(), FriendError> {
    if state::friend(host, public_key).is_none() {
        return Err(FriendError::NotFound(format!(
            "{public_key} is not a friend"
        )));
    }
    let group = match group_id {
        Some(id) => Some(group_name(host, id).await?),
        None => None,
    };
    let owner = state::owner(host)?;
    let pk = public_key.to_string();
    state::db_call!(host, "move friend to group", move |conn| {
        rekindle_db::repo::friends::set(
            conn,
            &owner,
            &pk,
            rekindle_db::repo::friends::Column::GroupId,
            group_id,
        )
    })
    .await?;
    state::update(host, public_key, |f| f.group.clone_from(&group));
    host.publish(SubscriptionEvent::Friend(FriendEvent::GroupChanged {
        peer_key: public_key.to_string(),
        group_label: rekindle_types::dto::friends::group_label(group.as_deref()),
        group,
    }));
    Ok(())
}
