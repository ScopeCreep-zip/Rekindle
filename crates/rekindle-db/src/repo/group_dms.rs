//! `group_dms` and `group_dm_sender_keys` (plan E2.6): the groups we are
//! in, their signed rosters and our sending epoch, and each member's group
//! sending key by epoch. The owner of their behaviour is `rekindle-dm`.

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
}

fn roster_json(roster: &[GroupDmRosterMember]) -> rusqlite::Result<String> {
    serde_json::to_string(roster).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupDmRow> {
    Ok(GroupDmRow {
        group_id: crate::row::get(row, "group_id")?,
        name: crate::row::get(row, "name")?,
        roster: crate::row::get_json(row, "roster")?,
        epoch: crate::row::get(row, "epoch")?,
        created_at: crate::row::get(row, "created_at")?,
    })
}

const COLS: &str = "group_id, name, roster, epoch, created_at";

/// Store a group we joined or created.
///
/// # Errors
/// The write failed (the group is already held, or a column is refused).
pub fn insert(conn: &Connection, owner_key: &str, group: &GroupDmRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO group_dms (owner_key, group_id, name, roster, epoch, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            owner_key,
            group.group_id,
            group.name,
            roster_json(&group.roster)?,
            group.epoch,
            group.created_at
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
    conn.query_row(
        &format!("SELECT {COLS} FROM group_dms WHERE owner_key = ?1 AND group_id = ?2"),
        params![owner_key, group_id],
        read_row,
    )
    .optional()
}

/// Every group we are in, oldest first.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn list(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<GroupDmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM group_dms WHERE owner_key = ?1 ORDER BY created_at, group_id"
    ))?;
    let rows = stmt.query_map(params![owner_key], read_row)?;
    rows.collect()
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
    conn.execute(
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
    conn.execute(
        "DELETE FROM group_dm_chains WHERE owner_key = ?1 AND group_id = ?2 AND member = ?3",
        params![owner_key, group_id, member],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    const G: &str = "0123456789abcdef0123456789abcdef";

    fn member(n: char) -> GroupDmRosterMember {
        GroupDmRosterMember {
            identity: n.to_string().repeat(64),
            profile_record_key: format!("VLD0:{}", "P".repeat(43)),
        }
    }

    fn group() -> GroupDmRow {
        GroupDmRow {
            group_id: G.to_string(),
            name: "raid night".to_string(),
            roster: vec![member('a'), member('b'), member('c')],
            epoch: 0,
            created_at: 5,
        }
    }

    #[test]
    fn a_group_is_stored_listed_updated_and_deleted() {
        let conn = fixture::conn();
        insert(&conn, OWNER, &group()).unwrap();
        assert!(insert(&conn, OWNER, &group()).is_err(), "held once");
        assert_eq!(get(&conn, OWNER, G).unwrap(), Some(group()));
        assert_eq!(list(&conn, OWNER).unwrap(), vec![group()]);
        let roster = vec![member('a'), member('b')];
        assert!(update_roster_epoch(&conn, OWNER, G, &roster, 1).unwrap());
        let held = get(&conn, OWNER, G).unwrap().unwrap();
        assert_eq!((held.roster, held.epoch), (roster, 1));
        assert!(!update_roster_epoch(&conn, OWNER, "ff", &[], 1).unwrap());
        assert!(delete(&conn, OWNER, G).unwrap());
        assert_eq!(get(&conn, OWNER, G).unwrap(), None);
        assert!(!delete(&conn, OWNER, G).unwrap());
    }

    #[test]
    fn a_corrupt_roster_or_id_is_refused() {
        let conn = fixture::conn();
        let mut bad = group();
        bad.group_id = "short".to_string();
        assert!(insert(&conn, OWNER, &bad).is_err());
        insert(&conn, OWNER, &group()).unwrap();
        conn.execute("UPDATE group_dms SET roster = '[1]'", [])
            .unwrap();
        assert!(get(&conn, OWNER, G).is_err(), "a roster entry is a member");
    }

    #[test]
    fn chains_are_kept_by_member_and_go_with_the_group() {
        let conn = fixture::conn();
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        assert!(
            put_chain(&conn, OWNER, G, &a, "VLD0:x").is_err(),
            "no such group"
        );
        insert(&conn, OWNER, &group()).unwrap();
        put_chain(&conn, OWNER, G, &a, "VLD0:x").unwrap();
        put_chain(&conn, OWNER, G, &a, "VLD0:y").unwrap();
        put_chain(&conn, OWNER, G, &b, "VLD0:z").unwrap();
        assert_eq!(
            chains(&conn, OWNER, G).unwrap(),
            vec![(a.clone(), "VLD0:y".into()), (b.clone(), "VLD0:z".into())]
        );
        assert_eq!(delete_chain(&conn, OWNER, G, &a).unwrap(), 1);
        delete(&conn, OWNER, G).unwrap();
        assert!(chains(&conn, OWNER, G).unwrap().is_empty(), "cascade");
    }

    #[test]
    fn sender_keys_are_kept_by_member_and_epoch() {
        let conn = fixture::conn();
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        assert!(
            put_sender_key(&conn, OWNER, G, &a, 0, &[1; 32]).is_err(),
            "no such group"
        );
        insert(&conn, OWNER, &group()).unwrap();
        put_sender_key(&conn, OWNER, G, &a, 0, &[1; 32]).unwrap();
        put_sender_key(&conn, OWNER, G, &a, 1, &[2; 32]).unwrap();
        put_sender_key(&conn, OWNER, G, &b, 0, &[3; 32]).unwrap();
        assert_eq!(sender_key(&conn, OWNER, G, &a, 0).unwrap(), Some([1; 32]));
        assert_eq!(sender_key(&conn, OWNER, G, &a, 2).unwrap(), None);
        assert_eq!(
            latest_sender_key(&conn, OWNER, G, &a).unwrap(),
            Some((1, [2; 32]))
        );
        assert_eq!(delete_member_keys(&conn, OWNER, G, &a).unwrap(), 2);
        assert_eq!(latest_sender_key(&conn, OWNER, G, &a).unwrap(), None);
        assert!(conn
            .execute(
                "INSERT INTO group_dm_sender_keys VALUES (?1, ?2, ?3, 5, x'00')",
                params![OWNER, G, b],
            )
            .is_err());
        delete(&conn, OWNER, G).unwrap();
        assert_eq!(
            latest_sender_key(&conn, OWNER, G, &b).unwrap(),
            None,
            "cascade"
        );
    }
}
