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
/// record both name it (plan E2.4). Re-enrolling the device replaces it.
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

/// Swap this device's prekey record and id in the same transaction as
/// the new prekey set. Used only after an acknowledged re-enrolment.
///
/// # Errors
/// The identity row or replacement material could not be stored.
pub fn replace_device_prekeys(
    conn: &mut Connection,
    public_key: &str,
    device_id: &str,
    record_key: &str,
    key_set: &MintedSet,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let updated = tx.execute(
        "UPDATE identity SET device_id = ?2, prekey_record_key = ?3 WHERE public_key = ?1",
        params![public_key, device_id, record_key],
    )?;
    if updated != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    crate::erase::delete(
        &tx,
        "DELETE FROM signal_prekeys WHERE owner_key = ?1",
        params![public_key],
    )?;
    crate::prekeys::store_set_rows(&tx, public_key, key_set)?;
    tx.commit()
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
#[path = "identity_tests.rs"]
mod tests;
