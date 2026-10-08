//! Role-id sets on member rows, and the read-modify-write role changes.

use rusqlite::{params, Connection, OptionalExtension as _};

use crate::repo::{communities, role_ids as codec};

/// Set the member's role ids; `onboarded` also marks onboarding complete.
///
/// # Errors
/// The write failed.
pub fn set_role_ids(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    role_ids: &[u32],
    onboarded: bool,
) -> rusqlite::Result<()> {
    let set = if onboarded {
        "role_ids = ?1, onboarding_complete = 1"
    } else {
        "role_ids = ?1"
    };
    conn.execute(
        &format!(
            "UPDATE community_members SET {set} \
             WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4"
        ),
        params![
            codec::encode(role_ids)?,
            owner_key,
            community_id,
            pseudonym_key
        ],
    )?;
    Ok(())
}

/// The member's role ids; `None` when the member is not stored.
///
/// # Errors
/// The query failed or the stored set does not decode.
pub fn role_ids(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
) -> rusqlite::Result<Option<Vec<u32>>> {
    conn.query_row(
        "SELECT role_ids FROM community_members \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
        params![owner_key, community_id, pseudonym_key],
        |row| codec::decode(0, &row.get::<_, String>(0)?),
    )
    .optional()
}

/// Give the member a role. A member not stored yet is left alone: the
/// registry scan writes its roles when it is found.
///
/// # Errors
/// The query or write failed, or the stored set does not decode.
pub fn add_role(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> rusqlite::Result<()> {
    let Some(mut ids) = role_ids(conn, owner_key, community_id, pseudonym_key)? else {
        return Ok(());
    };
    if ids.contains(&role_id) {
        return Ok(());
    }
    ids.push(role_id);
    set_role_ids(conn, owner_key, community_id, pseudonym_key, &ids, false)
}

/// Take a role from the member.
///
/// # Errors
/// The query or write failed, or the stored set does not decode.
pub fn remove_role(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym_key: &str,
    role_id: u32,
) -> rusqlite::Result<()> {
    let Some(mut ids) = role_ids(conn, owner_key, community_id, pseudonym_key)? else {
        return Ok(());
    };
    if !ids.contains(&role_id) {
        return Ok(());
    }
    ids.retain(|&id| id != role_id);
    set_role_ids(conn, owner_key, community_id, pseudonym_key, &ids, false)
}

/// A deleted role: take it from every member, and from our own set.
///
/// # Errors
/// A query or write failed, or a stored set does not decode.
pub fn remove_role_everywhere(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    role_id: u32,
) -> rusqlite::Result<()> {
    let holders: Vec<(String, Vec<u32>)> = {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, role_ids FROM community_members \
             WHERE owner_key = ?1 AND community_id = ?2",
        )?;
        let rows = stmt
            .query_map(params![owner_key, community_id], |row| {
                Ok((row.get(0)?, codec::decode(1, &row.get::<_, String>(1)?)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (pseudonym_key, mut ids) in holders {
        if ids.contains(&role_id) {
            ids.retain(|&id| id != role_id);
            set_role_ids(conn, owner_key, community_id, &pseudonym_key, &ids, false)?;
        }
    }
    if let Some(mut mine) = communities::my_role_ids(conn, owner_key, community_id)? {
        if mine.contains(&role_id) {
            mine.retain(|&id| id != role_id);
            communities::set_my_role_ids(conn, owner_key, community_id, &mine)?;
        }
    }
    Ok(())
}
