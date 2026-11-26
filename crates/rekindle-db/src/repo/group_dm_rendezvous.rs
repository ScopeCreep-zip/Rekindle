//! `group_dm_rendezvous` (plan decision 67 item 3): our place in a group's
//! rendezvous record — the record, our slot, our one-time writer of its
//! row and every member's slot from the creator's signed plan. The owner
//! of its behaviour is `rekindle-dm` (`GroupDmDeps::{put_rendezvous,
//! rendezvous}`); the row goes with its group (`ON DELETE CASCADE`).

use rusqlite::{params, Connection, OptionalExtension as _};

/// Our place in one group's rendezvous record.
#[derive(Clone, PartialEq, Eq)]
pub struct RendezvousRow {
    pub group_id: String,
    pub record_key: String,
    /// Our row (we write it) and column (we watch it), `0..8`.
    pub slot: u8,
    /// Our one-time writer of row `slot` (keypair text).
    pub writer: String,
    /// Every member's slot, `(identity, slot)`.
    pub slots: Vec<(String, u8)>,
}

/// The writer is never printed.
impl std::fmt::Debug for RendezvousRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendezvousRow")
            .field("group_id", &self.group_id)
            .field("record_key", &self.record_key)
            .field("slot", &self.slot)
            .field("slots", &self.slots)
            .finish_non_exhaustive()
    }
}

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// Store our place in `row.group_id`'s record, replacing the one held.
///
/// # Errors
/// The write failed (a column is refused, or the group is not held).
pub fn put(conn: &Connection, owner_key: &str, row: &RendezvousRow) -> rusqlite::Result<()> {
    let slots = serde_json::to_string(&row.slots).map_err(json_err)?;
    conn.execute(
        "INSERT INTO group_dm_rendezvous (owner_key, group_id, record_key, slot, writer, slots) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(owner_key, group_id) DO UPDATE SET \
         record_key = excluded.record_key, slot = excluded.slot, writer = excluded.writer, \
         slots = excluded.slots",
        params![
            owner_key,
            row.group_id,
            row.record_key,
            row.slot,
            row.writer,
            slots
        ],
    )?;
    Ok(())
}

/// Our place in `group_id`'s record, if held.
///
/// # Errors
/// The query failed or the row is corrupt.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    group_id: &str,
) -> rusqlite::Result<Option<RendezvousRow>> {
    conn.query_row(
        "SELECT group_id, record_key, slot, writer, slots FROM group_dm_rendezvous \
         WHERE owner_key = ?1 AND group_id = ?2",
        params![owner_key, group_id],
        |r| {
            Ok(RendezvousRow {
                group_id: crate::row::get(r, "group_id")?,
                record_key: crate::row::get(r, "record_key")?,
                slot: crate::row::get(r, "slot")?,
                writer: crate::row::get(r, "writer")?,
                slots: crate::row::get_json(r, "slots")?,
            })
        },
    )
    .optional()
}

/// Every group's place we hold: the watches the Host holds at unlock.
///
/// # Errors
/// The query failed or a row is corrupt.
pub fn all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<RendezvousRow>> {
    let mut stmt = conn.prepare(
        "SELECT group_id, record_key, slot, writer, slots FROM group_dm_rendezvous \
         WHERE owner_key = ?1 ORDER BY group_id",
    )?;
    let rows = stmt.query_map(params![owner_key], |r| {
        Ok(RendezvousRow {
            group_id: crate::row::get(r, "group_id")?,
            record_key: crate::row::get(r, "record_key")?,
            slot: crate::row::get(r, "slot")?,
            writer: crate::row::get(r, "writer")?,
            slots: crate::row::get_json(r, "slots")?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};
    use crate::repo::group_dms::{self, GroupDmRow};
    use rekindle_types::group_dm::GroupDmRosterMember;

    fn db() -> Connection {
        let conn = fixture::conn();
        let member = |n: char| GroupDmRosterMember {
            identity: n.to_string().repeat(64),
            profile_record_key: format!("VLD0:{}", "P".repeat(43)),
            display_name: format!("member {n}"),
        };
        let group = GroupDmRow {
            group_id: "ab".repeat(16),
            name: "raid night".to_string(),
            roster: vec![member('a'), member('b'), member('c')],
            epoch: 0,
            created_at: 5,
            origin_peer: None,
            favourite: false,
        };
        group_dms::insert(&conn, OWNER, &group).expect("group");
        conn
    }

    /// Accept stores the slot; a grow replaces the plan; leaving (the group
    /// deleted) deletes the row.
    #[test]
    fn a_place_is_stored_replaced_and_goes_with_its_group() {
        let conn = db();
        let gid = "ab".repeat(16);
        let mut row = RendezvousRow {
            group_id: gid.clone(),
            record_key: "VLD0:rendezvous".into(),
            slot: 1,
            writer: "VLD0:writer-1".into(),
            slots: vec![
                ("a".repeat(64), 0),
                ("b".repeat(64), 1),
                ("c".repeat(64), 2),
            ],
        };
        put(&conn, OWNER, &row).expect("put");
        assert_eq!(get(&conn, OWNER, &gid).expect("get"), Some(row.clone()));
        assert_eq!(all(&conn, OWNER).expect("all"), vec![row.clone()]);
        row.slots.push(("d".repeat(64), 3));
        put(&conn, OWNER, &row).expect("replace");
        assert_eq!(get(&conn, OWNER, &gid).expect("get"), Some(row.clone()));
        assert!(!format!("{row:?}").contains("writer-1"));
        row.slot = 8;
        assert!(put(&conn, OWNER, &row).is_err());
        conn.execute("DELETE FROM group_dms WHERE group_id = ?1", [&gid])
            .expect("leave");
        assert_eq!(get(&conn, OWNER, &gid).expect("get"), None);
        assert!(all(&conn, OWNER).expect("all").is_empty());
    }
}
