//! Each friend's current friend-presence key (plan E2.5): the
//! `presence_epoch` and `presence_key` columns of `friends`.

use rusqlite::{params, Connection};

/// Store `public_key`'s key of `epoch`, unless the row holds a newer one
/// (a late `FriendsKey` never rolls a friend back). Whether it was stored.
///
/// # Errors
/// The write failed.
pub fn set_presence_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    epoch: u32,
    key: &[u8],
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE friends SET presence_epoch = ?3, presence_key = ?4 \
         WHERE owner_key = ?1 AND public_key = ?2 \
         AND (presence_epoch IS NULL OR presence_epoch <= ?3)",
        params![owner_key, public_key, epoch, key],
    )?;
    Ok(changed > 0)
}

/// Forget `public_key`'s key (we removed them).
///
/// # Errors
/// The write failed.
pub fn clear_presence_key(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friends SET presence_epoch = NULL, presence_key = NULL \
         WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, public_key],
    )?;
    Ok(())
}

/// `(public_key, epoch, key)` of every friend whose key we hold.
///
/// # Errors
/// The query failed.
pub fn presence_keys(
    conn: &Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<(String, u32, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT public_key, presence_epoch, presence_key FROM friends \
         WHERE owner_key = ?1 AND presence_key IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect();
    rows
}
