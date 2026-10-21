//! `identity`: the database's one identity row (each identity has its own
//! database, plan D1), with the keys of the DHT records it owns. The
//! identities a root holds are listed by the registry
//! (`crate::registry`), not by this table.

use rekindle_crypto::signal::prekey_record::MintedSet;
use rusqlite::{params, Connection, OptionalExtension as _};

/// The keys of the DHT records an identity owns. Their owner keypairs are
/// in the vault (`rekindle_vault::typed::owner_keys`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityDhtColumns {
    pub profile: Option<String>,
    pub friend_list: Option<String>,
    pub account: Option<String>,
    pub mailbox: Option<String>,
    /// This device's prekey record (plan E2.4).
    pub prekeys: Option<String>,
    /// This device's id (hex), which the prekey device list names.
    pub device_id: Option<String>,
}

/// A DHT record the identity owns with a keypair of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedRecord {
    Profile,
    FriendList,
    Account,
    /// This device's prekey record (plan E2.4).
    Prekeys,
}

impl OwnedRecord {
    fn key_column(self) -> &'static str {
        match self {
            Self::Profile => "dht_record_key",
            Self::FriendList => "friend_list_dht_key",
            Self::Account => "account_dht_key",
            Self::Prekeys => "prekey_record_key",
        }
    }
}

/// Create the identity's row with its first friend-request token
/// (`rekindle_secrets::friend_token::mint`, plan E2.5).
///
/// # Errors
/// The write failed (an identity with this key already exists, or the
/// token is not 26 base32 characters).
pub fn insert(
    conn: &Connection,
    public_key: &str,
    display_name: &str,
    created_at: i64,
    friend_request_token: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO identity (public_key, display_name, created_at, friend_request_token) \
         VALUES (?1, ?2, ?3, ?4)",
        params![public_key, display_name, created_at, friend_request_token],
    )?;
    Ok(())
}

/// The friend-request token our friend code carries; `None` when there is
/// no such identity.
///
/// # Errors
/// The query failed.
pub fn friend_request_token(
    conn: &Connection,
    public_key: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT friend_request_token FROM identity WHERE public_key = ?1",
        params![public_key],
        |row| row.get(0),
    )
    .optional()
}

/// Replace the friend-request token (a rotation): requests carrying the
/// old one are dropped from now on.
///
/// # Errors
/// The write failed (the token is not 26 base32 characters).
pub fn set_friend_request_token(
    conn: &Connection,
    public_key: &str,
    token: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE identity SET friend_request_token = ?2 WHERE public_key = ?1",
        params![public_key, token],
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
         mailbox_dht_key, prekey_record_key, device_id FROM identity WHERE public_key = ?1",
        params![public_key],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                IdentityDhtColumns {
                    profile: row.get(1)?,
                    friend_list: row.get(2)?,
                    account: row.get(3)?,
                    mailbox: row.get(4)?,
                    prekeys: row.get(5)?,
                    device_id: row.get(6)?,
                },
            ))
        },
    )
    .optional()
}

/// This device's id, set to `minted` (16 random bytes, lowercase hex)
/// when the row has none: the prekey device list and the personal sync
/// record both name it (plan E2.4), and nothing replaces it once set.
///
/// # Errors
/// The write failed (`minted` is not 32 lowercase hex characters).
pub fn ensure_device_id(
    conn: &Connection,
    public_key: &str,
    minted: &str,
) -> rusqlite::Result<String> {
    conn.execute(
        "UPDATE identity SET device_id = ?2 WHERE public_key = ?1 AND device_id IS NULL",
        params![public_key, minted],
    )?;
    conn.query_row(
        "SELECT device_id FROM identity WHERE public_key = ?1",
        params![public_key],
        |row| row.get(0),
    )
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

/// Re-key the database to a rotated identity: the identity row and every
/// row owned by it (each table's `owner_key`). Foreign keys are checked
/// at commit, so the identity row and its dependants move together.
///
/// `prekeys` is the new identity's minted set (plan E2.4: head and every
/// slot, signed by the new key and already published to the same
/// record); it is stored under `new_key` in the same transaction.
///
/// # Errors
/// A write failed; the transaction rolls back.
pub fn rekey(
    conn: &mut Connection,
    old_key: &str,
    new_key: &str,
    prekeys: Option<&MintedSet>,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
    // Our sessions bind the old identity in every AD and our prekeys are
    // signed by it: neither carries over (plan E2.3), nor does the
    // bookkeeping of inits to those prekeys (E2.4). Friends' next
    // messages open sessions from the record the new identity publishes.
    for table in [
        "ratchet_sessions",
        "signal_prekeys",
        "prekey_lr_inits",
        "prekey_rejections",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE owner_key = ?1"),
            params![old_key],
        )?;
    }
    // What the old identity was addressed or signed as does not carry
    // over (audit 2026-10-20 R-2, R-3): open invites carry the old key and
    // are cancelled; requests out (`pending_out` rows) and in were
    // addressed to or from it and go; frames queued under the deleted
    // sessions (a `friend_handshake` row's wired frame) can never be read. Our own messages move
    // to the new key, and a verification goes with the safety number it
    // was made over, which the new key changes (Signal resets it).
    tx.execute(
        "UPDATE outgoing_invites SET status = 'cancelled' \
         WHERE owner_key = ?1 AND status IN ('pending', 'responded')",
        params![old_key],
    )?;
    tx.execute(
        "DELETE FROM outbox AS o WHERE o.owner_key = ?1 AND o.kind IN ('dm', 'friend_handshake') \
         AND substr(o.stream, instr(o.stream, ':') + 1) IN \
         (SELECT public_key FROM friends WHERE owner_key = ?1 AND friendship_state = 'pending_out')",
        params![old_key],
    )?;
    for sql in [
        "DELETE FROM friends WHERE owner_key = ?1 AND friendship_state = 'pending_out'",
        "DELETE FROM pending_friend_requests WHERE owner_key = ?1",
        "DELETE FROM outbox WHERE owner_key = ?1 AND kind = 'friend_handshake' \
         AND json_extract(CAST(payload AS TEXT), '$.kind') = 'wire'",
        "UPDATE signal_identities SET verified = 0, verified_at = NULL WHERE owner_key = ?1",
    ] {
        tx.execute(sql, params![old_key])?;
    }
    for column in ["sender_key", "forwarded_from_author"] {
        tx.execute(
            &format!("UPDATE messages SET {column} = ?1 WHERE owner_key = ?2 AND {column} = ?2"),
            params![new_key, old_key],
        )?;
    }
    let owned: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT m.name FROM sqlite_master m, pragma_table_info(m.name) c \
             WHERE m.type = 'table' AND m.sql NOT LIKE 'CREATE VIRTUAL%' \
             AND c.name = 'owner_key' ORDER BY m.name",
        )?;
        let names = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        names
    };
    for table in owned {
        tx.execute(
            &format!("UPDATE \"{table}\" SET owner_key = ?1 WHERE owner_key = ?2"),
            params![new_key, old_key],
        )?;
    }
    // The mailbox record is owned by the old identity keypair, which no
    // longer signs: the next unlock creates one the new identity owns.
    tx.execute(
        "UPDATE identity SET public_key = ?1, mailbox_dht_key = NULL WHERE public_key = ?2",
        params![new_key, old_key],
    )?;
    if let Some(set) = prekeys {
        crate::prekeys::store_set_rows(&tx, new_key, set)?;
    }
    tx.commit()
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

/// Set the personal sync record's key (its keypair is stored in the vault
/// first) and answer this device's id: the one held, or `minted` when the
/// row has none ([`ensure_device_id`]; the record never replaces it).
///
/// # Errors
/// The write failed.
pub fn set_personal_sync(
    conn: &Connection,
    public_key: &str,
    record_key: &str,
    minted_device_id: &str,
) -> rusqlite::Result<String> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE identity SET personal_sync_record_key = ?1 WHERE public_key = ?2",
        params![record_key, public_key],
    )?;
    let device_id = ensure_device_id(&tx, public_key, minted_device_id)?;
    tx.commit()?;
    Ok(device_id)
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
        set_owned_record(&conn, OWNER, OwnedRecord::Prekeys, "pk").unwrap();
        let (name, cols) = login_row(&conn, OWNER).unwrap().unwrap();
        assert_eq!(name, "me");
        assert_eq!(cols.profile.as_deref(), Some("k2"));
        assert_eq!(cols.mailbox.as_deref(), Some("mb"));
        assert_eq!(cols.prekeys.as_deref(), Some("pk"));
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
    fn rekey_moves_the_identity_and_everything_it_owns() {
        let mut conn = fixture::conn();
        crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        set_mailbox_key(&conn, OWNER, "mb").unwrap();
        rekey(&mut conn, OWNER, "new-pk", None).unwrap();
        assert_eq!(login_row(&conn, OWNER).unwrap(), None);
        // The old identity's mailbox does not carry over.
        let (_, cols) = login_row(&conn, "new-pk").unwrap().unwrap();
        assert_eq!(cols.mailbox, None);
        assert_eq!(
            crate::repo::friends::load_all(&conn, "new-pk")
                .unwrap()
                .len(),
            1
        );
        assert!(crate::repo::friends::load_all(&conn, OWNER)
            .unwrap()
            .is_empty());
        let violations: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    /// Our own rotation (audit 2026-10-20 R-2, R-3): our messages move to
    /// the new key, verifications are cleared, open invites are cancelled,
    /// requests to or from the old key go, and so do frames queued under
    /// the deleted sessions; sealed queued kinds stay.
    #[test]
    fn rekey_settles_what_the_old_key_was_addressed_as() {
        let mut conn = fixture::conn();
        crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
               VALUES ('{OWNER}', 'carol', 1, 'pending_out'); \
             INSERT INTO pending_friend_requests (owner_key, public_key, display_name, received_at) \
               VALUES ('{OWNER}', 'dave', 'Dave', 1); \
             INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp) \
               VALUES ('{OWNER}', 'bob', 'dm', '{OWNER}', 'mine', 1), \
                      ('{OWNER}', 'bob', 'dm', 'bob', 'theirs', 2); \
             INSERT INTO signal_identities (owner_key, peer_identity, identity_key, verified, verified_at) \
               VALUES ('{OWNER}', 'bob', zeroblob(32), 1, 5);"
        ))
        .unwrap();
        for (stream, id, payload) in [
            (
                "friend_handshake:bob",
                "w",
                &br#"{"kind":"wire","datagram":"AA=="}"#[..],
            ),
            ("friend_handshake:bob", "s", br#"{"kind":"sealed"}"#),
            ("friend_handshake:carol", "c", br#"{"kind":"sealed"}"#),
        ] {
            crate::repo::outbox::queue_for_test(&conn, OWNER, stream, id, payload);
        }
        insert_invite(&conn, "i1");
        rekey(&mut conn, OWNER, "new-pk", None).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM messages WHERE sender_key = 'new-pk' AND body = 'mine'"),
            1
        );
        assert_eq!(
            count("SELECT count(*) FROM messages WHERE sender_key = 'bob'"),
            1
        );
        assert_eq!(
            count("SELECT count(*) FROM signal_identities WHERE verified = 1"),
            0
        );
        assert_eq!(
            count("SELECT count(*) FROM outgoing_invites WHERE status = 'cancelled'"),
            1
        );
        assert_eq!(count("SELECT count(*) FROM pending_friend_requests"), 0);
        assert_eq!(count("SELECT count(*) FROM friends"), 1);
        assert_eq!(
            count("SELECT count(*) FROM outbox WHERE owner_key = 'new-pk' AND client_id = 's'"),
            1
        );
    }

    fn insert_invite(conn: &Connection, id: &str) {
        crate::repo::outgoing_invites::insert(conn, OWNER, id, "u", "inbox", 1).unwrap();
    }

    /// The rotation stores the new identity's minted set under the new
    /// key, after the old identity's prekeys and their bookkeeping go, in
    /// the same transaction.
    #[test]
    fn rekey_replaces_the_prekeys_with_the_new_set() {
        use rekindle_crypto::signal::prekey_record::mint_set;
        let mut conn = fixture::conn();
        let old = rekindle_crypto::Identity::generate();
        let rng = rekindle_utils::random::OsRandom;
        crate::prekeys::store_set_rows(&conn, OWNER, &mint_set(&old, &rng, 1)).unwrap();
        conn.execute(
            "INSERT INTO prekey_lr_inits VALUES (?1, 1, 1, zeroblob(32), 0)",
            params![OWNER],
        )
        .unwrap();
        let new = rekindle_crypto::Identity::generate();
        let set = mint_set(&new, &rng, 2);
        let head = set.head.spk_id;
        rekey(&mut conn, OWNER, "new-pk", Some(&set)).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM signal_prekeys WHERE owner_key = 'new-pk'"),
            202
        );
        assert_eq!(count("SELECT count(*) FROM signal_prekeys"), 202);
        assert_eq!(count("SELECT count(*) FROM prekey_lr_inits"), 0);
        let spk: u32 = conn
            .query_row(
                "SELECT id FROM signal_prekeys WHERE kind = 'spk' AND owner_key = 'new-pk'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(spk, head);
    }

    const DEV_A: &str = "0123456789abcdef0123456789abcdef";
    const DEV_B: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn the_device_id_is_minted_once() {
        let conn = fixture::conn();
        assert_eq!(ensure_device_id(&conn, OWNER, DEV_A).unwrap(), DEV_A);
        assert_eq!(ensure_device_id(&conn, OWNER, DEV_B).unwrap(), DEV_A);
        let (_, cols) = login_row(&conn, OWNER).unwrap().unwrap();
        assert_eq!(cols.device_id.as_deref(), Some(DEV_A));
    }

    /// Audit D-9: a device id is 32 lowercase hex characters.
    #[test]
    fn a_malformed_device_id_is_refused() {
        let conn = fixture::conn();
        for bad in [
            "",
            "aa",
            "0123456789ABCDEF0123456789ABCDEF",
            "g123456789abcdef0123456789abcdef",
        ] {
            assert!(ensure_device_id(&conn, OWNER, bad).is_err(), "{bad:?}");
        }
    }

    /// Audit D-6: the personal sync record names the device id the
    /// own-records stage minted; it never replaces it.
    #[test]
    fn personal_sync_keeps_the_device_id() {
        let conn = fixture::conn();
        assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
        ensure_device_id(&conn, OWNER, DEV_A).unwrap();
        assert_eq!(set_personal_sync(&conn, OWNER, "rk", DEV_B).unwrap(), DEV_A);
        assert_eq!(
            personal_sync(&conn, OWNER).unwrap(),
            Some(("rk".into(), DEV_A.into()))
        );
        let (_, cols) = login_row(&conn, OWNER).unwrap().unwrap();
        assert_eq!(cols.device_id.as_deref(), Some(DEV_A));
    }

    #[test]
    fn personal_sync_mints_the_device_id_when_none_is_held() {
        let conn = fixture::conn();
        assert_eq!(set_personal_sync(&conn, OWNER, "rk", DEV_B).unwrap(), DEV_B);
        assert_eq!(ensure_device_id(&conn, OWNER, DEV_A).unwrap(), DEV_B);
    }
}
