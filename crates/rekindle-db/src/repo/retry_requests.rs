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
//!
//! A message that fails again after its [`MAX_PER_MESSAGE`]th request is
//! exhausted ([`Taken::exhausted`]): no request is left for it, and the
//! Host offers the user a session re-initiation (plan E2.5 last stage,
//! ruling Q1), which runs only when they accept.

use crate::row::{query_list, FromRow};
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

impl FromRow for Due {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Due {
            msg_id: row.get(0)?,
            observed_ratchet_key: row.get(1)?,
        })
    }
}

/// What [`take_due`] hands out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Taken {
    /// The requests to send now.
    pub due: Vec<Due>,
    /// The messages that failed again with no request left: their retry
    /// requests are exhausted.
    pub exhausted: usize,
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
/// dropped from pending and counted exhausted.
///
/// # Errors
/// The query failed.
pub fn take_due(
    conn: &Connection,
    owner: &[u8; 32],
    peer: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<Taken> {
    let (owner, peer) = (identity_text(owner), identity_text(peer));
    let tx = conn.unchecked_transaction()?;
    let recent: u32 = tx.query_row(
        "SELECT COUNT(*) FROM retry_request_sends WHERE owner_key = ?1 AND peer = ?2 \
         AND sent_at > ?3",
        params![owner, peer, now_ms.saturating_sub(ms(PEER_WINDOW))],
        |row| row.get(0),
    )?;
    let exhausted = tx.execute(
        "UPDATE retry_requests SET pending = 0 WHERE owner_key = ?1 AND peer = ?2 \
         AND pending = 1 AND sent_count >= ?3",
        params![owner, peer, MAX_PER_MESSAGE],
    )?;
    let due: Vec<Due> = query_list(
        &tx,
        "SELECT msg_id, observed_ratchet_key FROM retry_requests \
         WHERE owner_key = ?1 AND peer = ?2 AND pending = 1 \
         ORDER BY failed_at, rowid LIMIT ?3",
        params![owner, peer, MAX_PER_PEER.saturating_sub(recent)],
    )?;
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
    Ok(Taken { due, exhausted })
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
#[path = "retry_requests_tests.rs"]
mod tests;
