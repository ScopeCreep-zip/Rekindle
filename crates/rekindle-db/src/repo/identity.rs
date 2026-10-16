//! `identity`: the database's one identity row (each identity has its own
//! database, plan D1), with the keys of the DHT records it owns. The
//! identities a root holds are listed by the registry
//! (`crate::registry`), not by this table.

use rusqlite::{params, Connection, OptionalExtension as _};

/// The keys of the DHT records an identity owns. Their owner keypairs are
/// in the vault (`rekindle_vault::typed::owner_keys`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityDhtColumns {
    pub profile: Option<String>,
    pub friend_list: Option<String>,
    pub account: Option<String>,
    pub mailbox: Option<String>,
}

/// A DHT record the identity owns with a keypair of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedRecord {
    Profile,
    FriendList,
    Account,
}

impl OwnedRecord {
    fn key_column(self) -> &'static str {
        match self {
            Self::Profile => "dht_record_key",
            Self::FriendList => "friend_list_dht_key",
            Self::Account => "account_dht_key",
        }
    }
}

/// Create the identity's row.
///
/// # Errors
/// The write failed (an identity with this key already exists).
pub fn insert(
    conn: &Connection,
    public_key: &str,
    display_name: &str,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO identity (public_key, display_name, created_at) VALUES (?1, ?2, ?3)",
        params![public_key, display_name, created_at],
    )?;
    Ok(())
}

/// The identity's display name and DHT records; `None` when there is no
/// such identity.
///
/// # Errors
/// The query failed.
pub fn login_row(
    conn: &Connection,
    public_key: &str,
) -> rusqlite::Result<Option<(String, IdentityDhtColumns)>> {
    conn.query_row(
        "SELECT display_name, dht_record_key, friend_list_dht_key, account_dht_key, \
         mailbox_dht_key FROM identity WHERE public_key = ?1",
        params![public_key],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                IdentityDhtColumns {
                    profile: row.get(1)?,
                    friend_list: row.get(2)?,
                    account: row.get(3)?,
                    mailbox: row.get(4)?,
                },
            ))
        },
    )
    .optional()
}

/// Set the display name.
///
/// # Errors
/// The write failed.
pub fn set_display_name(
    conn: &Connection,
    public_key: &str,
    display_name: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET display_name = ?1 WHERE public_key = ?2",
        params![display_name, public_key],
    )?;
    Ok(())
}

/// Set the avatar (WebP bytes).
///
/// # Errors
/// The write failed.
pub fn set_avatar(conn: &Connection, public_key: &str, webp: &[u8]) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET avatar_webp = ?1 WHERE public_key = ?2",
        params![webp, public_key],
    )?;
    Ok(())
}

/// The identity's avatar, if it has one.
///
/// # Errors
/// The query failed.
pub fn avatar(conn: &Connection, public_key: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT avatar_webp FROM identity WHERE public_key = ?1",
            params![public_key],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

/// Set an owned record's key (its keypair is stored in the vault first).
///
/// # Errors
/// The write failed.
pub fn set_owned_record(
    conn: &Connection,
    public_key: &str,
    record: OwnedRecord,
    key: &str,
) -> rusqlite::Result<()> {
    let key_col = record.key_column();
    conn.execute(
        &format!("UPDATE identity SET {key_col} = ?1 WHERE public_key = ?2"),
        params![key, public_key],
    )?;
    Ok(())
}

/// Set the mailbox record's key (the mailbox's keypair is the identity's
/// own, so none is stored).
///
/// # Errors
/// The write failed.
pub fn set_mailbox_key(conn: &Connection, public_key: &str, key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET mailbox_dht_key = ?1 WHERE public_key = ?2",
        params![key, public_key],
    )?;
    Ok(())
}

/// Set the friend inbox and its (published) keypair.
///
/// # Errors
/// The write failed.
pub fn set_friend_inbox(
    conn: &Connection,
    public_key: &str,
    inbox_key: &str,
    inbox_keypair: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET friend_inbox_key = ?1, friend_inbox_keypair = ?2 \
         WHERE public_key = ?3",
        params![inbox_key, inbox_keypair, public_key],
    )?;
    Ok(())
}

/// The friend inbox and its keypair, when both are set.
///
/// # Errors
/// The query failed.
pub fn friend_inbox(
    conn: &Connection,
    public_key: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    let row = conn
        .query_row(
            "SELECT friend_inbox_key, friend_inbox_keypair FROM identity WHERE public_key = ?1",
            params![public_key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        Some((Some(k), Some(p))) if !k.is_empty() && !p.is_empty() => Some((k, p)),
        _ => None,
    })
}

/// The personal sync record: `(record key, device id)`, when both are set
/// (its owner keypair is in the vault).
///
/// # Errors
/// The query failed.
pub fn personal_sync(
    conn: &Connection,
    public_key: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    let row = conn
        .query_row(
            "SELECT personal_sync_record_key, device_id FROM identity WHERE public_key = ?1",
            params![public_key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        Some((Some(k), Some(d))) if !k.is_empty() && !d.is_empty() => Some((k, d)),
        _ => None,
    })
}

/// Set the personal sync record (its keypair is stored in the vault first).
///
/// # Errors
/// The write failed.
pub fn set_personal_sync(
    conn: &Connection,
    public_key: &str,
    record_key: &str,
    device_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET personal_sync_record_key = ?1, device_id = ?2 WHERE public_key = ?3",
        params![record_key, device_id, public_key],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn an_owned_record_key_is_replaced() {
        let conn = fixture::conn();
        set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k1").unwrap();
        set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k2").unwrap();
        set_mailbox_key(&conn, OWNER, "mb").unwrap();
        let (name, cols) = login_row(&conn, OWNER).unwrap().unwrap();
        assert_eq!(name, "me");
        assert_eq!(cols.profile.as_deref(), Some("k2"));
        assert_eq!(cols.mailbox.as_deref(), Some("mb"));
        assert_eq!(cols.account, None);
        assert_eq!(login_row(&conn, "ghost").unwrap(), None);
    }

    #[test]
    fn the_avatar_round_trips() {
        let conn = fixture::conn();
        assert_eq!(avatar(&conn, OWNER).unwrap(), None);
        set_avatar(&conn, OWNER, &[9]).unwrap();
        assert_eq!(avatar(&conn, OWNER).unwrap(), Some(vec![9]));
    }

    #[test]
    fn the_friend_inbox_round_trips() {
        let conn = fixture::conn();
        assert_eq!(friend_inbox(&conn, OWNER).unwrap(), None);
        set_friend_inbox(&conn, OWNER, "ik", "ikp").unwrap();
        assert_eq!(
            friend_inbox(&conn, OWNER).unwrap(),
            Some(("ik".into(), "ikp".into()))
        );
    }

    #[test]
    fn personal_sync_needs_both_fields() {
        let conn = fixture::conn();
        assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
        set_personal_sync(&conn, OWNER, "rk", "dev").unwrap();
        assert_eq!(
            personal_sync(&conn, OWNER).unwrap(),
            Some(("rk".into(), "dev".into()))
        );
        set_personal_sync(&conn, OWNER, "rk", "").unwrap();
        assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
    }
}
