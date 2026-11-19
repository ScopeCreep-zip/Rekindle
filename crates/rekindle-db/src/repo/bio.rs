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
mod tests {
    use super::*;
    use crate::repo::fixture::{conn, OWNER};
    use crate::repo::{friends, outbox};

    #[test]
    fn our_bio_is_kept_and_refused_past_190_characters() {
        let conn = conn();
        assert_eq!(own(&conn, OWNER).unwrap(), None);
        set_own(&conn, OWNER, Some("hi")).unwrap();
        assert_eq!(own(&conn, OWNER).unwrap().as_deref(), Some("hi"));
        assert!(set_own(&conn, OWNER, Some(&"é".repeat(190))).is_ok());
        assert!(set_own(&conn, OWNER, Some(&"x".repeat(191))).is_err());
        assert!(set_own(&conn, OWNER, Some("")).is_err());
        set_own(&conn, OWNER, None).unwrap();
        assert_eq!(own(&conn, OWNER).unwrap(), None);
    }

    #[test]
    fn a_friends_bio_is_kept_and_removing_them_clears_it() {
        let conn = conn();
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
            params![OWNER],
        )
        .unwrap();
        assert!(set_friend(&conn, OWNER, "bob", Some("gg")).unwrap());
        assert!(!set_friend(&conn, OWNER, "carol", Some("gg")).unwrap());
        assert_eq!(friend(&conn, OWNER, "bob").unwrap().as_deref(), Some("gg"));
        let row = outbox::NewRow {
            owner_key: OWNER,
            stream: "friend_handshake:bob",
            client_id: "unfriended:bob",
            kind: "friend_handshake",
            payload: b"{}",
            next_at: 2,
            deadline: 9,
        };
        friends::mark_removing(&conn, OWNER, "bob", &row).unwrap();
        assert_eq!(friend(&conn, OWNER, "bob").unwrap(), None);
    }
}
