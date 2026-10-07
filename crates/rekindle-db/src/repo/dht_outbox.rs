//! `dht_outbox`: the durable record writes a logout left unsent (plan
//! C7.6h), re-pushed by the next login of the same identity.
//!
//! The next login takes the rows (reads and deletes them in one
//! transaction) before it writes anything, so a row is re-pushed once and
//! never after a newer value the next session wrote. A crash loses the
//! crashed session's unsent writes, never resurrects older ones.

use rusqlite::{params, Connection};

/// One unsent write: the record, the subkey and the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxWrite {
    pub record_key: String,
    pub subkey: u32,
    pub data: Vec<u8>,
}

/// Replace `owner_key`'s unsent writes with `writes`.
///
/// # Errors
/// The write failed.
pub fn save(conn: &Connection, owner_key: &str, writes: &[OutboxWrite]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM dht_outbox WHERE owner_key = ?1",
        params![owner_key],
    )?;
    for write in writes {
        tx.execute(
            "INSERT INTO dht_outbox (owner_key, record_key, subkey, data) VALUES (?1, ?2, ?3, ?4)",
            params![owner_key, write.record_key, write.subkey, write.data],
        )?;
    }
    tx.commit()
}

/// Take `owner_key`'s unsent writes: read them and delete them.
///
/// # Errors
/// The query failed.
pub fn take(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<OutboxWrite>> {
    let tx = conn.unchecked_transaction()?;
    let writes = {
        let mut stmt = tx.prepare(
            "SELECT record_key, subkey, data FROM dht_outbox WHERE owner_key = ?1
             ORDER BY record_key, subkey",
        )?;
        let rows = stmt
            .query_map(params![owner_key], |row| {
                Ok(OutboxWrite {
                    record_key: row.get(0)?,
                    subkey: row.get(1)?,
                    data: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        "DELETE FROM dht_outbox WHERE owner_key = ?1",
        params![owner_key],
    )?;
    tx.commit()?;
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    fn write(record_key: &str, subkey: u32, data: &[u8]) -> OutboxWrite {
        OutboxWrite {
            record_key: record_key.into(),
            subkey,
            data: data.to_vec(),
        }
    }

    #[test]
    fn saved_writes_are_taken_once() {
        let conn = fixture::conn();
        let writes = vec![write("VLD0:a", 0, b"one"), write("VLD0:a", 3, b"two")];
        save(&conn, OWNER, &writes).unwrap();
        assert_eq!(take(&conn, OWNER).unwrap(), writes);
        assert!(
            take(&conn, OWNER).unwrap().is_empty(),
            "taking deletes them"
        );
    }

    #[test]
    fn a_save_replaces_the_previous_logout() {
        let conn = fixture::conn();
        save(&conn, OWNER, &[write("VLD0:a", 0, b"old")]).unwrap();
        save(&conn, OWNER, &[write("VLD0:b", 1, b"new")]).unwrap();
        assert_eq!(
            take(&conn, OWNER).unwrap(),
            vec![write("VLD0:b", 1, b"new")]
        );
    }
}
