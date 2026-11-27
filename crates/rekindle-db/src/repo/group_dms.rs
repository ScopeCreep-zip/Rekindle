//! `group_dms` and `group_dm_sender_keys` (plan E2.6): the groups we are
//! in, their signed rosters and our sending epoch, and each member's group
//! sending key by epoch. The owner of their behaviour is `rekindle-dm`.

use crate::row::{query_list, query_opt, FromRow};
use rekindle_types::group_dm::GroupDmRosterMember;
use rusqlite::{params, Connection, OptionalExtension as _};

/// A group DM we are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDmRow {
    /// The creator's 16 random bytes, in hex.
    pub group_id: String,
    pub name: String,
    pub roster: Vec<GroupDmRosterMember>,
    /// Our own sending key's current epoch.
    pub epoch: u32,
    pub created_at: i64,
    /// The friend whose chat the group was created from (SK2-G §6).
    pub origin_peer: Option<String>,
    /// In our "Chat Rooms" favourites (local only).
    pub favourite: bool,
}

impl FromRow for GroupDmRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(GroupDmRow {
            group_id: crate::row::get(row, "group_id")?,
            name: crate::row::get(row, "name")?,
            roster: crate::row::get_json(row, "roster")?,
            epoch: crate::row::get(row, "epoch")?,
            created_at: crate::row::get(row, "created_at")?,
            origin_peer: crate::row::get(row, "origin_peer")?,
            favourite: crate::row::get(row, "favourite")?,
        })
    }
}

fn roster_json(roster: &[GroupDmRosterMember]) -> rusqlite::Result<String> {
    serde_json::to_string(roster).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

const COLS: &str = "group_id, name, roster, epoch, created_at, origin_peer, favourite";

/// Store a group we joined or created.
///
/// # Errors
/// The write failed (the group is already held, or a column is refused).
pub fn insert(conn: &Connection, owner_key: &str, group: &GroupDmRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO group_dms \
         (owner_key, group_id, name, roster, epoch, created_at, origin_peer, favourite) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            owner_key,
            group.group_id,
            group.name,
            roster_json(&group.roster)?,
            group.epoch,
            group.created_at,
            group.origin_peer,
            group.favourite
        ],
    )?;
    Ok(())
}

/// One group, if we are in it.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
) -> rusqlite::Result<Option<GroupDmRow>> {
    query_opt(
        conn,
        &format!("SELECT {COLS} FROM group_dms WHERE owner_key = ?1 AND group_id = ?2"),
        params![owner_key, group_id],
    )
}

/// What this identity knows of a group's call roster (E4.1): the members'
/// keys when we hold the group, `Invited` while our invite to it is still
/// pending, else `Unknown`. The one source for `group_call_refused`.
///
/// # Errors
/// A query failed or a row is corrupt.
pub fn call_roster(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    now_ms: i64,
) -> rusqlite::Result<rekindle_types::group_dm::GroupCallRoster> {
    use rekindle_types::group_dm::GroupCallRoster;
    if let Some(group) = get(conn, owner_key, group_id)? {
        let keys = group.roster.into_iter().map(|m| m.identity).collect();
        return Ok(GroupCallRoster::Held(keys));
    }
    let invited = super::group_dm_invites::list(conn, owner_key, now_ms)?
        .iter()
        .any(|invite| invite.group_id == group_id);
    Ok(if invited {
        GroupCallRoster::Invited
    } else {
        GroupCallRoster::Unknown
    })
}

/// Every group we are in, oldest first.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<GroupDmRow>> {
    query_list(
        conn,
        &format!("SELECT {COLS} FROM group_dms WHERE owner_key = ?1 ORDER BY created_at, group_id"),
        params![owner_key],
    )
}

/// Record the friend whose chat `group_id` was created from (SK2-G §6).
/// `false` when we are not in the group.
///
/// # Errors
/// The write failed (the key is refused).
pub fn set_origin_peer(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    peer: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE group_dms SET origin_peer = ?3 WHERE owner_key = ?1 AND group_id = ?2",
        params![owner_key, group_id, peer],
    )?;
    Ok(changed > 0)
}

/// The group's name (its creator renamed it). `false` when we are not in
/// the group or it already had that name.
///
/// # Errors
/// The write failed.
pub fn set_name(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    name: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE group_dms SET name = ?3 WHERE owner_key = ?1 AND group_id = ?2 AND name <> ?3",
        params![owner_key, group_id, name],
    )?;
    Ok(changed > 0)
}

/// The group most recently created from `peer`'s chat, the room attached
/// to it (`EnterAttachedRoom`).
///
/// # Errors
/// The query failed.
pub fn attached(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT group_id FROM group_dms WHERE owner_key = ?1 AND origin_peer = ?2 \
         ORDER BY created_at DESC, group_id DESC LIMIT 1",
        params![owner_key, peer],
        |row| row.get(0),
    )
    .optional()
}

/// Replace a group's roster and our epoch (a leave or a block rotates our
/// key). `false` when we are not in the group.
///
/// # Errors
/// The write failed.
pub fn update_roster_epoch(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    roster: &[GroupDmRosterMember],
    epoch: u32,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE group_dms SET roster = ?3, epoch = ?4 WHERE owner_key = ?1 AND group_id = ?2",
        params![owner_key, group_id, roster_json(roster)?, epoch],
    )?;
    Ok(changed > 0)
}

/// Forget a group (we left it); its sender keys go with it. `false` when we
/// were not in it.
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, owner_key: &str, group_id: &str) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "DELETE FROM group_dms WHERE owner_key = ?1 AND group_id = ?2",
        params![owner_key, group_id],
    )?;
    Ok(changed > 0)
}

fn key32(key: &[u8], index: usize) -> rusqlite::Result<[u8; 32]> {
    key.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Blob,
            format!("sender key of {} bytes", key.len()).into(),
        )
    })
}

/// Store `member`'s sending key of `epoch` (replacing a resend of it).
///
/// # Errors
/// The write failed (no such group, or a column is refused).
pub fn put_sender_key(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
    epoch: u32,
    key: &[u8; 32],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO group_dm_sender_keys (owner_key, group_id, member, epoch, key) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(owner_key, group_id, member, epoch) DO UPDATE SET key = excluded.key",
        params![owner_key, group_id, member, epoch, key.as_slice()],
    )?;
    Ok(())
}

/// `member`'s sending key of `epoch`, if held.
///
/// # Errors
/// The query failed or the stored key is not 32 bytes.
pub fn sender_key(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
    epoch: u32,
) -> rusqlite::Result<Option<[u8; 32]>> {
    conn.query_row(
        "SELECT key FROM group_dm_sender_keys \
         WHERE owner_key = ?1 AND group_id = ?2 AND member = ?3 AND epoch = ?4",
        params![owner_key, group_id, member, epoch],
        |row| key32(&row.get::<_, Vec<u8>>(0)?, 0),
    )
    .optional()
}

/// `member`'s newest sending key: `(epoch, key)`.
///
/// # Errors
/// The query failed or the stored key is not 32 bytes.
pub fn latest_sender_key(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
) -> rusqlite::Result<Option<(u32, [u8; 32])>> {
    conn.query_row(
        "SELECT epoch, key FROM group_dm_sender_keys \
         WHERE owner_key = ?1 AND group_id = ?2 AND member = ?3 ORDER BY epoch DESC LIMIT 1",
        params![owner_key, group_id, member],
        |row| Ok((row.get(0)?, key32(&row.get::<_, Vec<u8>>(1)?, 1)?)),
    )
    .optional()
}

/// Drop every key of `member` in a group (it left, or we blocked it): the
/// number of keys dropped.
///
/// # Errors
/// The write failed.
pub fn delete_member_keys(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
) -> rusqlite::Result<usize> {
    crate::erase::delete(
        conn,
        "DELETE FROM group_dm_sender_keys WHERE owner_key = ?1 AND group_id = ?2 AND member = ?3",
        params![owner_key, group_id, member],
    )
}

/// Store `record_key` as `member`'s group chain (replacing the held one).
///
/// # Errors
/// The write failed (no such group, or a column is refused).
pub fn put_chain(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
    record_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO group_dm_chains (owner_key, group_id, member, record_key) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(owner_key, group_id, member) DO UPDATE SET record_key = excluded.record_key",
        params![owner_key, group_id, member, record_key],
    )?;
    Ok(())
}

/// Every member's group chain in the group, `(member, record_key)`.
///
/// # Errors
/// The query failed.
pub fn chains(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT member, record_key FROM group_dm_chains \
         WHERE owner_key = ?1 AND group_id = ?2 ORDER BY member",
    )?;
    let rows = stmt.query_map(params![owner_key, group_id], |r| {
        Ok((
            crate::row::get(r, "member")?,
            crate::row::get(r, "record_key")?,
        ))
    })?;
    rows.collect()
}

/// Forget `member`'s group chain (it left, or we blocked it).
///
/// # Errors
/// The write failed.
pub fn delete_chain(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
    member: &str,
) -> rusqlite::Result<usize> {
    crate::erase::delete(
        conn,
        "DELETE FROM group_dm_chains WHERE owner_key = ?1 AND group_id = ?2 AND member = ?3",
        params![owner_key, group_id, member],
    )
}

#[cfg(test)]
#[path = "group_dms_tests.rs"]
mod tests;
