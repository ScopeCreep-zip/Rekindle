//! `friend_nicknames`: our nickname for a contact, bound to the key it was
//! given for (live finding 79, research batch 13). A nickname is a label the
//! user chose, shown beside the key fingerprint, never a confirm token.
//!
//! - It reads as given while the contact's key is the one it was given for;
//!   after an identity rotation (`peer_rekey` moves `contact_key`) it reads
//!   "<name> (name for previous key)" until the user sets it again.
//! - Ending the friendship deletes it ([`clear`]: our removal, theirs, a
//!   cancelled request); a block keeps it, the key unchanged.

use rusqlite::{params, Connection};

/// What a nickname given for another key reads as.
pub const PREVIOUS_KEY_SUFFIX: &str = " (name for previous key)";

/// The nickname as it reads for the contact `f.public_key` of `f.owner_key`
/// (a scalar subquery over the `friends f` row).
pub(super) const NICKNAME: &str = "(SELECT CASE WHEN n.for_key = n.contact_key THEN n.nickname \
     ELSE n.nickname || ' (name for previous key)' END \
     FROM friend_nicknames n \
     WHERE n.owner_key = f.owner_key AND n.contact_key = f.public_key)";

/// `name` as a nickname given for the contact's previous key reads.
#[must_use]
pub fn for_previous_key(name: &str) -> String {
    format!("{name}{PREVIOUS_KEY_SUFFIX}")
}

/// Give `public_key` the nickname `nickname`, bound to that key; `None`
/// removes it.
///
/// # Errors
/// The write failed.
pub fn set_nickname(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
    nickname: Option<&str>,
) -> rusqlite::Result<()> {
    match nickname {
        Some(name) => conn.execute(
            "INSERT OR REPLACE INTO friend_nicknames (owner_key, contact_key, for_key, nickname) \
             VALUES (?1, ?2, ?2, ?3)",
            params![owner_key, public_key, name],
        )?,
        None => clear(conn, owner_key, public_key)?,
    };
    Ok(())
}

/// The friendship with `public_key` ended: their nickname goes.
///
/// # Errors
/// The delete failed.
pub fn clear(conn: &Connection, owner_key: &str, public_key: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM friend_nicknames WHERE owner_key = ?1 AND contact_key = ?2",
        params![owner_key, public_key],
    )
}

/// The contact `old` is now `new` (an identity rotation): the nickname
/// follows the contact and stays bound to the key it was given for.
///
/// # Errors
/// The write failed.
pub(crate) fn rekey(
    conn: &Connection,
    owner_key: &str,
    old: &str,
    new: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE friend_nicknames SET contact_key = ?3 WHERE owner_key = ?1 AND contact_key = ?2",
        params![owner_key, old, new],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "nickname_tests.rs"]
mod tests;
