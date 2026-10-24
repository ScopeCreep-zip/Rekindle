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
mod tests {
    use super::*;
    use crate::repo::fixture::{conn, OWNER};
    use crate::repo::{blocked, friends, outbox};

    const BOB: &str = "bb";

    fn nickname_of(conn: &Connection, key: &str) -> Option<String> {
        friends::names(conn, OWNER, key).unwrap().and_then(|n| n.0)
    }

    fn named_friend(conn: &Connection) {
        friends::insert_accepted(conn, OWNER, BOB, "Bob", 1).unwrap();
        set_nickname(conn, OWNER, BOB, Some("Mac buddy")).unwrap();
        assert_eq!(nickname_of(conn, BOB).as_deref(), Some("Mac buddy"));
    }

    #[test]
    fn a_removal_clears_the_nickname_and_a_re_add_starts_clean() {
        let conn = conn();
        named_friend(&conn);
        let stream = format!("friend_handshake:{BOB}");
        let row = outbox::NewRow {
            owner_key: OWNER,
            stream: &stream,
            client_id: "unfriended:bb",
            kind: "friend_handshake",
            payload: b"{}",
            next_at: 2,
            deadline: 9,
        };
        friends::mark_removing(&conn, OWNER, BOB, &row).unwrap();
        assert_eq!(nickname_of(&conn, BOB), None, "gone while removing");
        friends::insert_pending_out(&conn, OWNER, BOB, "Bob", 3).unwrap();
        assert_eq!(nickname_of(&conn, BOB), None, "the re-add starts clean");
    }

    #[test]
    fn being_unfriended_or_cancelling_clears_it() {
        let conn = conn();
        named_friend(&conn);
        friends::unfriended(&conn, OWNER, BOB).unwrap();
        friends::insert_accepted(&conn, OWNER, BOB, "Bob", 2).unwrap();
        assert_eq!(nickname_of(&conn, BOB), None);
        set_nickname(&conn, OWNER, BOB, Some("Mac buddy")).unwrap();
        friends::cancel_request(&conn, OWNER, BOB).unwrap();
        friends::insert_accepted(&conn, OWNER, BOB, "Bob", 3).unwrap();
        assert_eq!(nickname_of(&conn, BOB), None);
    }

    #[test]
    fn a_block_and_unblock_of_the_same_key_keep_it() {
        let conn = conn();
        named_friend(&conn);
        friends::block(&conn, OWNER, BOB, "Bob", 2).unwrap();
        blocked::delete(&conn, OWNER, BOB).unwrap();
        friends::insert_accepted(&conn, OWNER, BOB, "Bob", 3).unwrap();
        assert_eq!(nickname_of(&conn, BOB).as_deref(), Some("Mac buddy"));
    }

    #[test]
    fn after_a_key_change_it_names_the_previous_key_until_set_again() {
        let conn = conn();
        named_friend(&conn);
        conn.execute(
            "UPDATE friends SET public_key = 'cc' WHERE public_key = ?1",
            params![BOB],
        )
        .unwrap();
        rekey(&conn, OWNER, BOB, "cc").unwrap();
        assert_eq!(
            nickname_of(&conn, "cc"),
            Some(for_previous_key("Mac buddy"))
        );
        assert_eq!(
            friends::load_all(&conn, OWNER).unwrap()[0].nickname,
            Some(for_previous_key("Mac buddy"))
        );
        set_nickname(&conn, OWNER, "cc", Some("Mac buddy")).unwrap();
        assert_eq!(nickname_of(&conn, "cc").as_deref(), Some("Mac buddy"));
    }
}
