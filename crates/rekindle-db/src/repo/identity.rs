//! `identity`: one row per local identity, with the keys of the DHT
//! records it owns.

use rusqlite::{params, Connection, OptionalExtension as _};

/// An identity as the login screen lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRow {
    pub public_key: String,
    pub display_name: String,
    pub created_at: i64,
    pub avatar_webp: Option<Vec<u8>>,
}

/// The DHT records an identity owns, as stored: each key, and the owner
/// keypair for the records that have one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityDhtColumns {
    pub existing_dht_key: Option<String>,
    pub existing_friend_list_key: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub friend_list_owner_keypair: Option<String>,
    pub account_dht_key: Option<String>,
    pub account_owner_keypair: Option<String>,
    pub mailbox_dht_key: Option<String>,
}

/// A DHT record the identity owns with a keypair of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedRecord {
    Profile,
    FriendList,
    Account,
}

impl OwnedRecord {
    fn columns(self) -> (&'static str, &'static str) {
        match self {
            Self::Profile => ("dht_record_key", "dht_owner_keypair"),
            Self::FriendList => ("friend_list_dht_key", "friend_list_owner_keypair"),
            Self::Account => ("account_dht_key", "account_owner_keypair"),
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

/// Delete the identity; every per-owner row goes with it (`ON DELETE
/// CASCADE`).
///
/// # Errors
/// The write failed.
pub fn delete(conn: &Connection, public_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM identity WHERE public_key = ?1",
        params![public_key],
    )?;
    Ok(())
}

/// Every identity, oldest first.
///
/// # Errors
/// The query failed.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<IdentityRow>> {
    let mut stmt = conn.prepare(
        "SELECT public_key, display_name, created_at, avatar_webp \
         FROM identity ORDER BY created_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(IdentityRow {
                public_key: row.get(0)?,
                display_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                created_at: row.get(2)?,
                avatar_webp: row.get(3)?,
            })
        })?
        .collect();
    rows
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
        "SELECT display_name, dht_record_key, friend_list_dht_key, dht_owner_keypair, \
         friend_list_owner_keypair, account_dht_key, account_owner_keypair, mailbox_dht_key \
         FROM identity WHERE public_key = ?1",
        params![public_key],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                IdentityDhtColumns {
                    existing_dht_key: row.get(1)?,
                    existing_friend_list_key: row.get(2)?,
                    dht_owner_keypair: row.get(3)?,
                    friend_list_owner_keypair: row.get(4)?,
                    account_dht_key: row.get(5)?,
                    account_owner_keypair: row.get(6)?,
                    mailbox_dht_key: row.get(7)?,
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

/// Set an owned record's key, and its keypair when one is given (a
/// reopened record keeps the stored keypair).
///
/// # Errors
/// The write failed.
pub fn set_owned_record(
    conn: &Connection,
    public_key: &str,
    record: OwnedRecord,
    key: &str,
    keypair: Option<&str>,
) -> rusqlite::Result<()> {
    let (key_col, keypair_col) = record.columns();
    conn.execute(
        &format!(
            "UPDATE identity SET {key_col} = ?1, {keypair_col} = COALESCE(?2, {keypair_col}) \
             WHERE public_key = ?3"
        ),
        params![key, keypair, public_key],
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

/// The personal sync record: `(record key, owner keypair, device id)`,
/// when all three are set.
///
/// # Errors
/// The query failed.
pub fn personal_sync(
    conn: &Connection,
    public_key: &str,
) -> rusqlite::Result<Option<(String, String, String)>> {
    let row = conn
        .query_row(
            "SELECT personal_sync_record_key, personal_sync_owner_keypair, device_id \
             FROM identity WHERE public_key = ?1",
            params![public_key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        Some((Some(k), Some(p), Some(d))) if !k.is_empty() && !p.is_empty() && !d.is_empty() => {
            Some((k, p, d))
        }
        _ => None,
    })
}

/// Set the personal sync record.
///
/// # Errors
/// The write failed.
pub fn set_personal_sync(
    conn: &Connection,
    public_key: &str,
    record_key: &str,
    owner_keypair: &str,
    device_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET personal_sync_record_key = ?1, personal_sync_owner_keypair = ?2, \
         device_id = ?3 WHERE public_key = ?4",
        params![record_key, owner_keypair, device_id, public_key],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_reopened_record_keeps_its_stored_keypair() {
        let conn = fixture::conn();
        set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k1", Some("kp1")).unwrap();
        set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k2", None).unwrap();
        set_mailbox_key(&conn, OWNER, "mb").unwrap();
        let (name, cols) = login_row(&conn, OWNER).unwrap().unwrap();
        assert_eq!(name, "me");
        assert_eq!(cols.existing_dht_key.as_deref(), Some("k2"));
        assert_eq!(cols.dht_owner_keypair.as_deref(), Some("kp1"));
        assert_eq!(cols.mailbox_dht_key.as_deref(), Some("mb"));
        assert_eq!(cols.account_dht_key, None);
        assert_eq!(login_row(&conn, "ghost").unwrap(), None);
    }

    #[test]
    fn identities_list_oldest_first_and_delete_cascades() {
        let conn = fixture::conn();
        insert(&conn, "second", "Two", 5).unwrap();
        set_avatar(&conn, "second", &[9]).unwrap();
        let listed = list(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].public_key, OWNER);
        assert_eq!(listed[1].avatar_webp.as_deref(), Some(&[9][..]));
        assert_eq!(avatar(&conn, "second").unwrap(), Some(vec![9]));
        assert_eq!(avatar(&conn, OWNER).unwrap(), None);
        crate::repo::friends::insert_accepted(&conn, "second", "bob", "Bob", 1).unwrap();
        delete(&conn, "second").unwrap();
        assert!(crate::repo::friends::load_all(&conn, "second")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn personal_sync_needs_all_three_fields() {
        let conn = fixture::conn();
        assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
        set_personal_sync(&conn, OWNER, "rk", "kp", "dev").unwrap();
        assert_eq!(
            personal_sync(&conn, OWNER).unwrap(),
            Some(("rk".into(), "kp".into(), "dev".into()))
        );
        set_personal_sync(&conn, OWNER, "rk", "", "dev").unwrap();
        assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
    }
}
