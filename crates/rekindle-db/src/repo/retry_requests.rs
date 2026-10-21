//! `retry_requests` and `retry_request_sends`: the ratchet messages from a
//! peer we could not open, and the `RetryRequest`s we sent for them (plan
//! E2.5, D24).
//!
//! A failure is held pending ([`note_failed`]) until the source's
//! CaughtUp, then [`take_due`] hands out what may go: at most
//! [`MAX_PER_MESSAGE`] requests per message and [`MAX_PER_PEER`] per peer
//! per [`PEER_WINDOW`] (Signal-Android's `retryReceiptMaxCount` and its
//! per-sender bound, r10 B.3). Each request handed out is counted in the
//! same transaction, so a crash never sends more than the caps allow.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rusqlite::{params, Connection};

use super::identity_text;

/// Requests per message, ever.
pub const MAX_PER_MESSAGE: u32 = 5;
/// Requests per peer within [`PEER_WINDOW`].
pub const MAX_PER_PEER: u32 = 10;
/// The per-peer window: 3 hours.
pub const PEER_WINDOW: Duration = Duration::from_secs(3 * 60 * 60);

/// A request to send now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub msg_id: [u8; 32],
    pub observed_ratchet_key: [u8; 32],
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).expect("durations fit i64 ms")
}

/// A ratchet message `msg_id` from `peer`, under `observed_ratchet_key`,
/// did not open: hold a request for it until the source's CaughtUp.
///
/// # Errors
/// The write failed.
pub fn note_failed(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    due: &Due,
    now_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO retry_requests (owner_key, peer, msg_id, observed_ratchet_key, pending, \
         failed_at) VALUES (?1, ?2, ?3, ?4, 1, ?5) \
         ON CONFLICT (owner_key, peer, msg_id) DO UPDATE SET pending = 1, \
         observed_ratchet_key = excluded.observed_ratchet_key, failed_at = excluded.failed_at",
        params![
            identity_text(owner),
            identity_text(peer),
            due.msg_id,
            due.observed_ratchet_key,
            now_ms
        ],
    )?;
    Ok(())
}

/// `peer` caught up: the pending requests that fit the caps, oldest
/// first, each counted as sent; a pending one at its message cap is
/// dropped from pending.
///
/// # Errors
/// The query failed.
pub fn take_due(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<Vec<Due>> {
    let (owner, peer) = (identity_text(owner), identity_text(peer));
    let tx = conn.unchecked_transaction()?;
    let recent: u32 = tx.query_row(
        "SELECT COUNT(*) FROM retry_request_sends WHERE owner_key = ?1 AND peer = ?2 \
         AND sent_at > ?3",
        params![owner, peer, now_ms.saturating_sub(ms(PEER_WINDOW))],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE retry_requests SET pending = 0 WHERE owner_key = ?1 AND peer = ?2 \
         AND sent_count >= ?3",
        params![owner, peer, MAX_PER_MESSAGE],
    )?;
    let due = {
        let mut stmt = tx.prepare(
            "SELECT msg_id, observed_ratchet_key FROM retry_requests \
             WHERE owner_key = ?1 AND peer = ?2 AND pending = 1 \
             ORDER BY failed_at, rowid LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(
                params![owner, peer, MAX_PER_PEER.saturating_sub(recent)],
                |row| {
                    Ok(Due {
                        msg_id: row.get(0)?,
                        observed_ratchet_key: row.get(1)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for d in &due {
        tx.execute(
            "UPDATE retry_requests SET pending = 0, sent_count = sent_count + 1 \
             WHERE owner_key = ?1 AND peer = ?2 AND msg_id = ?3",
            params![owner, peer, d.msg_id],
        )?;
        tx.execute(
            "INSERT INTO retry_request_sends (owner_key, peer, sent_at) VALUES (?1, ?2, ?3)",
            params![owner, peer, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(due)
}

/// Drop the sends older than [`PEER_WINDOW`] and the requests whose last
/// failure is older than [`DELIVERY_HORIZON`].
///
/// # Errors
/// The write failed.
pub fn prune(conn: &Connection, owner: &[u8; 32], now_ms: i64) -> rusqlite::Result<()> {
    let owner = identity_text(owner);
    conn.execute(
        "DELETE FROM retry_request_sends WHERE owner_key = ?1 AND sent_at <= ?2",
        params![owner, now_ms.saturating_sub(ms(PEER_WINDOW))],
    )?;
    conn.execute(
        "DELETE FROM retry_requests WHERE owner_key = ?1 AND failed_at < ?2",
        params![owner, now_ms.saturating_sub(ms(DELIVERY_HORIZON))],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture;

    const OWNER: [u8; 32] = [1; 32];
    const PEER: [u8; 32] = [2; 32];

    fn due(id: u8) -> Due {
        Due {
            msg_id: [id; 32],
            observed_ratchet_key: [9; 32],
        }
    }

    #[test]
    fn nothing_goes_before_a_failure_is_noted() {
        let conn = fixture::identity_conn(&OWNER);
        assert!(take_due(&conn, &OWNER, &PEER, 0).unwrap().is_empty());
        note_failed(&conn, &OWNER, &PEER, &due(1), 0).unwrap();
        assert_eq!(take_due(&conn, &OWNER, &PEER, 1).unwrap(), vec![due(1)]);
        // Sent: not again until it fails again.
        assert!(take_due(&conn, &OWNER, &PEER, 2).unwrap().is_empty());
    }

    #[test]
    fn five_requests_per_message() {
        let conn = fixture::identity_conn(&OWNER);
        let window = ms(PEER_WINDOW);
        let mut sent = 0;
        for i in 0..8 {
            // Outside each other's window, so only the message cap binds.
            let now = i * window * 2;
            note_failed(&conn, &OWNER, &PEER, &due(1), now).unwrap();
            sent += take_due(&conn, &OWNER, &PEER, now).unwrap().len();
        }
        assert_eq!(sent, MAX_PER_MESSAGE as usize);
    }

    #[test]
    fn ten_requests_per_peer_per_three_hours() {
        let conn = fixture::identity_conn(&OWNER);
        for id in 0..12 {
            note_failed(&conn, &OWNER, &PEER, &due(id), i64::from(id)).unwrap();
        }
        assert_eq!(take_due(&conn, &OWNER, &PEER, 100).unwrap().len(), 10);
        assert!(take_due(&conn, &OWNER, &PEER, 200).unwrap().is_empty());
        // Another peer has its own window.
        note_failed(&conn, &OWNER, &[3; 32], &due(1), 0).unwrap();
        assert_eq!(take_due(&conn, &OWNER, &[3; 32], 300).unwrap().len(), 1);
        // Past the window the rest go.
        let later = 100 + ms(PEER_WINDOW);
        assert_eq!(
            take_due(&conn, &OWNER, &PEER, later).unwrap(),
            vec![due(10), due(11)]
        );
        prune(&conn, &OWNER, later + ms(PEER_WINDOW)).unwrap();
        let sends: u32 = conn
            .query_row("SELECT COUNT(*) FROM retry_request_sends", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sends, 0);
    }
}
