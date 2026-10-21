//! Counts an intent review states (plan U1.4; ADR 0019): the history a
//! deletion destroys, the audience a message reaches, and the work a
//! logout or lock leaves unsent.

use rusqlite::{params, Connection};

fn count(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<u32> {
    let n: i64 = conn.query_row(sql, args, |row| row.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Messages stored for a channel: what deleting it destroys.
///
/// # Errors
/// The query failed.
pub fn channel_messages(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
) -> rusqlite::Result<u32> {
    count(
        conn,
        "SELECT COUNT(*) FROM messages \
         WHERE owner_key = ?1 AND conversation_type = 'channel' AND conversation_id = ?2",
        params![owner_key, channel_id],
    )
}

/// Members we know of in a community.
///
/// # Errors
/// The query failed.
pub fn community_members(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<u32> {
    count(
        conn,
        "SELECT COUNT(*) FROM community_members WHERE owner_key = ?1 AND community_id = ?2",
        params![owner_key, community_id],
    )
}

/// Sends the outbox has not settled (plan E2.5), and record writes a
/// logout left for the next login: what a logout or lock leaves
/// undelivered.
///
/// # Errors
/// The query failed.
pub fn unsent(conn: &Connection, owner_key: &str) -> rusqlite::Result<u32> {
    let writes = count(
        conn,
        "SELECT COUNT(*) FROM dht_outbox WHERE owner_key = ?1",
        params![owner_key],
    )?;
    let sends = super::outbox::unsettled_count(conn, owner_key)?;
    Ok(writes.saturating_add(u32::try_from(sends).unwrap_or(u32::MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn counts_are_per_owner_and_per_container() {
        let conn = fixture::conn();
        fixture::community(&conn, "c");
        for (i, ch) in ["ch1", "ch1", "ch2"].iter().enumerate() {
            conn.execute(
                "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp) \
                 VALUES (?1, ?2, 'channel', 's', 'b', ?3)",
                params![OWNER, ch, i64::try_from(i).unwrap()],
            )
            .unwrap();
        }
        assert_eq!(channel_messages(&conn, OWNER, "ch1").unwrap(), 2);
        assert_eq!(channel_messages(&conn, "other", "ch1").unwrap(), 0);

        for pk in ["a", "b"] {
            conn.execute(
                "INSERT INTO community_members (owner_key, community_id, pseudonym_key, joined_at) \
                 VALUES (?1, 'c', ?2, 0)",
                params![OWNER, pk],
            )
            .unwrap();
        }
        assert_eq!(community_members(&conn, OWNER, "c").unwrap(), 2);

        assert_eq!(unsent(&conn, OWNER).unwrap(), 0);
        crate::repo::outbox::queue_for_test(&conn, OWNER, "dm:r", "m1", b"{}");
        crate::repo::dht_outbox::save(
            &conn,
            OWNER,
            &[crate::repo::dht_outbox::OutboxWrite {
                record_key: "k".into(),
                subkey: 0,
                data: vec![1],
            }],
        )
        .unwrap();
        assert_eq!(unsent(&conn, OWNER).unwrap(), 2);
    }
}
