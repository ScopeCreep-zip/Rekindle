//! `audit_entries`: the tamper-evident audit chain's rows.
//!
//! Pure I/O: the caller owns the in-memory `AuditChain` and advances it;
//! these functions store and read the entries it produces.

use rekindle_audit::{AuditEntry, AuditRecord};
use rusqlite::{params, Connection};

/// Insert one audit entry. Caller owns chain-state advancement (i.e. the
/// `AuditChain` instance in `AppState::audit_chain`); this is pure I/O.
pub fn insert_entry(
    conn: &Connection,
    owner_key: &str,
    entry: &AuditEntry,
) -> Result<(), rusqlite::Error> {
    let payload_json = serde_json::to_string(&entry.record)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let cursor_i64 = entry.cursor.cast_signed();
    conn.execute(
        "INSERT INTO audit_entries (owner_key, cursor, prev_mac, mac, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            owner_key,
            cursor_i64,
            &entry.prev_mac[..],
            &entry.mac[..],
            payload_json,
        ],
    )?;
    Ok(())
}

/// Load every entry for `owner_key` in cursor order.
pub fn load_all(conn: &Connection, owner_key: &str) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key], row_to_entry)?;
    rows.collect()
}

/// Load entries with `cursor > since_cursor` for export.
pub fn load_since(
    conn: &Connection,
    owner_key: &str,
    since_cursor: u64,
) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    let since_i64 = since_cursor.cast_signed();
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 AND cursor > ?2 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key, since_i64], row_to_entry)?;
    rows.collect()
}

/// Latest (cursor, mac) for `owner_key`, or `(0, [0u8; 32])` if empty.
/// Used to initialize the in-memory `AuditChain` on vault unlock.
pub fn load_tail(conn: &Connection, owner_key: &str) -> Result<(u64, [u8; 32]), rusqlite::Error> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT cursor, mac FROM audit_entries \
             WHERE owner_key = ?1 ORDER BY cursor DESC LIMIT 1",
            params![owner_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cursor_i64, mac_bytes)) = row else {
        return Ok((0, [0u8; 32]));
    };
    if mac_bytes.len() != 32 || cursor_i64 < 0 {
        // Truncated/corrupt — surface during verify rather than panic here.
        return Ok((0, [0u8; 32]));
    }
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&mac_bytes);
    let cursor = cursor_i64.cast_unsigned();
    Ok((cursor, mac))
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let cursor_i64: i64 = row.get(0)?;
    let prev_mac_bytes: Vec<u8> = row.get(1)?;
    let mac_bytes: Vec<u8> = row.get(2)?;
    let payload_json: String = row.get(3)?;
    let cursor = cursor_i64.cast_unsigned();
    let record: AuditRecord = serde_json::from_str(&payload_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let mut prev_mac = [0u8; 32];
    let mut mac = [0u8; 32];
    if prev_mac_bytes.len() != 32 || mac_bytes.len() != 32 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!(
                "audit_entries row at cursor {cursor} has malformed MAC blobs (prev_mac={}B, mac={}B)",
                prev_mac_bytes.len(),
                mac_bytes.len()
            )
            .into(),
        ));
    }
    prev_mac.copy_from_slice(&prev_mac_bytes);
    mac.copy_from_slice(&mac_bytes);
    Ok(AuditEntry {
        cursor,
        prev_mac,
        mac,
        record,
    })
}

#[cfg(test)]
mod tests {
    //! The chain against the real schema: what is stored verifies, a row
    //! edited on disk does not, and dropped rows show against the anchor.

    use super::*;
    use crate::repo::fixture::{self, OWNER};
    use rekindle_audit::{AuditChain, AuditKind, TailCheck, VerifyError, MAC_LEN};

    fn chain(key: u8) -> AuditChain {
        AuditChain::open(zeroize::Zeroizing::new([key; 32]), [0; MAC_LEN], 0)
    }

    fn record(n: u64) -> AuditRecord {
        AuditRecord {
            at_ms: 1_700_000_000_000 + n.cast_signed(),
            actor_pub: OWNER.into(),
            kind: AuditKind::FriendAdded,
            payload: serde_json::json!({ "peer": format!("bob-{n}") }),
        }
    }

    /// Append `n` records for `owner` and store them; the entries in order.
    fn store(conn: &Connection, owner: &str, chain: &mut AuditChain, n: u64) -> Vec<AuditEntry> {
        (1..=n)
            .map(|i| {
                let entry = chain.append(record(i)).unwrap();
                insert_entry(conn, owner, &entry).unwrap();
                entry
            })
            .collect()
    }

    #[test]
    fn stored_entries_load_back_and_verify() {
        let conn = fixture::conn();
        let originals = store(&conn, OWNER, &mut chain(7), 5);
        let loaded = load_all(&conn, OWNER).unwrap();
        assert_eq!(loaded.len(), 5);
        for (a, b) in originals.iter().zip(&loaded) {
            assert_eq!((a.cursor, a.mac, a.prev_mac), (b.cursor, b.mac, b.prev_mac));
            assert_eq!(a.record.actor_pub, b.record.actor_pub);
        }
        chain(7).verify(&loaded).unwrap();
    }

    #[test]
    fn a_row_edited_on_disk_fails_verify_at_its_cursor() {
        let conn = fixture::conn();
        store(&conn, OWNER, &mut chain(7), 3);
        conn.execute(
            "UPDATE audit_entries SET payload_json = \
             '{\"at_ms\":0,\"actor_pub\":\"EVIL\",\"kind\":\"FriendAdded\",\"payload\":{}}' \
             WHERE owner_key = ?1 AND cursor = 2",
            [OWNER],
        )
        .unwrap();
        match chain(7).verify(&load_all(&conn, OWNER).unwrap()) {
            Err(VerifyError::MacMismatch { cursor, .. }) => assert_eq!(cursor, 2),
            other => panic!("expected MacMismatch at cursor 2, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_chain_has_the_genesis_tail() {
        let conn = fixture::conn();
        assert_eq!(load_tail(&conn, OWNER).unwrap(), (0, [0; 32]));
    }

    #[test]
    fn the_stored_tail_resumes_the_chain() {
        let conn = fixture::conn();
        let last = store(&conn, OWNER, &mut chain(7), 4).pop().unwrap();
        let (cursor, mac) = load_tail(&conn, OWNER).unwrap();
        assert_eq!((cursor, mac), (last.cursor, last.mac));
        let mut reopened = AuditChain::open(zeroize::Zeroizing::new([7; 32]), mac, cursor);
        let next = reopened.append(record(99)).unwrap();
        assert_eq!((next.cursor, next.prev_mac), (5, last.mac));
    }

    #[test]
    fn load_since_returns_only_later_cursors() {
        let conn = fixture::conn();
        store(&conn, OWNER, &mut chain(7), 5);
        let cursors: Vec<u64> = load_since(&conn, OWNER, 3)
            .unwrap()
            .iter()
            .map(|e| e.cursor)
            .collect();
        assert_eq!(cursors, [4, 5]);
    }

    /// Dropped trailing rows leave a chain that still verifies; the vault
    /// anchor written by the last real append is what catches it.
    #[test]
    fn dropped_tail_rows_show_against_the_anchor() {
        let conn = fixture::conn();
        let anchor_entry = store(&conn, OWNER, &mut chain(7), 5).pop().unwrap();
        conn.execute(
            "DELETE FROM audit_entries WHERE owner_key = ?1 AND cursor > 3",
            [OWNER],
        )
        .unwrap();
        chain(7).verify(&load_all(&conn, OWNER).unwrap()).unwrap();
        let anchor = (anchor_entry.cursor, anchor_entry.mac);
        let stored = load_tail(&conn, OWNER).unwrap();
        assert_eq!(
            TailCheck::of(Some(anchor), stored),
            TailCheck::Tampered { anchor }
        );
    }

    #[test]
    fn deleting_the_identity_deletes_its_chain() {
        let conn = fixture::conn();
        store(&conn, OWNER, &mut chain(7), 3);
        conn.execute("DELETE FROM identity WHERE public_key = ?1", [OWNER])
            .unwrap();
        assert!(load_all(&conn, OWNER).unwrap().is_empty());
    }

    #[test]
    fn each_owner_has_its_own_chain() {
        let conn = fixture::conn();
        conn.execute(
            "INSERT INTO identity (public_key, created_at) VALUES ('other', 0)",
            [],
        )
        .unwrap();
        store(&conn, OWNER, &mut chain(1), 3);
        store(&conn, "other", &mut chain(2), 3);
        let mine = load_all(&conn, OWNER).unwrap();
        let theirs = load_all(&conn, "other").unwrap();
        assert_eq!((mine.len(), theirs.len()), (3, 3));
        for (a, b) in mine.iter().zip(&theirs) {
            assert_ne!(a.mac, b.mac);
        }
    }
}
