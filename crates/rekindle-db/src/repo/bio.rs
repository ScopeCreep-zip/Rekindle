//! Bios (plan E7.2): ours (`identity.bio`) and each friend's as their last
//! `ProfileBio` DmFrame (0x0171) stated it (`friends.bio`). Never in the
//! profile record, whose key travels in every invite and friend code. A
//! friend's goes when we remove them ([`super::friends::mark_removing`]) and
//! with their row otherwise. `None` is no bio; the table refuses an empty or
//! over-long one (190 characters, Discord's About Me).

use rusqlite::{params, Connection, OptionalExtension as _};

/// Our bio, or `None`.
///
/// # Errors
/// The query failed.
pub fn own(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT bio FROM identity WHERE public_key = ?1",
        params![owner_key],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Set our bio; `None` removes it.
///
/// # Errors
/// The write failed, or the bio breaks the column's check.
pub fn set_own(conn: &Connection, owner_key: &str, bio: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET bio = ?2 WHERE public_key = ?1",
        params![owner_key, bio],
    )?;
    Ok(())
}

/// `friend_key`'s bio, or `None` (none stated, or no such friend).
///
/// # Errors
/// The query failed.
pub fn friend(
    conn: &Connection,
    owner_key: &str,
    friend_key: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT bio FROM friends WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, friend_key],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Keep `friend_key`'s bio; `None` clears it. Whether a friend row took it.
///
/// # Errors
/// The write failed, or the bio breaks the column's check.
pub fn set_friend(
    conn: &Connection,
    owner_key: &str,
    friend_key: &str,
    bio: Option<&str>,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE friends SET bio = ?3 WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, friend_key, bio],
    )?;
    Ok(changed > 0)
}

#[cfg(test)]
#[path = "bio_tests.rs"]
mod tests;
